//! Process lifecycle helpers for llama-server.
//!
//! Mirrors ``llama_launcher/process.py``: Windows-only process management
//! using ``tasklist`` / ``taskkill`` and ``CreateProcessW`` with detached
//! creation flags.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use crate::monitoring::parse_tasklist_csv;

use winapi::shared::ntdef::HANDLE;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::fileapi::{CreateFileW, OPEN_EXISTING};
use winapi::um::handleapi::{CloseHandle, DuplicateHandle};
use winapi::um::processthreadsapi::{
    CreateProcessW, GetCurrentProcessId, GetExitCodeProcess, OpenProcess, PROCESS_INFORMATION,
    STARTUPINFOW, TerminateProcess,
};
use winapi::um::winbase::{
    CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, STARTF_USESTDHANDLES,
};
use winapi::um::winnt::{
    DUPLICATE_SAME_ACCESS, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
    PROCESS_DUP_HANDLE, PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE,
};

/// Read the PID stored in *pid_file*, returning 0 on any failure.
pub fn read_pid(pid_file: &Path) -> i32 {
    if !pid_file.exists() {
        return 0;
    }
    match std::fs::read_to_string(pid_file) {
        Ok(text) => text.trim().parse::<i32>().unwrap_or(0),
        Err(_) => 0,
    }
}

/// Write *pid* to *pid_file*.
pub fn write_pid(pid_file: &Path, pid: i32) {
    std::fs::write(pid_file, pid.to_string()).ok();
}

/// Check whether a Windows process with *pid* is alive (tasklist).
pub fn is_process_running(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    match std::process::Command::new("tasklist")
        .args(&["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
        .output()
    {
        Ok(output) => {
            let out = String::from_utf8_lossy(&output.stdout).to_string();
            let out = out.trim();
            if out.is_empty() {
                return false;
            }
            if out.contains("No tasks are running") {
                return false;
            }
            out.contains(&format!("\"{}\"", pid))
        }
        Err(_) => false,
    }
}

/// Launch *cmd* as a detached subprocess, writing stdout to *stdout_path*.
///
/// Uses ``CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP`` (0x08000000 | 0x00000200)
/// to detach the child from the parent job, matching Python's ``creationflags``.
///
/// Returns the child PID on success, or 0 on failure.
pub fn start_server(cmd: &[String], stdout_path: &Path, cwd: &Path) -> i32 {
    // Build the command-line string (Windows CreateProcessW expects a single string).
    let cmd_line = build_command_line(cmd);

    // Open the stdout file for writing (truncated).
    let stdout_handle: HANDLE = match open_stdout_handle(stdout_path) {
        Ok(h) => h,
        Err(_) => return 0,
    };

    // Duplicate the handle so it's inheritable.
    let inherit_handle: HANDLE = match duplicate_handle_inheritable(stdout_handle) {
        Ok(h) => h,
        Err(_) => {
            unsafe { CloseHandle(stdout_handle) };
            return 0;
        }
    };

    // Open NUL device for stdin, matching Python's stdin=subprocess.DEVNULL.
    let nul_wide: Vec<u16> = "NUL\0".encode_utf16().collect();
    let nul_handle: HANDLE = unsafe {
        CreateFileW(
            nul_wide.as_ptr(),
            0, // no access needed for DEVNULL
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    let invalid_handle = unsafe { std::mem::transmute(-1isize) };
    let stdin_handle = if nul_handle.is_null() || nul_handle == invalid_handle {
        // Fallback: if NUL cannot be opened, use null handle (child inherits parent stdin).
        std::ptr::null_mut()
    } else {
        match duplicate_handle_inheritable(nul_handle) {
            Ok(h) => {
                unsafe { CloseHandle(nul_handle) };
                h
            }
            Err(_) => {
                unsafe { CloseHandle(nul_handle) };
                std::ptr::null_mut()
            }
        }
    };

    // Build the working directory as mutable UTF-16.
    let cwd_wide: Vec<u16> = cwd
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // Build the command line as mutable UTF-16.
    let mut cmd_wide: Vec<u16> = OsStr::new(&cmd_line)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // Set up STARTUPINFO with redirected stdout, stderr, and stdin=DEVNULL.
    let mut startup_info: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup_info.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup_info.hStdInput = stdin_handle;
    startup_info.hStdOutput = inherit_handle;
    startup_info.hStdError = inherit_handle;
    startup_info.dwFlags = STARTF_USESTDHANDLES;

    let mut proc_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    let creation_flags: u32 = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;

    let success = unsafe {
        CreateProcessW(
            std::ptr::null_mut(),
            cmd_wide.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            1, // bInheritHandles = TRUE
            creation_flags,
            std::ptr::null_mut(),
            cwd_wide.as_ptr() as *mut u16,
            &mut startup_info,
            &mut proc_info,
        )
    };

    // Close the duplicated inheritable handles (child has its own copies).
    unsafe {
        CloseHandle(inherit_handle);
        CloseHandle(stdout_handle);
        if !stdin_handle.is_null() {
            CloseHandle(stdin_handle);
        }
    }

    if success != 0 {
        let pid = proc_info.dwProcessId as i32;
        // Close the process and thread handles we don't need.
        unsafe {
            CloseHandle(proc_info.hProcess);
            CloseHandle(proc_info.hThread);
        }
        pid
    } else {
        0
    }
}

/// Return the PID of a running llama-server process, or 0 if not found.
///
/// Uses ``tasklist`` filtered by image name as a fallback when the PID file
/// is missing or stale.
pub fn find_llama_server_pid() -> i32 {
    match std::process::Command::new("tasklist")
        .args(&["/FI", "IMAGENAME eq llama-server*", "/FO", "CSV", "/NH"])
        .output()
    {
        Ok(output) => {
            let out = String::from_utf8_lossy(&output.stdout).to_string();
            let out = out.trim();
            if out.is_empty() || out.contains("No tasks are running") {
                return 0;
            }
            // tasklist CSV columns: "ImageName","PID","SessionName","Session#","MemUsage"
            // PID is the second column (index 1). Parse robustly with quote-aware splitting.
            if let Some(first_line) = out.lines().next() {
                let fields = parse_tasklist_csv(first_line.trim());
                if fields.len() >= 2 {
                    let pid_str = fields[1].trim_matches('"');
                    return pid_str.parse::<i32>().unwrap_or(0);
                }
            }
            0
        }
        Err(_) => 0,
    }
}

/// Force-kill the process tree rooted at *pid* (taskkill /F /T).
pub fn stop_server(pid: i32) {
    std::process::Command::new("taskkill")
        .args(&["/PID", &pid.to_string(), "/F", "/T"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok();
}

/// Terminate a single process by PID using the WinAPI ``TerminateProcess``.
/// Returns true if the process was successfully terminated.
pub fn terminate_process(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid as u32) };
    if handle.is_null() {
        return false;
    }
    let success = unsafe { TerminateProcess(handle, 1) != 0 };
    unsafe { CloseHandle(handle) };
    success
}

/// Wait for a process to exit, returning its exit code.
/// Returns ``None`` if the process handle cannot be opened.
pub fn get_process_exit_code(pid: i32) -> Option<u32> {
    if pid <= 0 {
        return None;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid as u32) };
    if handle.is_null() {
        return None;
    }
    let mut exit_code: u32 = 0;
    let success = unsafe { GetExitCodeProcess(handle, &mut exit_code) != 0 };
    unsafe { CloseHandle(handle) };
    if success {
        Some(exit_code)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build a Windows command-line string from a list of arguments.
///
/// Arguments containing spaces are quoted. Quotes and backslashes are escaped
/// per the Windows CommandLineToArgvW rules.
fn build_command_line(cmd: &[String]) -> String {
    if cmd.is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::new();
    for (i, arg) in cmd.iter().enumerate() {
        if i == 0 {
            // First argument is the executable; quote it if it contains spaces.
            if arg.contains(' ') || arg.contains('\t') {
                parts.push(format!("\"{}\"", escape_for_cmdline(arg)));
            } else {
                parts.push(arg.clone());
            }
        } else {
            if arg.contains(' ') || arg.contains('\t') || arg.contains('"') {
                parts.push(format!("\"{}\"", escape_for_cmdline(arg)));
            } else {
                parts.push(arg.clone());
            }
        }
    }
    parts.join(" ")
}

/// Escape a string for inclusion in a quoted Windows command-line argument.
///
/// Backslashes before a quote are doubled, and the trailing quote is escaped.
fn escape_for_cmdline(s: &str) -> String {
    let mut result = String::new();
    let mut backslash_count = 0;
    for c in s.chars() {
        if c == '\\' {
            backslash_count += 1;
        } else if c == '"' {
            // Escape backslashes before the quote: double them.
            for _ in 0..backslash_count {
                result.push('\\');
                result.push('\\');
            }
            backslash_count = 0;
            result.push('"');
        } else {
            // Flush backslashes as-is (not followed by a quote).
            for _ in 0..backslash_count {
                result.push('\\');
            }
            backslash_count = 0;
            result.push(c);
        }
    }
    // Flush any remaining backslashes.
    for _ in 0..backslash_count {
        result.push('\\');
    }
    result
}

/// Open a file handle for writing (truncated), suitable for use as a child
/// process stdout. Returns a raw WinAPI HANDLE.
fn open_stdout_handle(path: &Path) -> Result<HANDLE, std::io::Error> {
    // Truncate the file first (matching Python's open("w")).
    std::fs::write(path, "")?;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .append(true)
        .open(path)?;
    // RawHandle is *mut std::ffi::c_void, HANDLE is *mut winapi::ctypes::c_void.
    // They have the same representation so we can transmute.
    Ok(unsafe { std::mem::transmute(std::os::windows::io::IntoRawHandle::into_raw_handle(file)) })
}

/// Duplicate a handle so it's inheritable.
fn duplicate_handle_inheritable(handle: HANDLE) -> Result<HANDLE, std::io::Error> {
    let my_pid = unsafe { GetCurrentProcessId() };
    let current_proc = unsafe { OpenProcess(PROCESS_DUP_HANDLE, 0, my_pid) };
    if current_proc.is_null() {
        return Err(std::io::Error::from_raw_os_error(unsafe { GetLastError() as i32 }));
    }

    let mut new_handle: HANDLE = std::ptr::null_mut();

    let success = unsafe {
        DuplicateHandle(
            current_proc,
            handle,
            current_proc,
            &mut new_handle,
            0,
            1, // bInheritHandle = TRUE
            DUPLICATE_SAME_ACCESS,
        )
    };

    unsafe { CloseHandle(current_proc) };

    if success == 0 {
        Err(std::io::Error::from_raw_os_error(unsafe { GetLastError() as i32 }))
    } else {
        Ok(new_handle)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    // ---- Acceptance: read_pid / PID file round-trip ----

    #[test]
    fn test_read_pid_missing_file() {
        assert_eq!(read_pid(Path::new("/nonexistent/pid.txt")), 0);
    }

    #[test]
    fn test_read_pid_valid() {
        let tmp = TempDir::new().expect("create temp dir");
        let pid_file = tmp.path().join("server.pid");
        std::fs::write(&pid_file, "12345").unwrap();
        assert_eq!(read_pid(&pid_file), 12345);
    }

    #[test]
    fn test_read_pid_malformed() {
        let tmp = TempDir::new().expect("create temp dir");
        let pid_file = tmp.path().join("server.pid");
        std::fs::write(&pid_file, "not-a-number").unwrap();
        assert_eq!(read_pid(&pid_file), 0);
    }

    #[test]
    fn test_read_pid_whitespace() {
        let tmp = TempDir::new().expect("create temp dir");
        let pid_file = tmp.path().join("server.pid");
        std::fs::write(&pid_file, "  9876  \n").unwrap();
        assert_eq!(read_pid(&pid_file), 9876);
    }

    #[test]
    fn test_write_pid_roundtrip() {
        let tmp = TempDir::new().expect("create temp dir");
        let pid_file = tmp.path().join("server.pid");
        write_pid(&pid_file, 42);
        assert_eq!(read_pid(&pid_file), 42);
    }

    // ---- Acceptance: is_process_running ----

    #[test]
    fn test_is_process_running_invalid_pid() {
        assert!(!is_process_running(0));
        assert!(!is_process_running(-1));
    }

    #[test]
    fn test_is_process_running_nonexistent() {
        // PID 99999999 is extremely unlikely to exist.
        assert!(!is_process_running(99999999));
    }

    #[test]
    fn test_is_process_running_self() {
        // Our own process should be running.
        let my_pid = std::process::id() as i32;
        assert!(is_process_running(my_pid));
    }

  // ---- Acceptance: start_server spawns a detached process and returns its PID ----

    #[test]
    fn test_start_server_returns_valid_pid() {
        let tmp = TempDir::new().expect("create temp dir");
        let stdout_path = tmp.path().join("server.log");
        let cwd = tmp.path();

        // Use a long-running no-op command: ping -n 30 127.0.0.1
        // (30 seconds should be enough for the test).
        let cmd = vec![
            "ping".into(),
            "-n".into(),
            "30".into(),
            "127.0.0.1".into(),
        ];

        let pid = start_server(&cmd, &stdout_path, cwd);
        assert!(pid > 0, "start_server should return a positive PID, got {}", pid);

        // Verify the process is actually running.
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(is_process_running(pid), "Process {} should be running", pid);

        // Clean up.
        stop_server(pid);
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(!is_process_running(pid), "Process {} should be stopped", pid);
    }

    #[test]
    fn test_start_server_stdout_redirect() {
        let tmp = TempDir::new().expect("create temp dir");
        let stdout_path = tmp.path().join("server.log");
        let cwd = tmp.path();

        // Use a short command that writes output and exits.
        // "cmd /c echo hello" will print "hello" to stdout.
        let cmd = vec!["cmd".into(), "/c".into(), "echo".into(), "hello".into()];

        let pid = start_server(&cmd, &stdout_path, cwd);
        assert!(pid > 0, "start_server should return a positive PID");

        // Wait for the process to finish (short command).
        for _ in 0..20 {
            if !is_process_running(pid) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        // The log file should contain "hello".
        let content = std::fs::read_to_string(&stdout_path).unwrap_or_default();
        assert!(
            content.contains("hello"),
            "stdout should contain 'hello', got: {}",
            content
        );
    }

    #[test]
    fn test_start_server_invalid_cmd() {
        let tmp = TempDir::new().expect("create temp dir");
        let stdout_path = tmp.path().join("server.log");
        let cwd = tmp.path();

        let cmd = vec!["nonexistent_binary_that_does_not_exist".into()];
        let pid = start_server(&cmd, &stdout_path, cwd);
        assert_eq!(pid, 0, "start_server should return 0 for invalid command");
    }

    // ---- Finding 2: start_server mirrors stdin=DEVNULL ----

    #[test]
    fn test_start_server_stdin_is_devnull() {
        let tmp = TempDir::new().expect("create temp dir");
        let stdout_path = tmp.path().join("server.log");
        let cwd = tmp.path();

        // "cmd /c set /p var=" reads from stdin. With stdin=DEVNULL (NUL),
        // it should get EOF immediately and return without blocking.
        let cmd = vec!["cmd".into(), "/c".into(), "set".into(), "/p".into(), "var=".into()];

        let pid = start_server(&cmd, &stdout_path, cwd);
        assert!(pid > 0, "start_server should return a positive PID");

        // The process should exit quickly because stdin is NUL (EOF).
        let mut exited = false;
        for _ in 0..50 {
            if !is_process_running(pid) {
                exited = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(
            exited,
            "Process should exit quickly with stdin=DEVNULL (no blocking on input)"
        );
    }

    // ---- Acceptance: find_llama_server_pid ----

    #[test]
    fn test_find_llama_server_pid_no_server() {
        // Unless a llama-server is actually running, this should return 0.
        let pid = find_llama_server_pid();
        // We can't guarantee 0 (someone might have llama-server running),
        // but we can verify it doesn't panic and returns a valid i32.
        assert!(pid >= 0);
    }

    // ---- Finding 3: find_llama_server_pid parses PID from second CSV column ----

    #[test]
    fn test_find_llama_server_pid_csv_parsing() {
        // Verify parse_tasklist_csv correctly splits tasklist CSV output
        // and that PID is extracted from column index 1 (second column).
        let line = r#""llama-server.exe","12345","Console","1","52,432 K""#;
        let fields = parse_tasklist_csv(line);
        assert!(fields.len() >= 2, "should have at least 2 CSV fields");
        assert_eq!(fields[0], "llama-server.exe");
        assert_eq!(fields[1], "12345");
        // The memory field should preserve internal commas.
        assert!(fields.len() >= 5);
        // Memory field contains comma as thousands separator.
        assert_eq!(fields[4], "52,432 K");
    }

    // ---- Acceptance: stop_server ----

    #[test]
    fn test_stop_server_kills_process() {
        let tmp = TempDir::new().expect("create temp dir");
        let stdout_path = tmp.path().join("server.log");
        let cwd = tmp.path();

        let cmd = vec!["ping".into(), "-n".into(), "30".into(), "127.0.0.1".into()];
        let pid = start_server(&cmd, &stdout_path, cwd);
        assert!(pid > 0);
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(is_process_running(pid));

        stop_server(pid);
        std::thread::sleep(std::time::Duration::from_millis(1000));
        assert!(!is_process_running(pid), "Process should be killed after stop_server");
    }

    // ---- Acceptance: terminate_process ----

    #[test]
    fn test_terminate_process_invalid_pid() {
        assert!(!terminate_process(0));
        assert!(!terminate_process(-1));
    }

    #[test]
    fn test_terminate_process_nonexistent() {
        assert!(!terminate_process(99999999));
    }

    // ---- Acceptance: terminate_process kills a live process ----

    #[test]
    fn test_terminate_process_lifecycle() {
        let tmp = TempDir::new().expect("create temp dir");
        let stdout_path = tmp.path().join("server.log");
        let cwd = tmp.path();

        let cmd = vec!["ping".into(), "-n".into(), "30".into(), "127.0.0.1".into()];
        let pid = start_server(&cmd, &stdout_path, cwd);
        assert!(pid > 0);
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(is_process_running(pid));

        let result = terminate_process(pid);
        assert!(result, "terminate_process should succeed");
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(!is_process_running(pid));
    }

    // ---- Acceptance: get_process_exit_code ----

    #[test]
    fn test_get_process_exit_code_invalid_pid() {
        assert_eq!(get_process_exit_code(0), None);
        assert_eq!(get_process_exit_code(-1), None);
    }

    #[test]
    fn test_get_process_exit_code_nonexistent() {
        assert_eq!(get_process_exit_code(99999999), None);
    }

    // ---- Acceptance: build_command_line ----

    #[test]
    fn test_build_command_line_simple() {
        let cmd = vec!["ping".to_string(), "-n".to_string(), "5".to_string(), "127.0.0.1".to_string()];
        let result = build_command_line(&cmd);
        assert_eq!(result, "ping -n 5 127.0.0.1");
    }

    #[test]
    fn test_build_command_line_with_spaces() {
        let cmd = vec!["C:\\Program Files\\app.exe".to_string(), "--arg".to_string()];
        let result = build_command_line(&cmd);
        assert_eq!(result, r#""C:\Program Files\app.exe" --arg"#);
    }

    #[test]
    fn test_build_command_line_empty() {
        let cmd: Vec<String> = vec![];
        let result = build_command_line(&cmd);
        assert!(result.is_empty());
    }
}
