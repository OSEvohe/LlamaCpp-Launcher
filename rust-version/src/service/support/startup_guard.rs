pub(crate) fn ensure_startup_pid(action: &str, pid: i32) -> Result<(), String> {
    if pid > 0 {
        Ok(())
    } else {
        Err(format!("startup profile {} failed to start process", action))
    }
}
