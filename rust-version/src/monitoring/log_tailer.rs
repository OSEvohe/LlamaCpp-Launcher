//! Log tailing with marker-based rewrite detection.
//!
//! Mirrors Python ``tail_f``: reads new content appended to a log file,
//! detects truncations and equal-size rewrites using a trailing marker.

use std::path::Path;

/// Marker length in bytes for rewrite detection (mirrors Python ``_MARKER_LEN = 64``).
const MARKER_LEN: usize = 64;

/// Cursor state for incremental log tailing.
///
/// Holds the byte offset and trailing marker used to detect
/// file truncations and equal-size rewrites.
#[derive(Debug, Clone)]
pub struct LogCursor {
    /// Last known file size (character count after lossy decode).
    pub last_size: usize,
    /// Tail of the previously-seen log prefix (rewrite detection marker).
    pub last_marker: String,
}

impl Default for LogCursor {
    fn default() -> Self {
        Self {
            last_size: 0,
            last_marker: String::new(),
        }
    }
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
