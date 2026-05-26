//! Performance statistics parsed from llama.cpp server logs.
//!
//! Regex-based extraction of prompt/gen throughput, model-load markers,
//! and user prompts.

use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

/// Performance statistics gathered from llama-server log output.
///
/// Observable fields only — cursor state (last_log_size / last_log_marker)
/// lives in ``MonitoringService``, not here.
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
        }
    }
}

/// Read-only snapshot of performance stats for API responses.
#[derive(Debug, Clone)]
pub struct PerfSnapshot {
    pub prompt_tps: Option<f64>,
    pub gen_tps: Option<f64>,
    pub model_loaded: bool,
    pub model_loaded_at: u64,
    pub model_uptime_secs: u64,
    pub last_prompt: String,
}

impl From<&PerfStats> for PerfSnapshot {
    fn from(stats: &PerfStats) -> Self {
        Self {
            prompt_tps: stats.prompt_tps,
            gen_tps: stats.gen_tps,
            model_loaded: stats.model_loaded,
            model_loaded_at: stats.model_loaded_at,
            model_uptime_secs: stats.model_uptime_secs,
            last_prompt: stats.last_prompt.clone(),
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
        };
        reset_perf_stats(&mut stats);
        assert_eq!(stats.prompt_tps, None);
        assert_eq!(stats.gen_tps, None);
        assert!(!stats.model_loaded);
        assert_eq!(stats.model_loaded_at, 0);
        assert_eq!(stats.model_uptime_secs, 0);
        assert!(stats.last_prompt.is_empty());
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
