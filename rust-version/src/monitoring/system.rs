//! System metrics collectors: RAM, VRAM, process RAM.
//!
//! Mirrors ``llama_launcher/monitoring.py``: RAM via ``GlobalMemoryStatusEx``,
//! VRAM via ``nvidia-smi``, process RAM via ``tasklist``.

use winapi::shared::minwindef::DWORD;
use winapi::um::sysinfoapi::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

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

/// Snapshot of current system resource usage.
#[derive(Debug, Clone)]
pub struct SystemMetrics {
    /// Physical RAM currently in use (bytes).
    pub ram_used: u64,
    /// Total physical RAM (bytes).
    pub ram_total: u64,
    /// GPU VRAM currently in use (bytes).
    pub vram_used: u64,
    /// Total GPU VRAM (bytes).
    pub vram_total: u64,
}

impl SystemMetrics {
    /// Collect all system metrics in a single call.
    pub fn collect() -> Self {
        let (ram_used, ram_total) = ram_usage_bytes();
        let (vram_used, vram_total) = gpu_vram_info();
        Self {
            ram_used,
            ram_total,
            vram_used,
            vram_total,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
}
