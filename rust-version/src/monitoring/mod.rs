//! System monitoring and log-tailing helpers.
//!
//! Mirrors ``llama_launcher/monitoring.py``: RAM via ``GlobalMemoryStatusEx``,
//! VRAM via ``nvidia-smi``, process RAM via ``tasklist``, and marker-based
//! log tailing.
//!
//! # Module layout
//!
//! - ``system`` — RAM, VRAM, process RAM collectors
//! - ``log_tailer`` — tail_log_chunk, cursor/marker logic
//! - ``perf`` — PerfStats, log parsing, regexes
//! - ``service`` — MonitoringService (TDA facade)

pub mod log_tailer;
pub mod perf;
pub mod service;
pub mod system;

// ---------------------------------------------------------------------------
// Backward-compatible re-exports (mirror old flat monitoring.rs)
// ---------------------------------------------------------------------------

// System metrics
pub use system::{
    bytes_to_gb,
    build_monitoring_text,
    gpu_vram_info,
    parse_tasklist_csv,
    process_ram_bytes,
    ram_usage_bytes,
    SystemMetrics,
};

// Log tailer
pub use log_tailer::{tail_log_chunk, LogCursor};

// Performance stats
pub use perf::{
    feed_perf_stats,
    refresh_perf_uptime,
    reset_perf_stats,
    PerfSnapshot,
    PerfStats,
};

// Service
pub use service::MonitoringService;
