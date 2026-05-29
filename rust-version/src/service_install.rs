use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const TASK_NAME: &str = "LLama Launcher";
pub const SERVICE_NAME: &str = "LlamaLauncher";
pub const SERVICE_DISPLAY_NAME: &str = "LLama Launcher";

fn install_task_args(exe_path: &Path, force: bool) -> Vec<String> {
    let mut args = vec![
        "/create".to_string(),
        "/tn".to_string(),
        TASK_NAME.to_string(),
        "/sc".to_string(),
        "onlogon".to_string(),
        "/tr".to_string(),
        format!("\"{}\"", exe_path.display()),
    ];

    if force {
        args.push("/f".to_string());
    }

    args
}

fn uninstall_task_args() -> Vec<String> {
    vec![
        "/delete".to_string(),
        "/tn".to_string(),
        TASK_NAME.to_string(),
        "/f".to_string(),
    ]
}

fn service_query_args() -> Vec<String> {
    vec!["query".to_string(), SERVICE_NAME.to_string()]
}

fn install_service_args(exe_path: &Path) -> Vec<String> {
    vec![
        "create".to_string(),
        SERVICE_NAME.to_string(),
        "binPath=".to_string(),
        format!("\"{}\"", exe_path.display()),
        "start=".to_string(),
        "auto".to_string(),
        "DisplayName=".to_string(),
        SERVICE_DISPLAY_NAME.to_string(),
    ]
}

fn config_service_bin_path_args(exe_path: &Path) -> Vec<String> {
    vec![
        "config".to_string(),
        SERVICE_NAME.to_string(),
        "binPath=".to_string(),
        format!("\"{}\"", exe_path.display()),
    ]
}

fn config_service_delayed_auto_args() -> Vec<String> {
    vec![
        "config".to_string(),
        SERVICE_NAME.to_string(),
        "start=".to_string(),
        "delayed-auto".to_string(),
    ]
}

fn start_service_args() -> Vec<String> {
    vec!["start".to_string(), SERVICE_NAME.to_string()]
}

fn stop_service_args() -> Vec<String> {
    vec!["stop".to_string(), SERVICE_NAME.to_string()]
}

fn uninstall_service_args() -> Vec<String> {
    vec!["delete".to_string(), SERVICE_NAME.to_string()]
}

fn default_install_exe_path(current_exe: &Path) -> PathBuf {
    let base = std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    let file_name = current_exe
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "llama-launcher.exe".into());

    base.join("LLama Launcher").join(file_name)
}

fn stop_service_if_running() -> Result<(), String> {
    let stop_args = stop_service_args();
    let stop_output = run_sc(&stop_args, "stop")?;
    if stop_output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&stop_output.stdout);
    if stdout.contains("1062") || stdout.contains("1052") {
        return Ok(());
    }

    Err(render_sc_error(&stop_output, "stop"))
}

fn start_service_after_install() -> Result<(), String> {
    let start_args = start_service_args();
    let start_output = run_sc(&start_args, "start")?;
    if start_output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&start_output.stdout);
    if stdout.contains("1056") {
        return Ok(());
    }

    Err(render_sc_error(&start_output, "start"))
}

fn run_sc(args: &[String], action: &str) -> Result<std::process::Output, String> {
    Command::new("sc.exe")
        .args(args)
        .output()
        .map_err(|err| format!("failed to run sc.exe {}: {}", action, err))
}

fn render_sc_error(output: &std::process::Output, action: &str) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}\n{}", stdout.trim(), stderr.trim()).trim().to_string();
    format!(
        "failed to {} Windows service '{}': {}",
        action,
        SERVICE_NAME,
        combined
    )
}

pub fn task_exists() -> Result<bool, String> {
    let output = Command::new("schtasks")
        .args(["/query", "/tn", TASK_NAME])
        .output()
        .map_err(|err| format!("failed to run schtasks /query: {}", err))?;

    Ok(output.status.success())
}

pub fn install_task(force: bool) -> Result<(), String> {
    if !force && task_exists()? {
        return Err(format!(
            "scheduled task '{}' already exists; rerun with --force to overwrite",
            TASK_NAME
        ));
    }

    let exe = std::env::current_exe().map_err(|err| format!("failed to resolve current executable: {}", err))?;
    let output = Command::new("schtasks")
        .args(install_task_args(&exe, force))
        .output()
        .map_err(|err| format!("failed to run schtasks /create: {}", err))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "failed to install scheduled task '{}': {}",
        TASK_NAME,
        stderr.trim()
    ))
}

pub fn uninstall_task() -> Result<(), String> {
    if !task_exists()? {
        return Ok(());
    }

    let output = Command::new("schtasks")
        .args(uninstall_task_args())
        .output()
        .map_err(|err| format!("failed to run schtasks /delete: {}", err))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "failed to uninstall scheduled task '{}': {}",
        TASK_NAME,
        stderr.trim()
    ))
}

pub fn service_exists() -> Result<bool, String> {
    let args = service_query_args();
    let output = run_sc(&args, "query")?;
    Ok(output.status.success())
}

pub fn install_service() -> Result<(), String> {
    let current_exe =
        std::env::current_exe().map_err(|err| format!("failed to resolve current executable: {}", err))?;
    let install_exe = default_install_exe_path(&current_exe);
    let install_dir = install_exe
        .parent()
        .ok_or_else(|| format!("failed to resolve install directory for {}", install_exe.display()))?;
    let exists = service_exists()?;

    if exists {
        stop_service_if_running()?;
    }

    fs::create_dir_all(install_dir).map_err(|err| {
        format!(
            "failed to create install directory '{}': {}",
            install_dir.display(),
            err
        )
    })?;
    fs::copy(&current_exe, &install_exe).map_err(|err| {
        format!(
            "failed to copy executable from '{}' to '{}': {}",
            current_exe.display(),
            install_exe.display(),
            err
        )
    })?;

    if exists {
        let config_args = config_service_bin_path_args(&install_exe);
        let config_output = run_sc(&config_args, "config binPath")?;
        if !config_output.status.success() {
            return Err(render_sc_error(&config_output, "update"));
        }
    } else {
        let create_args = install_service_args(&install_exe);
        let create_output = run_sc(&create_args, "create")?;
        if !create_output.status.success() {
            return Err(render_sc_error(&create_output, "install"));
        }
    }

    let delayed_auto_args = config_service_delayed_auto_args();
    let delayed_auto_output = run_sc(&delayed_auto_args, "config delayed-auto start")?;
    if !delayed_auto_output.status.success() {
        return Err(render_sc_error(
            &delayed_auto_output,
            "configure delayed-auto start for",
        ));
    }

    start_service_after_install()
}

pub fn uninstall_service() -> Result<(), String> {
    if !service_exists()? {
        return Ok(());
    }

    stop_service_if_running()?;

    let delete_args = uninstall_service_args();
    let delete_output = run_sc(&delete_args, "delete")?;
    if delete_output.status.success() {
        return Ok(());
    }

    Err(render_sc_error(&delete_output, "uninstall"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_task_args_without_force() {
        let exe = Path::new(r"C:\apps\llama-launcher.exe");
        let args = install_task_args(exe, false);

        assert_eq!(args[0], "/create");
        assert_eq!(args[1], "/tn");
        assert_eq!(args[2], TASK_NAME);
        assert_eq!(args[3], "/sc");
        assert_eq!(args[4], "onlogon");
        assert_eq!(args[5], "/tr");
        assert_eq!(args[6], "\"C:\\apps\\llama-launcher.exe\"");
        assert!(!args.iter().any(|a| a == "/f"));
    }

    #[test]
    fn test_install_task_args_with_force() {
        let exe = Path::new(r"C:\apps\llama-launcher.exe");
        let args = install_task_args(exe, true);
        assert_eq!(args.last().map(String::as_str), Some("/f"));
    }

    #[test]
    fn test_uninstall_task_args() {
        let args = uninstall_task_args();
        assert_eq!(
            args,
            vec![
                "/delete".to_string(),
                "/tn".to_string(),
                TASK_NAME.to_string(),
                "/f".to_string(),
            ]
        );
    }

    #[test]
    fn test_service_query_args() {
        let args = service_query_args();
        assert_eq!(args, vec!["query".to_string(), SERVICE_NAME.to_string()]);
    }

    #[test]
    fn test_install_service_args() {
        let exe = Path::new(r"C:\apps\llama-launcher.exe");
        let args = install_service_args(exe);

        assert_eq!(args[0], "create");
        assert_eq!(args[1], SERVICE_NAME);
        assert_eq!(args[2], "binPath=");
        assert_eq!(args[3], "\"C:\\apps\\llama-launcher.exe\"");
        assert_eq!(args[4], "start=");
        assert_eq!(args[5], "auto");
        assert_eq!(args[6], "DisplayName=");
        assert_eq!(args[7], SERVICE_DISPLAY_NAME);
    }

    #[test]
    fn test_start_service_args() {
        let args = start_service_args();
        assert_eq!(args, vec!["start".to_string(), SERVICE_NAME.to_string()]);
    }

    #[test]
    fn test_config_service_delayed_auto_args() {
        let args = config_service_delayed_auto_args();
        assert_eq!(
            args,
            vec![
                "config".to_string(),
                SERVICE_NAME.to_string(),
                "start=".to_string(),
                "delayed-auto".to_string(),
            ]
        );
    }

    #[test]
    fn test_config_service_bin_path_args() {
        let exe = Path::new(r"C:\Program Files\LLama Launcher\llama-launcher.exe");
        let args = config_service_bin_path_args(exe);
        assert_eq!(args[0], "config");
        assert_eq!(args[1], SERVICE_NAME);
        assert_eq!(args[2], "binPath=");
        assert_eq!(args[3], "\"C:\\Program Files\\LLama Launcher\\llama-launcher.exe\"");
    }

    #[test]
    fn test_stop_service_args() {
        let args = stop_service_args();
        assert_eq!(args, vec!["stop".to_string(), SERVICE_NAME.to_string()]);
    }

    #[test]
    fn test_uninstall_service_args() {
        let args = uninstall_service_args();
        assert_eq!(args, vec!["delete".to_string(), SERVICE_NAME.to_string()]);
    }
}
