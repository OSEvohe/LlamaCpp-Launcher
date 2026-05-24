use std::path::Path;
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

fn start_service_args() -> Vec<String> {
    vec!["start".to_string(), SERVICE_NAME.to_string()]
}

fn stop_service_args() -> Vec<String> {
    vec!["stop".to_string(), SERVICE_NAME.to_string()]
}

fn uninstall_service_args() -> Vec<String> {
    vec!["delete".to_string(), SERVICE_NAME.to_string()]
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
    if service_exists()? {
        return Err(format!(
            "Windows service '{}' already exists; uninstall it before reinstalling",
            SERVICE_NAME
        ));
    }

    let exe =
        std::env::current_exe().map_err(|err| format!("failed to resolve current executable: {}", err))?;
    let args = install_service_args(&exe);
    let output = run_sc(&args, "create")?;
    if output.status.success() {
        return Ok(());
    }

    Err(render_sc_error(&output, "install"))
}

pub fn uninstall_service() -> Result<(), String> {
    if !service_exists()? {
        return Ok(());
    }

    let stop_args = stop_service_args();
    let stop_output = run_sc(&stop_args, "stop")?;
    if !stop_output.status.success() {
        let stdout = String::from_utf8_lossy(&stop_output.stdout);
        if !stdout.contains("FAILED 1062") && !stdout.contains("FAILED 1052") {
            return Err(render_sc_error(&stop_output, "stop"));
        }
    }

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
