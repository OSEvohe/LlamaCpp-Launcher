//! System monitoring and log-tailing helpers.
//!
//! Mirrors ``llama_launcher/monitoring.py``: RAM via ``GlobalMemoryStatusEx``,
//! VRAM via ``nvidia-smi``, process RAM via ``tasklist``, and marker-based
//! log tailing.

use std::path::Path;

use winapi::shared::minwindef::DWORD;
use winapi::um::sysinfoapi::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

/// Marker length in bytes for rewrite detection (mirrors Python ``_MARKER_LEN = 64``).
const MARKER_LEN: usize = 64;

/// Format *value* (bytes) as a human-readable GB string like ``12.3GB``.
pub fn bytes_to_gb(value: u64) -> String {
    let gb = value as f64 / (1024.0 * 1024.0 * 1024.0);
    format!("{:.1}GB", gb)
}

/// Return ``(used_bytes, total_bytes)`` of physical RAM via Windows API.
pub fn ram_usage_bytes() -> (u64, u64) {
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as DWORD;

    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return (0, 0);
    }

    let total = status.ullTotalPhys as u64;
    let avail = status.ullAvailPhys as u64;
    let used = total.saturating_sub(avail);
    (used, total)
}

/// Return ``(used_bytes, total_bytes)`` of GPU VRAM via ``nvidia-smi``.
///
/// Aggregates across all GPUs when multiple are present.
/// Returns ``(0, 0)`` when ``nvidia-smi`` is absent or fails.
pub fn gpu_vram_info() -> (u64, u64) {
    let output = match std::process::Command::new("nvidia-smi")
        .args(&[
            "--query-gpu=memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        Ok(o) => o,
        Err(_) => return (0, 0),
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();

    if lines.is_empty() {
        return (0, 0);
    }

    let mut used_sum: u64 = 0;
    let mut total_sum: u64 = 0;

    for line in &lines {
        let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if parts.len() < 2 {
            continue;
        }
        if let Ok(used_mb) = parts[0].parse::<u64>() {
            used_sum += used_mb * 1024 * 1024;
        }
        if let Ok(total_mb) = parts[1].parse::<u64>() {
            total_sum += total_mb * 1024 * 1024;
        }
    }

    (used_sum, total_sum)
}

/// Return approximate RAM usage (bytes) of the process with *pid*.
///
/// Uses ``tasklist`` CSV output, parsing the 5th column (memory usage).
pub fn process_ram_bytes(pid: i32) -> u64 {
    if pid <= 0 {
        return 0;
    }
    let output = match std::process::Command::new("tasklist")
        .args(&["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return 0,
    };

    let out = String::from_utf8_lossy(&output.stdout).to_string();
    let out = out.trim();

    if out.is_empty() || out.to_uppercase().contains("INFO:") {
        return 0;
    }

    // Parse CSV: tasklist CSV output has quoted fields like "name","pid","session","type","52,432 K"
    // We need to handle commas inside quoted fields (memory column uses commas as thousands separators).
    // Strategy: match all quoted fields with regex, or use a simple state machine.
    let row: Vec<String> = parse_tasklist_csv(out);

    if row.len() < 5 {
        return 0;
    }

    // 5th column (index 4) is memory usage, e.g., "1,234,568 K" or "71 328 Ko" (French locale).
    // Strip all non-digit characters to handle locale-specific formatting.
    let mem_field: String = row[4].chars().filter(|c| c.is_ascii_digit()).collect();

    if let Ok(kb) = mem_field.parse::<u64>() {
        kb * 1024
    } else {
        0
    }
}

/// Parse a single-line ``tasklist`` CSV output into fields.
///
/// Handles commas inside quoted fields (e.g., memory column ``"52,432 K"``).
pub fn parse_tasklist_csv(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in line.chars() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c == ',' && !in_quotes {
            fields.push(current.trim().to_string());
            current = String::new();
        } else {
            current.push(c);
        }
    }
    fields.push(current.trim().to_string());
    fields
}

/// Build a two-line ``RAM … / VRAM …`` string suitable for display.
pub fn build_monitoring_text() -> String {
    let (used_ram, total_ram) = ram_usage_bytes();
    let ram_line = if total_ram > 0 {
        format!("RAM: {}/{}", bytes_to_gb(used_ram), bytes_to_gb(total_ram))
    } else {
        "RAM: N/A".to_string()
    };

    let (used_vram, total_vram) = gpu_vram_info();
    let vram_line = if total_vram > 0 {
        format!("VRAM: {}/{}", bytes_to_gb(used_vram), bytes_to_gb(total_vram))
    } else {
        "VRAM: N/A".to_string()
    };

    format!("{}\n{}", ram_line, vram_line)
}

/// Read new content appended to *path* since *last_size*.
///
/// Returns ``(chunk_text, new_size, reset_required, new_marker)``.
///
/// - ``reset_required`` is ``true`` only when the file was genuinely
///   truncated (current size < last_size), when the file is empty
///   and the caller had previously seen content (last_size > 0),
///   or when a rewrite/replacement is detected (marker mismatch).
/// - When ``reset_required`` is ``true``, ``chunk_text`` contains the
///   full current file content so the caller can clear and repopulate
///   in a single step without a second read.
/// - A steady empty file (last_size == 0, current == 0) returns
///   ``("", 0, false, "")`` — a no-op, not a truncation.
/// - *prev_marker* is the tail of the previously-seen prefix (last
///   ``MARKER_LEN`` bytes up to *last_size*).  When the file grows
///   or stays the same size, the marker is re-checked at the
///   expected boundary; a mismatch indicates a rewrite and triggers
///   a reset.  The returned *new_marker* should be stored by the
///   caller for the next poll.
pub fn tail_log_chunk(
    path: &Path,
    last_size: usize,
    prev_marker: &str,
) -> (String, usize, bool, String) {
    // Read raw bytes then decode with lossy replacement, matching Python's
    // ``read_text(encoding="utf-8", errors="replace")`` — invalid UTF-8 bytes
    // become ``\u{FFFD}`` instead of aborting with an empty fallback.
    let raw = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return (String::new(), 0, false, String::new()),
    };
    let data: Vec<char> = String::from_utf8_lossy(&raw).chars().collect();
    let current = data.len();

    if last_size > current {
        // Genuine truncation: return full content for single-step recovery.
        let marker_start = if current > MARKER_LEN {
            current - MARKER_LEN
        } else {
            0
        };
        let new_marker: String = data[marker_start..current].iter().collect();
        let full: String = data.iter().collect();
        return (full, current, true, new_marker);
    }

    if last_size > 0 && current == 0 {
        // File was emptied (truncated to zero): treat as truncation.
        return (String::new(), 0, true, String::new());
    }

    if current <= last_size {
        // current == last_size > 0: check for equal-size rewrite.
        if !prev_marker.is_empty() {
            let marker_start = if last_size > MARKER_LEN {
                last_size - MARKER_LEN
            } else {
                0
            };
            let stored_marker: String = data[marker_start..last_size].iter().collect();
            if stored_marker != prev_marker {
                let new_marker_start = if current > MARKER_LEN {
                    current - MARKER_LEN
                } else {
                    0
                };
                let new_marker: String = data[new_marker_start..current].iter().collect();
                let full: String = data.iter().collect();
                return (full, current, true, new_marker);
            }
        }
        // Steady state (covers last_size == 0, current == 0).
        let marker_start = if current > MARKER_LEN {
            current - MARKER_LEN
        } else {
            0
        };
        let new_marker: String = data[marker_start..current].iter().collect();
        return (String::new(), current, false, new_marker);
    }

    // current > last_size: new content appended.
    let chunk: String = data[last_size..].iter().collect();

    // Verify marker at boundary (rewrite detection).
    if !prev_marker.is_empty() && last_size > 0 {
        let marker_start = if last_size > MARKER_LEN {
            last_size - MARKER_LEN
        } else {
            0
        };
        let stored_marker: String = data[marker_start..last_size].iter().collect();
        if stored_marker != prev_marker {
            let new_marker_start = if current > MARKER_LEN {
                current - MARKER_LEN
            } else {
                0
            };
            let new_marker: String = data[new_marker_start..current].iter().collect();
            let full: String = data.iter().collect();
            return (full, current, true, new_marker);
        }
    }

    let marker_start = if current > MARKER_LEN {
        current - MARKER_LEN
    } else {
        0
    };
    let new_marker: String = data[marker_start..current].iter().collect();
    (chunk, current, false, new_marker)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ---- Finding 3: parse_tasklist_csv handles commas inside quoted fields ----

    #[test]
    fn test_parse_tasklist_csv_commas_in_fields() {
        // Simulate tasklist CSV output where the memory field has commas.
        let line = r#""llama-server.exe","12345","Console","1","52,432 K""#;
        let fields = parse_tasklist_csv(line);
        assert_eq!(fields.len(), 5);
        assert_eq!(fields[0], "llama-server.exe");
        assert_eq!(fields[1], "12345");
        assert_eq!(fields[2], "Console");
        assert_eq!(fields[3], "1");
        assert_eq!(fields[4], "52,432 K");
    }

    #[test]
    fn test_parse_tasklist_csv_simple() {
        let line = r#""notepad.exe","9876","Console","1","100 K""#;
        let fields = parse_tasklist_csv(line);
        assert_eq!(fields.len(), 5);
        assert_eq!(fields[0], "notepad.exe");
        assert_eq!(fields[1], "9876");
    }

    // ---- Acceptance: bytes_to_gb ----

    #[test]
    fn test_bytes_to_gb() {
        assert_eq!(bytes_to_gb(0), "0.0GB");
        assert_eq!(bytes_to_gb(1_073_741_824), "1.0GB"); // exactly 1 GB
        assert_eq!(bytes_to_gb(13_314_400_618), "12.4GB"); // exactly 12.4 GB
    }

    // ---- Acceptance: ram_usage_bytes returns non-zero on a machine with RAM ----

    #[test]
    fn test_ram_usage_bytes_nonzero() {
        let (used, total) = ram_usage_bytes();
        assert!(total > 0, "total RAM should be > 0 on a real machine");
        assert!(used >= 0, "used RAM should be >= 0");
        assert!(used <= total, "used RAM should not exceed total");
    }

    // ---- Acceptance: gpu_vram_info returns (0, 0) when nvidia-smi is absent ----

    #[test]
    fn test_gpu_vram_info_no_nvidia_smi() {
        // On a machine without nvidia-smi, this should return (0, 0).
        // On a machine with it, we just verify the values are plausible.
        let (used, total) = gpu_vram_info();
        assert!(used >= 0);
        assert!(total >= 0);
        // We can't assert (0, 0) because the test machine might have a GPU.
    }

    // ---- Acceptance: process_ram_bytes ----

    #[test]
    fn test_process_ram_bytes_invalid_pid() {
        assert_eq!(process_ram_bytes(0), 0);
        assert_eq!(process_ram_bytes(-1), 0);
    }

    #[test]
    fn test_process_ram_bytes_self() {
        let my_pid = std::process::id() as i32;
        let ram = process_ram_bytes(my_pid);
        assert!(ram > 0, "Our own process should have some RAM usage");
    }

    // ---- Acceptance: build_monitoring_text ----

    #[test]
    fn test_build_monitoring_text_format() {
        let text = build_monitoring_text();
        assert!(text.contains("RAM:"));
        assert!(text.contains("VRAM:"));
        assert!(text.contains('\n'));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
    }

    // ---- Acceptance: tail_log_chunk — append detection ----

    #[test]
    fn test_tail_log_chunk_append() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");

        // Initial write.
        std::fs::write(&log_path, "line1\n").unwrap();
        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 0, "");
        assert_eq!(chunk, "line1\n");
        assert_eq!(size, 6);
        assert!(!reset);
        assert!(!marker.is_empty());

        // Append more content.
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap()
            .write_all(b"line2\n")
            .unwrap();

        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 6, &marker);
        assert_eq!(chunk, "line2\n");
        assert_eq!(size, 12);
        assert!(!reset);
    }

    // ---- Acceptance: tail_log_chunk — truncation detection ----

    #[test]
    fn test_tail_log_chunk_truncation() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");

        // Write initial content.
        std::fs::write(&log_path, "long content here\n").unwrap();
        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 0, "");
        assert_eq!(chunk, "long content here\n");
        assert_eq!(size, 18);
        assert!(!reset);

        // Truncate the file (write shorter content).
        std::fs::write(&log_path, "short\n").unwrap();

        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 18, &marker);
        assert_eq!(chunk, "short\n");
        assert_eq!(size, 6);
        assert!(reset, "truncation should trigger reset_required");
    }

    // ---- Acceptance: tail_log_chunk — equal-size rewrite detection ----

    #[test]
    fn test_tail_log_chunk_equal_size_rewrite() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");

        // Write initial content.
        std::fs::write(&log_path, "AAAA\n").unwrap();
        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 0, "");
        assert_eq!(chunk, "AAAA\n");
        assert_eq!(size, 5);
        assert!(!reset);

        // Rewrite with same-length but different content.
        std::fs::write(&log_path, "BBBB\n").unwrap();

        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 5, &marker);
        assert_eq!(chunk, "BBBB\n");
        assert_eq!(size, 5);
        assert!(reset, "equal-size rewrite should trigger reset_required");
    }

    // ---- Acceptance: tail_log_chunk — steady empty file ----

    #[test]
    fn test_tail_log_chunk_steady_empty() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");

        // Create empty file.
        std::fs::write(&log_path, "").unwrap();

        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 0, "");
        assert_eq!(chunk, "");
        assert_eq!(size, 0);
        assert!(!reset, "steady empty file should not trigger reset");
        assert_eq!(marker, "");
    }

    // ---- Acceptance: tail_log_chunk — file emptied after content ----

    #[test]
    fn test_tail_log_chunk_file_emptied() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");

        // Write initial content.
        std::fs::write(&log_path, "some content\n").unwrap();
        let (_chunk, size, _reset, marker) = tail_log_chunk(&log_path, 0, "");
        assert_eq!(size, 13);

        // Empty the file.
        std::fs::write(&log_path, "").unwrap();

        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 13, &marker);
        assert_eq!(chunk, "");
        assert_eq!(size, 0);
        assert!(reset, "emptying file should trigger reset_required");
    }

    // ---- Acceptance: tail_log_chunk — append with marker mismatch ----

    #[test]
    fn test_tail_log_chunk_append_with_marker_mismatch() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");

        // Write initial content.
        std::fs::write(&log_path, "AAAA\n").unwrap();
        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 0, "");
        assert_eq!(size, 5);
        assert!(!reset);

        // Simulate a rewrite: file now has different prefix + new content.
        std::fs::write(&log_path, "XXXX\nnew line\n").unwrap();

        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 5, &marker);
        assert!(reset, "marker mismatch during append should trigger reset");
        assert_eq!(size, 14);
        assert_eq!(chunk, "XXXX\nnew line\n");
    }

    // ---- Acceptance: tail_log_chunk — marker with small file (< MARKER_LEN) ----

    #[test]
    fn test_tail_log_chunk_small_file_marker() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");

        // Write content smaller than MARKER_LEN (64 bytes).
        std::fs::write(&log_path, "tiny\n").unwrap();
        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 0, "");
        assert_eq!(chunk, "tiny\n");
        assert_eq!(size, 5);
        assert!(!reset);
        // Marker should be the entire file content (since it's < 64 bytes).
        assert_eq!(marker, "tiny\n");

        // Append more.
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap()
            .write_all(b"data\n")
            .unwrap();

        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 5, &marker);
        assert_eq!(chunk, "data\n");
        assert_eq!(size, 10);
        assert!(!reset);
    }

    // ---- Acceptance: tail_log_chunk — missing file ----

    #[test]
    fn test_tail_log_chunk_missing_file() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("nonexistent.log");

        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 0, "");
        assert_eq!(chunk, "");
        assert_eq!(size, 0);
        assert!(!reset);
        assert_eq!(marker, "");
    }

    // ---- Acceptance: tail_log_chunk — no-op on unchanged content ----

    #[test]
    fn test_tail_log_chunk_noop_unchanged() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");

        std::fs::write(&log_path, "unchanged content\n").unwrap();
        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 0, "");
        assert_eq!(chunk, "unchanged content\n");
        assert_eq!(size, 18);

        // Poll again without changes.
        let (chunk, size, reset, marker) = tail_log_chunk(&log_path, 18, &marker);
        assert_eq!(chunk, "");
        assert_eq!(size, 18);
        assert!(!reset);
    }

    // ---- Finding 1: tail_log_chunk tolerates non-UTF8 like Python errors='replace' ----

    #[test]
    fn test_tail_log_chunk_non_utf8_lossy() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");

        // Write valid UTF-8 prefix + invalid UTF-8 bytes (0xFF 0xFE are invalid alone)
        // + valid UTF-8 suffix.  Python's errors='replace' turns each bad byte into
        // the Unicode replacement character \u{FFFD}.
        let raw: Vec<u8> = b"hello ".to_vec()
            .into_iter()
            .chain(vec![0xFF, 0xFE])
            .chain(b" world".to_vec())
            .collect();
        std::fs::write(&log_path, &raw).unwrap();

        let (chunk, size, reset, _marker) = tail_log_chunk(&log_path, 0, "");
        assert!(!reset);
        // Must NOT be empty — the lossy decode should produce content with \u{FFFD}.
        assert!(!chunk.is_empty(), "non-UTF8 file must not return empty fallback");
        assert!(chunk.contains('\u{FFFD}'), "invalid bytes should be replaced with U+FFFD");
        assert_eq!(chunk, "hello \u{FFFD}\u{FFFD} world");
        // Size is character count after lossy decode.
        assert_eq!(size, "hello \u{FFFD}\u{FFFD} world".chars().count());
    }
}
