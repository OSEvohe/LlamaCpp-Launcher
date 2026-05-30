//! Download, extract, validate, and register llama.cpp versions.
//!
//! Handles the full install lifecycle:
//! 1. Download zip to a temp file
//! 2. Extract to a staging directory
//! 3. Validate ``llama-server.exe`` exists
//! 4. Move to final install path
//! 5. Register metadata in global settings

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;
use zip::ZipArchive;

// ---------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------

/// Download a file from *url* to *dest*, reporting progress via *on_progress*.
///
/// *on_progress* is called with ``(downloaded_bytes, total_bytes_or_0)*.
pub async fn download_file<F>(
    url: &str,
    dest: &Path,
    on_progress: F,
) -> Result<u64, String>
where
    F: Fn(u64, u64),
{
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300)) // 5 min for large downloads
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Download failed with HTTP {}", resp.status().as_u16()));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("Failed to create temp file: {}", e))?;
    let mut downloaded: u64 = 0;

    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download stream error: {}", e))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Failed to write chunk: {}", e))?;
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total);
    }

    Ok(downloaded)
}

// ---------------------------------------------------------------------------
// Extract
// ---------------------------------------------------------------------------

/// Extract a zip file to *dest_dir*. Returns the path to ``llama-server.exe``
/// if found inside the extracted tree.
pub fn extract_zip(zip_path: &Path, dest_dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("Failed to open zip file: {}", e))?;

    let mut archive = ZipArchive::new(file)
        .map_err(|e| format!("Failed to read zip archive: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| format!("Failed to read zip entry {}: {}", i, e))?;
        let path = file.enclosed_name().ok_or_else(|| {
            format!("Unsafe zip entry path: {:?}", file.name())
        })?;
        let out_path = dest_dir.join(&path);

        if file.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create dir {:?}: {}", out_path, e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent dir: {}", e))?;
            }
            let mut outfile = std::fs::File::create(&out_path)
                .map_err(|e| format!("Failed to create file {:?}: {}", out_path, e))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("Failed to extract {:?}: {}", path, e))?;
        }
    }

    // Find llama-server.exe
    find_llama_server_exe(dest_dir)
}

/// Recursively search *dir* for ``llama-server.exe``.
fn find_llama_server_exe(dir: &Path) -> Result<PathBuf, String> {
    if !dir.is_dir() {
        return Err(format!("{:?} is not a directory", dir));
    }

    // Check direct children first
    for entry in std::fs::read_dir(dir).map_err(|e| format!("Failed to read dir {:?}: {}", dir, e))? {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let name = entry.file_name();
        if name.to_string_lossy() == "llama-server.exe" {
            return Ok(entry.path());
        }
        // Check immediate subdirectories
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            for sub_entry in std::fs::read_dir(entry.path()).map_err(|e| format!("Failed to read sub-dir: {}", e))? {
                let sub = sub_entry.map_err(|e| format!("Failed to read sub-entry: {}", e))?;
                if sub.file_name().to_string_lossy() == "llama-server.exe" {
                    return Ok(sub.path());
                }
            }
        }
    }

    Err("llama-server.exe not found in extracted files".into())
}

// ---------------------------------------------------------------------------
// Cleanup helpers
// ---------------------------------------------------------------------------

/// Remove a directory tree (best-effort, ignores errors).
pub fn remove_dir_all_force(dir: &Path) {
    std::fs::remove_dir_all(dir).ok();
}

/// Remove a file (best-effort, ignores errors).
pub fn remove_file_force(path: &Path) {
    std::fs::remove_file(path).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_llama_server_exe_not_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("other.exe"), b"").ok();
        let result = find_llama_server_exe(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_find_llama_server_exe_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("llama-server.exe"), b"").ok();
        let path = find_llama_server_exe(dir.path()).unwrap();
        assert!(path.ends_with("llama-server.exe"));
    }

    #[test]
    fn test_find_llama_server_exe_subdir() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("bin");
        std::fs::create_dir_all(&sub).ok();
        std::fs::write(sub.join("llama-server.exe"), b"").ok();
        let path = find_llama_server_exe(dir.path()).unwrap();
        assert!(path.ends_with("llama-server.exe"));
    }
}
