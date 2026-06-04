use std::path::{Path, PathBuf};

/// Recursively search *dir* for ``llama-server.exe``, returning the full path.
pub(crate) fn find_exe_in_dir(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries {
        let entry = entry.ok()?;
        if entry.file_name() == "llama-server.exe" {
            return Some(entry.path());
        }
    }
    // Check one level deep
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries {
        let entry = entry.ok()?;
        if entry.file_type().ok()?.is_dir() {
            if let Some(found) = find_exe_in_dir(&entry.path()) {
                return Some(found);
            }
        }
    }
    None
}
