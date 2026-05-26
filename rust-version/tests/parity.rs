use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::StatusCode;

const READINESS_TIMEOUT: Duration = Duration::from_secs(20);

struct ServerProcess {
    child: Child,
    base_url: String,
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rust-version should have repo parent")
        .to_path_buf()
}

fn python_executable() -> Option<String> {
    std::env::var("PYTHON").ok().or_else(|| Some("python".to_string()))
}

fn should_run_full_harness() -> bool {
    std::env::var("LLAMA_PARITY_RUN")
        .ok()
        .map(|v| {
            let lower = v.to_ascii_lowercase();
            lower == "1" || lower == "true" || lower == "yes"
        })
        .unwrap_or(false)
}

fn wait_for_server_url(mut child: Child) -> Result<ServerProcess, String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "child process did not expose stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "child process did not expose stderr".to_string())?;

    let (tx, rx) = std::sync::mpsc::channel::<String>();

    let tx_out = tx.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let _ = tx_out.send(line);
        }
    });

    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });

    let start = Instant::now();
    while start.elapsed() < READINESS_TIMEOUT {
        if let Ok(Some(status)) = child.try_wait().map(Some) {
            return Err(format!("child exited before ready (status: {status:?})"));
        }

        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                if let Some(idx) = line.find("http://") {
                    let base = line[idx..].trim().to_string();
                    return Ok(ServerProcess {
                        child,
                        base_url: base,
                    });
                }
                if let Some(addr) = line.strip_prefix("Starting LLama Launcher API server on ") {
                    let base = format!("http://{}", addr.trim());
                    return Ok(ServerProcess {
                        child,
                        base_url: base,
                    });
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    Err("timed out waiting for server bind log line".to_string())
}

fn spawn_python_server() -> Result<ServerProcess, String> {
    let python = python_executable().ok_or_else(|| "python executable not configured".to_string())?;
    let root = repo_root();
    let child = Command::new(python)
        .arg("main.py")
        .arg("--api-host")
        .arg("127.0.0.1")
        .arg("--api-port")
        .arg("0")
        .current_dir(&root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn python server: {err}"))?;
    wait_for_server_url(child)
}

fn spawn_rust_server() -> Result<ServerProcess, String> {
    let root = repo_root();
    let exe = std::env::var("CARGO_BIN_EXE_llama-launcher")
        .map_err(|_| "CARGO_BIN_EXE_llama-launcher not set by cargo test".to_string())?;

    let child = Command::new(exe)
        .arg("--api-host")
        .arg("127.0.0.1")
        .arg("--api-port")
        .arg("0")
        .current_dir(&root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn rust server: {err}"))?;
    wait_for_server_url(child)
}

async fn json_bytes(client: &reqwest::Client, base: &str, path: &str) -> Result<Vec<u8>, String> {
    let resp = client
        .get(format!("{base}{path}"))
        .send()
        .await
        .map_err(|err| format!("request failed for {path}: {err}"))?;

    if resp.status() != StatusCode::OK {
        return Err(format!("unexpected status for {path}: {}", resp.status()));
    }

    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|err| format!("failed reading response body for {path}: {err}"))
}

fn run_python_snippet(root: &Path, script: &str) -> Result<String, String> {
    let python = python_executable().ok_or_else(|| "python executable not configured".to_string())?;
    let output = Command::new(python)
        .arg("-c")
        .arg(script)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| format!("failed to run python snippet: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("python snippet failed: {}", stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[tokio::test]
async fn parity_api_json_byte_identical_scaffold() {
    if !should_run_full_harness() {
        eprintln!("SKIP: set LLAMA_PARITY_RUN=1 to run cross-runtime parity harness");
        return;
    }

    let py = match spawn_python_server() {
        Ok(s) => s,
        Err(err) => {
            eprintln!("SKIP: {err}");
            return;
        }
    };
    let rust = match spawn_rust_server() {
        Ok(s) => s,
        Err(err) => {
            eprintln!("SKIP: {err}");
            return;
        }
    };

    let client = reqwest::Client::new();
    let paths = ["/api/status", "/api/settings", "/api/profiles", "/api/models", "/api/logs"];

    for path in paths {
        let left = json_bytes(&client, &py.base_url, path).await.expect("python response");
        let right = json_bytes(&client, &rust.base_url, path).await.expect("rust response");
        assert_eq!(left, right, "JSON bytes differ for endpoint {path}");
    }
}

#[test]
fn parity_state_file_compatibility_scaffold() {
    if !should_run_full_harness() {
        eprintln!("SKIP: set LLAMA_PARITY_RUN=1 to run cross-runtime parity harness");
        return;
    }

    let root = repo_root();
    let temp = tempfile::tempdir().expect("create temp state dir");
    let app_dir = temp.path().to_string_lossy().replace('\\', "\\\\");

    let py_write = format!(
        "import json; from llama_launcher.api import LlamaLauncherService; s=LlamaLauncherService(app_dir=r\"{app_dir}\"); s.save_global({{'llama_server_path':'C:/py/llama-server.exe','model_dirs':['C:/models'],'api_host':'127.0.0.1','api_port':17890}}); p=s.load_profiles(); p[0].name='py-wrote-profile'; s.save_profiles(p); print('ok')"
    );
    run_python_snippet(&root, &py_write).expect("python should write compatible state");

    let rust_reader = llama_launcher::service::LlamaLauncherService::new(Some(temp.path().to_path_buf()));
    let global = rust_reader.load_global();
    assert_eq!(global.llama_server_path, "C:/py/llama-server.exe");
    assert_eq!(global.api_port, 17890);
    let profiles = rust_reader.load_profiles();
    assert_eq!(profiles[0].name, "py-wrote-profile");

    rust_reader.save_global(llama_launcher::models::GlobalSettings {
        llama_server_path: "C:/rust/llama-server.exe".to_string(),
        model_dirs: vec!["C:/models-rust".to_string()],
        api_host: "127.0.0.1".to_string(),
        api_port: 27890,
    });
    let mut rust_profiles = rust_reader.load_profiles();
    rust_profiles[0].name = "rust-wrote-profile".to_string();
    rust_reader.save_profiles(rust_profiles);

    let py_read = format!(
        "from llama_launcher.api import LlamaLauncherService; s=LlamaLauncherService(app_dir=r\"{app_dir}\"); g=s.load_global(); p=s.load_profiles(); print('{{}}|{{}}|{{}}'.format(g.llama_server_path,g.api_port,p[0].name))"
    );
    let echoed = run_python_snippet(&root, &py_read).expect("python should read rust-written state");
    assert_eq!(echoed, "C:/rust/llama-server.exe|27890|rust-wrote-profile");

    let profiles_file = temp.path().join(".launcher").join("profiles.json");
    let global_file = temp.path().join(".launcher").join("global.json");
    let _ = std::fs::read_to_string(profiles_file).expect("profiles state should exist");
    let _ = std::fs::read_to_string(global_file).expect("global state should exist");

}
