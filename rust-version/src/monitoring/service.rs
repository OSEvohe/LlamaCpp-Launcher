//! MonitoringService — TDA facade that owns the internal log cursor
//! and performance stats, and delegates system metrics collection.

use std::path::{Path, PathBuf};

use crate::monitoring::log_tailer::{tail_log_chunk, LogCursor};
use crate::monitoring::perf::{feed_perf_stats, refresh_perf_uptime, reset_perf_stats, PerfSnapshot, PerfStats};
use crate::monitoring::system::{bytes_to_gb, gpu_vram_info, process_ram_bytes, ram_usage_bytes, SystemMetrics};

/// Encapsulates all monitoring state and behavior.
///
/// Follows TDA: the service tells itself to refresh, and the caller
/// asks for a snapshot.  Cursor state is kept internal.
pub struct MonitoringService {
    /// Path to the llama-server stdout log.
    log_path: PathBuf,
    /// Internal cursor for incremental log tailing.
    cursor: LogCursor,
    /// Observable performance statistics.
    stats: PerfStats,
}

impl MonitoringService {
    /// Create a new monitoring service watching *log_path*.
    pub fn new(log_path: PathBuf) -> Self {
        Self {
            log_path,
            cursor: LogCursor::default(),
            stats: PerfStats::default(),
        }
    }

    // -- Tell: "refresh yourself from the log" --------------------------------

    /// Tail the log for new content, feed perf stats, refresh uptime.
    ///
    /// Uses the internal cursor to tail **only new content** — avoids
    /// re-parsing historical markers (e.g. model-load) that would undo
    /// a reset.
    pub fn refresh(&mut self) {
        if self.log_path.exists() {
            let (chunk, new_size, reset, new_marker) = tail_log_chunk(
                &self.log_path,
                self.cursor.last_size,
                &self.cursor.last_marker,
            );
            if reset {
                // Log was truncated or rewritten — feed the recovery chunk
                // (full current file) into perf stats, then advance cursor.
                feed_perf_stats(&mut self.stats, &chunk);
                self.cursor.last_size = new_size;
                self.cursor.last_marker = new_marker;
            } else if !chunk.is_empty() {
                feed_perf_stats(&mut self.stats, &chunk);
                self.cursor.last_size = new_size;
                self.cursor.last_marker = new_marker;
            }
            refresh_perf_uptime(&mut self.stats);
        } else {
            // No log file — just refresh uptime from stored timestamp.
            refresh_perf_uptime(&mut self.stats);
        }
    }

    // -- Ask: "give me a snapshot" --------------------------------------------

    /// Return a read-only snapshot of the current performance statistics.
    pub fn snapshot(&self) -> PerfSnapshot {
        PerfSnapshot::from(&self.stats)
    }

    /// Return a cloned ``PerfStats`` for backward compatibility.
    pub fn stats_clone(&self) -> PerfStats {
        self.stats.clone()
    }

    // -- Tell: "reset your perf stats" ----------------------------------------

    /// Reset observable perf stats to defaults.
    ///
    /// Preserves the internal cursor so the next ``refresh()`` only sees
    /// *new* log content — avoids re-injecting historical markers after
    /// a reset.
    pub fn reset_perf(&mut self) {
        reset_perf_stats(&mut self.stats);
        // cursor is intentionally preserved
    }

    /// Full reset: clear both observable stats and the internal cursor.
    ///
    /// Used on server launch/restart when a fresh log file is created.
    pub fn full_reset(&mut self) {
        self.stats = PerfStats::default();
        self.cursor = LogCursor::default();
    }

    // -- System metrics (stateless, static) -----------------------------------

    /// Collect current system resource usage.
    pub fn system_metrics(&self) -> SystemMetrics {
        SystemMetrics::collect()
    }

    /// Return ``(used_bytes, total_bytes)`` of physical RAM.
    pub fn ram_usage_bytes(&self) -> (u64, u64) {
        ram_usage_bytes()
    }

    /// Return ``(used_bytes, total_bytes)`` of GPU VRAM.
    pub fn gpu_vram_info(&self) -> (u64, u64) {
        gpu_vram_info()
    }

    /// Return approximate RAM usage (bytes) of the process with *pid*.
    pub fn process_ram_bytes(&self, pid: i32) -> u64 {
        process_ram_bytes(pid)
    }

    /// Format *value* (bytes) as a human-readable GB string.
    pub fn format_bytes(&self, value: u64) -> String {
        bytes_to_gb(value)
    }

    // -- Client-facing log tail (separate from internal cursor) ---------------

    /// Read new content appended to the log file since *last_size*.
    ///
    /// Uses the *client's* cursor (last_size/last_marker).  Feeding stats
    /// from a client cursor would undo /api/perf/reset when a slow client
    /// re-scans historical log content.  Perf stats are driven solely by
    /// the internal cursor in ``refresh()``.
    pub fn tail_log_for_client(
        &self,
        log_path: &Path,
        last_size: usize,
        last_marker: &str,
    ) -> (String, usize, bool, String) {
        if !log_path.exists() {
            return (String::new(), last_size, false, last_marker.to_string());
        }
        tail_log_chunk(log_path, last_size, last_marker)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_monitoring_service_initial_state() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");
        let svc = MonitoringService::new(log_path.clone());

        let snap = svc.snapshot();
        assert!(snap.prompt_tps.is_none());
        assert!(snap.gen_tps.is_none());
        assert!(!snap.model_loaded);
        assert_eq!(snap.model_loaded_at, 0);
        assert!(snap.last_prompt.is_empty());
    }

    #[test]
    fn test_monitoring_service_refresh_picks_up_stats() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");

        std::fs::write(
            &log_path,
            "llama_model_loader: loaded model\n\
             User prompt: Hello\n\
             prompt eval time = 100.00 ms / 5 tokens (20.00 ms per token, 50.00 tokens per second)\n",
        )
        .expect("write log");

        let mut svc = MonitoringService::new(log_path);
        svc.refresh();

        let snap = svc.snapshot();
        assert!(snap.model_loaded);
        assert_eq!(snap.last_prompt, "Hello");
        assert_eq!(snap.prompt_tps, Some(50.00));
    }

    #[test]
    fn test_monitoring_service_reset_preserves_cursor() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");

        std::fs::write(
            &log_path,
            "llama_model_loader: loaded model\n\
             User prompt: Hello\n\
             prompt eval time = 100.00 ms / 5 tokens (20.00 ms per token, 50.00 tokens per second)\n",
        )
        .expect("write log");

        let mut svc = MonitoringService::new(log_path);
        svc.refresh();
        assert!(svc.snapshot().model_loaded);

        // Reset clears observable stats but preserves cursor.
        svc.reset_perf();
        let snap = svc.snapshot();
        assert!(!snap.model_loaded);
        assert!(snap.prompt_tps.is_none());
        assert!(snap.last_prompt.is_empty());

        // Append new content — cursor should still advance from where it left off.
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&svc.log_path)
            .unwrap()
            .write_all(b"eval time   = 1000.00 ms / 20 tokens (50.00 ms per token, 20.00 tokens per second)\n")
            .unwrap();

        svc.refresh();
        let snap = svc.snapshot();
        // model_loaded stays false because cursor skipped the historical marker.
        assert!(!snap.model_loaded);
        assert_eq!(snap.gen_tps, Some(20.00));
    }

    #[test]
    fn test_monitoring_service_system_metrics() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");
        let svc = MonitoringService::new(log_path);

        let metrics = svc.system_metrics();
        assert!(metrics.ram_total > 0, "total RAM should be > 0");
        assert!(metrics.ram_used <= metrics.ram_total);
    }

    #[test]
    fn test_monitoring_service_tail_log_for_client() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("test.log");
        let svc = MonitoringService::new(log_path.clone());

        std::fs::write(&log_path, "line1\n").unwrap();

        let (chunk, size, reset, marker) = svc.tail_log_for_client(&log_path, 0, "");
        assert_eq!(chunk, "line1\n");
        assert_eq!(size, 6);
        assert!(!reset);
    }

    #[test]
    fn test_monitoring_service_tail_log_missing_file() {
        let tmp = TempDir::new().expect("create temp dir");
        let log_path = tmp.path().join("nonexistent.log");
        let svc = MonitoringService::new(log_path.clone());

        let (chunk, size, reset, marker) = svc.tail_log_for_client(&log_path, 0, "");
        assert_eq!(chunk, "");
        assert_eq!(size, 0);
        assert!(!reset);
        assert_eq!(marker, "");
    }
}
