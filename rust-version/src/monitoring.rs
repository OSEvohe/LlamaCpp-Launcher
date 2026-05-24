//! System monitoring and log-tailing helpers.
//!
//! Mirrors ``llama_launcher/monitoring.py``: RAM via ``GlobalMemoryStatusEx``,
//! VRAM via ``nvidia-smi``, process RAM via ``tasklist``, and marker-based
//! log tailing.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
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
// Performance stats (prompt / generation speed parsed from llama.cpp logs)
// ---------------------------------------------------------------------------

/// Performance statistics gathered from llama-server log output.
#[derive(Debug, Clone)]
pub struct PerfStats {
    /// Tokens per second during prompt evaluation (latest).
    pub prompt_tps: Option<f64>,
    /// Tokens per second during text generation (latest).
    pub gen_tps: Option<f64>,
    /// ``true`` when a model load marker was seen.
    pub model_loaded: bool,
    /// Timestamp (UNIX epoch seconds) of the first model-load marker, or 0.
    pub model_loaded_at: u64,
    /// Elapsed seconds since model was loaded, or 0.
    pub model_uptime_secs: u64,
    /// Last recognized user prompt text (truncated to 200 chars).
    pub last_prompt: String,
    /// Byte offset used by ``refresh_and_get_perf_stats`` to tail only new
    /// log content (avoids re-parsing historical markers after a reset).
    pub last_log_size: usize,
    /// Marker string paired with ``last_log_size`` for rewrite detection.
    pub last_log_marker: String,
}

impl Default for PerfStats {
    fn default() -> Self {
        Self {
            prompt_tps: None,
            gen_tps: None,
            model_loaded: false,
            model_loaded_at: 0,
            model_uptime_secs: 0,
            last_prompt: String::new(),
            last_log_size: 0,
            last_log_marker: String::new(),
        }
    }
}

/// Lazy-loaded regex for prompt-eval timing lines.
///
/// Matches llama.cpp output such as:
/// ``prompt eval time = 1234.56 ms / 10 tokens (123.46 ms per token, 8.10 tokens per second)``
fn prompt_eval_re() -> &'static Regex {
    use once_cell::sync::Lazy;
    // Fallback if once_cell is not available — compile inline.
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?:prompt\s+eval\s+time|prompt_eval_time)\s*=\s*[\d.]+\s*ms\s*/\s*\d+\s*tokens\s*\([^)]*,\s*([\d.]+)\s*tokens?\s*(?:per\s+)?second\b",
        )
        .expect("compile prompt_eval regex")
    });
    &RE
}

/// Lazy-loaded regex for generation-eval timing lines.
///
/// Matches llama.cpp output such as:
/// ``eval time   = 4567.89 ms / 20 tokens (228.40 ms per token, 4.38 tokens per second)``
fn eval_re() -> &'static Regex {
    use once_cell::sync::Lazy;
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?:^\s*eval\s+time|eval_time)\s*=\s*[\d.]+\s*ms\s*/\s*\d+\s*tokens\s*\([^)]*,\s*([\d.]+)\s*tokens?\s*(?:per\s+)?second\b",
        )
        .expect("compile eval regex")
    });
    &RE
}

/// Lazy-loaded regex for model-load detection.
///
/// Matches lines like:
/// ``llama_model_loader: loaded model`` or ``llm_load_model_from_file:``
fn model_loaded_re() -> &'static Regex {
    use once_cell::sync::Lazy;
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?:llama_model_loader:\s*loaded\s+model|llm_load_model_from_file:|model\.gguf\s+loaded|model\s+loaded)",
        )
        .expect("compile model_loaded regex")
    });
    &RE
}

/// Lazy-loaded regex for user prompt detection.
///
/// Matches ``User prompt: ...`` or ``Inference started`` markers.
fn prompt_re() -> &'static Regex {
    use once_cell::sync::Lazy;
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?:User\s+prompt|prompt):\s*(.+)").expect("compile prompt regex")
    });
    &RE
}

/// Feed a log *chunk* into *stats*, updating in-place.
///
/// Scans each line for timing markers and model-load events.
pub fn feed_perf_stats(stats: &mut PerfStats, chunk: &str) {
    for line in chunk.lines() {
        // Prompt eval speed
        if let Some(caps) = prompt_eval_re().captures(line) {
            if let Some(m) = caps.get(1) {
                if let Ok(tps) = m.as_str().parse::<f64>() {
                    stats.prompt_tps = Some(tps);
                }
            }
        }

        // Generation eval speed
        if let Some(caps) = eval_re().captures(line) {
            if let Some(m) = caps.get(1) {
                if let Ok(tps) = m.as_str().parse::<f64>() {
                    stats.gen_tps = Some(tps);
                }
            }
        }

        // Model loaded marker
        if !stats.model_loaded && model_loaded_re().is_match(line) {
            stats.model_loaded = true;
            stats.model_loaded_at = now_unix_secs();
        }

        // User prompt
        if let Some(caps) = prompt_re().captures(line) {
            if let Some(m) = caps.get(1) {
                let text = m.as_str().trim();
                if !text.is_empty() {
                    let chars: Vec<char> = text.chars().collect();
                    stats.last_prompt = if chars.len() > 200 {
                        let truncated: String = chars.into_iter().take(200).collect();
                        format!("{}…", truncated)
                    } else {
                        text.to_string()
                    };
                }
            }
        }
    }

    // Update uptime
    if stats.model_loaded && stats.model_loaded_at > 0 {
        stats.model_uptime_secs = now_unix_secs().saturating_sub(stats.model_loaded_at);
    }
}

/// Reset *stats* to their default (empty) values.
pub fn reset_perf_stats(stats: &mut PerfStats) {
    *stats = PerfStats::default();
}

/// Refresh *model_uptime_secs* from the current wall clock.
///
/// Call before returning stats from read-only endpoints so the uptime
/// stays accurate even when no new log lines have been parsed.
pub fn refresh_perf_uptime(stats: &mut PerfStats) {
    if stats.model_loaded && stats.model_loaded_at > 0 {
        stats.model_uptime_secs = now_unix_secs().saturating_sub(stats.model_loaded_at);
    }
}

/// Return current UNIX epoch time in seconds.
fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

    // ---- Acceptance: feed_perf_stats — prompt eval speed ----

    #[test]
    fn test_feed_perf_stats_prompt_eval() {
        let mut stats = PerfStats::default();
        feed_perf_stats(
            &mut stats,
            "prompt eval time = 1234.56 ms / 10 tokens (123.46 ms per token, 8.10 tokens per second)\n",
        );
        assert_eq!(stats.prompt_tps, Some(8.10));
        assert!(!stats.model_loaded);
        assert!(stats.last_prompt.is_empty());
    }

    // ---- Acceptance: feed_perf_stats — generation eval speed ----

    #[test]
    fn test_feed_perf_stats_gen_eval() {
        let mut stats = PerfStats::default();
        feed_perf_stats(
            &mut stats,
            "eval time   = 4567.89 ms / 20 tokens (228.40 ms per token, 4.38 tokens per second)\n",
        );
        assert_eq!(stats.gen_tps, Some(4.38));
        assert!(stats.prompt_tps.is_none());
    }

    // ---- Acceptance: feed_perf_stats — model loaded marker ----

    #[test]
    fn test_feed_perf_stats_model_loaded() {
        let mut stats = PerfStats::default();
        feed_perf_stats(
            &mut stats,
            "llama_model_loader: loaded model\n",
        );
        assert!(stats.model_loaded);
        assert!(stats.model_loaded_at > 0);
        assert!(stats.model_uptime_secs >= 0);
    }

    // ---- Acceptance: feed_perf_stats — model loaded alternate formats ----

    #[test]
    fn test_feed_perf_stats_model_loaded_alternate() {
        let mut stats = PerfStats::default();
        feed_perf_stats(&mut stats, "llm_load_model_from_file:\n");
        assert!(stats.model_loaded);
    }

    #[test]
    fn test_feed_perf_stats_model_loaded_gguf_marker() {
        let mut stats = PerfStats::default();
        feed_perf_stats(&mut stats, "model.gguf loaded\n");
        assert!(stats.model_loaded);
    }

    // ---- Acceptance: feed_perf_stats — user prompt ----

    #[test]
    fn test_feed_perf_stats_user_prompt() {
        let mut stats = PerfStats::default();
        feed_perf_stats(
            &mut stats,
            "User prompt: Hello, how are you?\n",
        );
        assert_eq!(stats.last_prompt, "Hello, how are you?");
    }

    #[test]
    fn test_feed_perf_stats_prompt_truncated() {
        let mut stats = PerfStats::default();
        let long = "x".repeat(300);
        feed_perf_stats(&mut stats, &format!("User prompt: {}\n", long));
        assert_eq!(stats.last_prompt.chars().count(), 201); // 200 chars + "…"
        assert!(stats.last_prompt.ends_with("…"));
    }

    // ---- Fix: UTF-8 safe truncation (no panic on multi-byte boundaries) ----

    #[test]
    fn test_feed_perf_stats_prompt_truncated_multibyte_utf8() {
        let mut stats = PerfStats::default();
        // 300 Chinese characters (3 bytes each) — byte slicing at 200 would panic.
        let long = "中".repeat(300);
        feed_perf_stats(&mut stats, &format!("User prompt: {}\n", long));
        assert_eq!(stats.last_prompt.chars().count(), 201); // 200 chars + "…"
        assert!(stats.last_prompt.ends_with("…"));
        // First 200 chars are all "中".
        assert!(stats.last_prompt.starts_with(&"中".repeat(200)));
    }

    // ---- Acceptance: feed_perf_stats — combined chunk ----

    #[test]
    fn test_feed_perf_stats_combined() {
        let mut stats = PerfStats::default();
        let chunk = "llama_model_loader: loaded model\n\
                     User prompt: Tell me a joke\n\
                     prompt eval time = 500.00 ms / 5 tokens (100.00 ms per token, 10.00 tokens per second)\n\
                     eval time   = 2000.00 ms / 40 tokens (50.00 ms per token, 20.00 tokens per second)\n";
        feed_perf_stats(&mut stats, chunk);

        assert!(stats.model_loaded);
        assert_eq!(stats.last_prompt, "Tell me a joke");
        assert_eq!(stats.prompt_tps, Some(10.00));
        assert_eq!(stats.gen_tps, Some(20.00));
    }

    // ---- Acceptance: feed_perf_stats — latest values win ----

    #[test]
    fn test_feed_perf_stats_latest_wins() {
        let mut stats = PerfStats::default();
        feed_perf_stats(
            &mut stats,
            "prompt eval time = 100.00 ms / 5 tokens (20.00 ms per token, 50.00 tokens per second)\n",
        );
        assert_eq!(stats.prompt_tps, Some(50.00));

        feed_perf_stats(
            &mut stats,
            "prompt eval time = 200.00 ms / 10 tokens (20.00 ms per token, 50.00 tokens per second)\n\
             eval time   = 1000.00 ms / 20 tokens (50.00 ms per token, 20.00 tokens per second)\n",
        );
        assert_eq!(stats.prompt_tps, Some(50.00));
        assert_eq!(stats.gen_tps, Some(20.00));
    }

    // ---- Acceptance: reset_perf_stats ----

    #[test]
    fn test_reset_perf_stats() {
        let mut stats = PerfStats {
            prompt_tps: Some(10.0),
            gen_tps: Some(5.0),
            model_loaded: true,
            model_loaded_at: 12345,
            model_uptime_secs: 100,
            last_prompt: "test".to_string(),
            last_log_size: 42,
            last_log_marker: "marker".to_string(),
        };
        reset_perf_stats(&mut stats);
        assert_eq!(stats.prompt_tps, None);
        assert_eq!(stats.gen_tps, None);
        assert!(!stats.model_loaded);
        assert_eq!(stats.model_loaded_at, 0);
        assert_eq!(stats.model_uptime_secs, 0);
        assert!(stats.last_prompt.is_empty());
        assert_eq!(stats.last_log_size, 0);
        assert!(stats.last_log_marker.is_empty());
    }

    // ---- Acceptance: feed_perf_stats — no false positives ----

    #[test]
    fn test_feed_perf_stats_no_false_positives() {
        let mut stats = PerfStats::default();
        feed_perf_stats(
            &mut stats,
            "some random log line\n\
             another line without timing info\n\
             [info] server ready\n",
        );
        assert!(stats.prompt_tps.is_none());
        assert!(stats.gen_tps.is_none());
        assert!(!stats.model_loaded);
        assert!(stats.last_prompt.is_empty());
    }

    // ---- Fix: refresh_perf_uptime updates uptime without new logs ----

    #[test]
    fn test_refresh_perf_uptime_increases() {
        let mut stats = PerfStats::default();
        feed_perf_stats(&mut stats, "llama_model_loader: loaded model\n");
        let uptime1 = stats.model_uptime_secs;

        std::thread::sleep(std::time::Duration::from_secs(1));

        refresh_perf_uptime(&mut stats);
        let uptime2 = stats.model_uptime_secs;

        assert!(
            uptime2 > uptime1,
            "uptime should increase after refresh ({} vs {})",
            uptime2,
            uptime1
        );
    }

    #[test]
    fn test_refresh_perf_uptime_noop_when_not_loaded() {
        let mut stats = PerfStats::default();
        refresh_perf_uptime(&mut stats);
        assert_eq!(stats.model_uptime_secs, 0);
    }
}
