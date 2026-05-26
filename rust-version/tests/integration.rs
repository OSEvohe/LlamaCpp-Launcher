use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use reqwest::StatusCode;
use llama_launcher::server;
use llama_launcher::service::LlamaLauncherService;
use reqwest::Client;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

struct TestServer {
    base: String,
    client: Client,
    app_dir: PathBuf,
    _tmp: TempDir,
    handle: JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn boot_server() -> TestServer {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let app_dir = tmp.path().to_path_buf();
    let service = LlamaLauncherService::new(Some(app_dir.clone()));
    let state = Arc::new(RwLock::new(service));

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");
    let server_state = Arc::clone(&state);
    let handle = tokio::spawn(async move {
        server::serve(listener, server_state)
            .await
            .expect("run test server");
    });

    TestServer {
        base: format!("http://{}", addr),
        client: Client::new(),
        app_dir,
        _tmp: tmp,
        handle,
    }
}

#[test]
fn launch_status_stop_restart_dummy_lifecycle_with_pid_cleanup() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let app_dir = tmp.path().to_path_buf();
    let svc = LlamaLauncherService::new(Some(app_dir.clone()));

    let pid_file = app_dir.join(".launcher").join("llama-server.pid");

    let cmd = vec![
        "cmd".to_string(),
        "/C".to_string(),
        "ping".to_string(),
        "127.0.0.1".to_string(),
        "-n".to_string(),
        "20".to_string(),
    ];

    let pid1 = svc.launch(cmd.clone(), "");
    assert!(pid1 > 0, "launch should return a positive pid");
    assert!(pid_file.exists(), "pid file should exist after launch");
    let (running1, status_pid1) = svc.is_server_running();
    assert!(running1, "status should report running after launch");
    assert_eq!(status_pid1, pid1, "status pid should match launched pid");

    let stopped_pid = svc.stop();
    assert_eq!(stopped_pid, pid1, "stop should return the launched pid");
    assert!(
        !pid_file.exists(),
        "pid file should be cleaned up after stop"
    );
    let (running2, _status_pid2) = svc.is_server_running();
    assert!(!running2, "status should report stopped after stop");

    let pid2 = svc.restart(cmd, "");
    assert!(pid2 > 0, "restart should relaunch process");
    assert_ne!(pid1, pid2, "restart should produce a new pid");
    assert!(pid_file.exists(), "pid file should exist after restart launch");

    let stopped_pid2 = svc.stop();
    assert_eq!(stopped_pid2, pid2, "stop should return restarted pid");
    assert!(
        !pid_file.exists(),
        "pid file should be cleaned up after final stop"
    );
}

#[test]
fn tail_log_append_truncate_and_equal_size_rewrite() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let app_dir = tmp.path().to_path_buf();
    let svc = LlamaLauncherService::new(Some(app_dir.clone()));
    let log = app_dir.join(".launcher").join("llama-server.log");
    fs::create_dir_all(log.parent().expect("log parent")).expect("create state dir");

    fs::write(&log, "line-a\n").expect("seed log");
    let (chunk1, size1, reset1, marker1) = svc.tail_log(0, "");
    assert_eq!(chunk1, "line-a\n");
    assert!(!reset1);

    fs::write(&log, "line-a\nline-b\n").expect("append log");
    let (chunk2, size2, reset2, marker2) = svc.tail_log(size1, &marker1);
    assert_eq!(chunk2, "line-b\n");
    assert!(size2 > size1);
    assert!(!reset2);

    fs::write(&log, "short\n").expect("truncate log");
    let (chunk3, size3, reset3, marker3) = svc.tail_log(size2, &marker2);
    assert_eq!(chunk3, "short\n");
    assert_eq!(size3, "short\n".len());
    assert!(reset3);

    fs::write(&log, "other\n").expect("rewrite equal-size log");
    let (chunk4, size4, reset4, _marker4) = svc.tail_log(size3, &marker3);
    assert_eq!(chunk4, "other\n");
    assert_eq!(size4, size3);
    assert!(reset4);
}

#[tokio::test]
async fn concurrent_api_requests_do_not_fail_or_panic() {
    let ts = boot_server().await;

    let mut handles = Vec::new();
    for i in 0..24_i64 {
        let client = ts.client.clone();
        let base = ts.base.clone();
        handles.push(tokio::spawn(async move {
            let put = client
                .put(format!("{}/api/settings", base))
                .json(&json!({
                    "api_host": "127.0.0.1",
                    "api_port": 10000 + i,
                    "llama_server_path": format!("C:/dummy/llama-server-{}.exe", i)
                }))
                .send()
                .await
                .expect("put settings request");
            assert_eq!(put.status(), StatusCode::OK);

            let status = client
                .get(format!("{}/api/status", base))
                .send()
                .await
                .expect("get status request");
            assert_eq!(status.status(), StatusCode::OK);

            let profiles = client
                .get(format!("{}/api/profiles", base))
                .send()
                .await
                .expect("get profiles request");
            assert_eq!(profiles.status(), StatusCode::OK);
        }));
    }

    for h in handles {
        h.await.expect("concurrent task join");
    }

    let after = ts
        .client
        .get(format!("{}/api/settings", ts.base))
        .send()
        .await
        .expect("get settings after concurrency");
    assert_eq!(after.status(), StatusCode::OK);
    let body: Value = after.json().await.expect("settings json");
    let api_port = body
        .get("api_port")
        .and_then(Value::as_i64)
        .expect("api_port should be int");
    assert!((10000..=10023).contains(&api_port));
}

#[tokio::test]
async fn headless_startup_behavior_ephemeral_bind_and_dashboard_api_alive() {
    let ts = boot_server().await;

    let root = ts
        .client
        .get(format!("{}/", ts.base))
        .send()
        .await
        .expect("get dashboard");
    assert_eq!(root.status(), StatusCode::OK);

    let status = ts
        .client
        .get(format!("{}/api/status", ts.base))
        .send()
        .await
        .expect("get status");
    assert_eq!(status.status(), StatusCode::OK);
    let status_json: Value = status.json().await.expect("status json");
    assert!(status_json.get("running").is_some());
    assert!(status_json.get("pid").is_some());
}

#[tokio::test]
async fn startup_regression_concurrent_boot_uses_isolated_tempdirs_and_ports() {
    let mut servers = Vec::new();
    for _ in 0..3 {
        servers.push(boot_server().await);
    }

    let mut bases = HashSet::new();
    let mut app_dirs = HashSet::new();
    for ts in &servers {
        bases.insert(ts.base.clone());
        app_dirs.insert(ts.app_dir.clone());
    }
    assert_eq!(bases.len(), 3, "each server should have unique port");
    assert_eq!(app_dirs.len(), 3, "each server should have isolated app dir");

    for ts in &servers {
        let resp = ts
            .client
            .get(format!("{}/api/profiles", ts.base))
            .send()
            .await
            .expect("get profiles on each boot");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    tokio::time::sleep(Duration::from_millis(10)).await;
}
