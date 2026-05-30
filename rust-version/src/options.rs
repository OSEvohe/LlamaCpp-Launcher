//! Llama-server option discovery helpers mirroring ``llama_launcher/options.py``.
//!
//! Parses ``llama-server --help`` output into structured ``LlamaOption`` entries
//! and resolves the executable path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::models::LlamaOption;

/// Default llama-server path (mirrors legacy ``DEFAULT_LLAMA_SERVER``).
const DEFAULT_LLAMA_SERVER: &str = r"C:\llama-cpp\llama-server.exe";

/// Resolve a raw path string to a ``PathBuf`` pointing at ``llama-server.exe``.
///
/// If *raw* is a directory, appends ``llama-server.exe``.
/// If *raw* is empty after stripping (before quote removal), returns the default path.
///
/// Mirrors legacy behavior: `if not str(raw).strip(): return DEFAULT_LLAMA_SERVER`
/// checks the original trimmed string, NOT the post-quote-stripped version.
/// So ``raw='""'`` yields ``Path("")`` → current dir → ``./llama-server.exe``,
/// NOT the default path.
pub fn resolve_llama_server_path(raw: &str) -> PathBuf {
    // Legacy checks emptiness on the original trimmed string (before quote strip)
    if raw.trim().is_empty() {
        return PathBuf::from(DEFAULT_LLAMA_SERVER);
    }
    let trimmed = raw.trim().trim_matches('"');
    let p = PathBuf::from(trimmed);
    if p.is_dir() {
        let mut result = p;
        result.push("llama-server.exe");
        return result;
    }
    p
}

/// Parse ``llama-server --help`` text into a map of canonical option keys.
///
/// Mirrors legacy ``parse_help_options()``: groups consecutive lines starting
/// with ``-X`` or ``--X`` into blocks, then extracts aliases, arity, defaults
/// and descriptions.
pub fn parse_help_options(help_text: &str) -> HashMap<String, LlamaOption> {
    let lines: Vec<&str> = help_text.lines().collect();

    // Group lines into sections (blocks starting with -X or --X)
    let mut sections: Vec<Vec<String>> = Vec::new();
    let mut current: Option<Vec<String>> = None;

    for line in &lines {
        let trimmed = line.trim();
        // Check if line starts with an option flag
        let is_option_line = if let Some(rest) = trimmed.strip_prefix("--") {
            !rest.starts_with('-') && !rest.starts_with(' ')
        } else if let Some(rest) = trimmed.strip_prefix('-') {
            !rest.starts_with('-') && !rest.is_empty() && !rest.starts_with(' ')
        } else {
            false
        };

        if is_option_line {
            if let Some(block) = current.take() {
                sections.push(block);
            }
            current = Some(vec![line.to_string()]);
        } else if current.is_some() && !trimmed.is_empty() {
            current.as_mut().unwrap().push(line.to_string());
        } else {
            if let Some(block) = current.take() {
                sections.push(block);
            }
        }
    }
    if let Some(block) = current.take() {
        sections.push(block);
    }

    let mut options: HashMap<String, LlamaOption> = HashMap::new();

    for block in &sections {
        let first = block[0].trim();

        // Split first line by 2+ spaces (mirrors legacy re.split(r"\s{2,}", first, maxsplit=1))
        let parts: Vec<&str> = split_by_double_space(first);
        let names_raw = parts[0].trim();
        let mut desc = if parts.len() > 1 {
            parts[1].trim().to_string()
        } else {
            String::new()
        };

        // Append remaining lines to description
        if block.len() > 1 {
            let extra: Vec<String> = block[1..]
                .iter()
                .map(|l| l.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !extra.is_empty() {
                if !desc.is_empty() {
                    desc.push(' ');
                }
                desc.push_str(&extra.join(" "));
            }
        }

        // Normalize whitespace in description
        desc = collapse_whitespace(&desc);

        // Parse alias specs (comma-separated)
        let alias_specs: Vec<&str> = names_raw
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        let mut aliases: Vec<String> = Vec::new();
        let mut arity: i64 = 0;

        for spec in &alias_specs {
            let chunks: Vec<&str> = spec.split_whitespace().collect();
            if chunks.is_empty() {
                continue;
            }
            let flag = chunks[0];
            aliases.push(flag.to_string());
            if chunks.len() > 1 {
                let arg_count = (chunks.len() - 1) as i64;
                if arg_count > arity {
                    arity = arg_count;
                }
            }
        }

        if aliases.is_empty() {
            continue;
        }

        // Determine canonical key
        let long_aliases: Vec<&str> = aliases.iter().filter(|a| a.starts_with("--")).map(|s| s.as_str()).collect();

        // non_no filters out --no- prefixed aliases
        let non_no: Vec<&str> = long_aliases
            .iter()
            .filter(|a| !a.starts_with("--no-"))
            .copied()
            .collect();

        let key = if !non_no.is_empty() {
            non_no[0].to_string()
        } else if !long_aliases.is_empty() {
            long_aliases[0].to_string()
        } else {
            aliases[0].clone()
        };

        let positive = if !non_no.is_empty() {
            non_no[0].to_string()
        } else {
            String::new()
        };

        let mut negative = String::new();
        if !positive.is_empty() {
            let neg_candidate = format!("--no-{}", &positive[2..]);
            if aliases.contains(&neg_candidate) {
                negative = neg_candidate;
            }
        }

        // Extract default value from description: (default: X)
        let default_value = extract_default(&desc);

        options.insert(
            key.clone(),
            LlamaOption {
                key,
                aliases,
                arity,
                default_value,
                description: desc,
                positive_flag: positive,
                negative_flag: negative,
            },
        );
    }

    options
}

/// Run ``exe_path --help`` and parse the output.
///
/// Returns an empty map if the executable cannot be run.
pub fn load_options_from_exe(exe_path: &Path) -> HashMap<String, LlamaOption> {
    let output = match std::process::Command::new(exe_path)
        .arg("--help")
        .output()
    {
        Ok(o) => o,
        Err(_) => return HashMap::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{}\n{}", stdout, stderr);

    parse_help_options(&combined)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Split a string by 2 or more consecutive whitespace characters (max 1 split).
/// Mirrors legacy ``re.split(r"\s{2,}", s, maxsplit=1)``.
fn split_by_double_space(s: &str) -> Vec<&str> {
    let mut i = 0;
    let bytes = s.as_bytes();

    while i < bytes.len() {
        // Check for 2+ whitespace chars
        if bytes[i].is_ascii_whitespace() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let ws_len = i - start;
            if ws_len >= 2 {
                return vec![&s[..start], &s[i..]];
            }
        } else {
            i += 1;
        }
    }

    vec![s]
}

/// Collapse all runs of whitespace into single spaces, trim.
fn collapse_whitespace(s: &str) -> String {
    let mut result = String::new();
    let mut prev_ws = false;

    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_ws && !result.is_empty() {
                result.push(' ');
            }
            prev_ws = true;
        } else {
            result.push(c);
            prev_ws = false;
        }
    }

    result.trim().to_string()
}

/// Extract the default value from a description string.
///
/// Matches legacy ``re.search(r"\(default:\s*([^\)]+)\)", desc)``.
fn extract_default(desc: &str) -> String {
    // Find "(default: X)" pattern
    if let Some(start) = desc.find("(default:") {
        let rest = &desc[start + 10..]; // skip "(default:"
        // Find the closing paren
        if let Some(end) = rest.find(')') {
            let val = rest[..end].trim();
            return val.to_string();
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Acceptance: parse_help_options extracts option keys, aliases, arity, defaults.
    #[test]
    fn test_parse_help_options_basic() {
        let help_text = r#"
Available options:
  --host HOST            server hostname (default: 127.0.0.1)
  --port PORT            port to listen on (default: 8080)
  --threads, -t N        number of threads to use
  --help, -h             print this help
  --embedding            enable embeddings mode
"#;

        let options = parse_help_options(help_text);

        // --host should be parsed
        assert!(options.contains_key("--host"));
        let host_opt = &options["--host"];
        assert_eq!(host_opt.arity, 1);
        assert_eq!(host_opt.default_value, "127.0.0.1");
        assert!(host_opt.description.contains("server hostname"));

        // --port should be parsed
        assert!(options.contains_key("--port"));
        let port_opt = &options["--port"];
        assert_eq!(port_opt.default_value, "8080");

        // --threads with alias -t
        assert!(options.contains_key("--threads"));
        let threads_opt = &options["--threads"];
        assert!(threads_opt.aliases.contains(&"--threads".to_string()));
        assert!(threads_opt.aliases.contains(&"-t".to_string()));
        assert_eq!(threads_opt.arity, 1);

        // --help with alias -h
        assert!(options.contains_key("--help"));
        let help_opt = &options["--help"];
        assert!(help_opt.aliases.contains(&"--help".to_string()));
        assert!(help_opt.aliases.contains(&"-h".to_string()));
    }

    /// Acceptance: negative flags are detected (--no-X when --X exists in same block).
    #[test]
    fn test_parse_help_options_negative_flag() {
        // Legacy detects negative flags within a single comma-separated block
        let help_text = r#"
  --flash-attn, --no-flash-attn   enable/disable flash attention (default: off)
"#;

        let options = parse_help_options(help_text);

        assert!(options.contains_key("--flash-attn"));
        let opt = &options["--flash-attn"];
        assert_eq!(opt.positive_flag, "--flash-attn");
        assert_eq!(opt.negative_flag, "--no-flash-attn");
    }

    /// Acceptance: multi-line descriptions are joined.
    #[test]
    fn test_parse_help_options_multiline() {
        let help_text = r#"
  --model MODEL_FILE     path to model file
                         this is the GGUF model
  --verbose              enable verbose output
"#;

        let options = parse_help_options(help_text);

        assert!(options.contains_key("--model"));
        let model_opt = &options["--model"];
        assert!(model_opt.description.contains("path to model file"));
        assert!(model_opt.description.contains("GGUF model"));
    }

    /// Acceptance: resolve_llama_server_path returns default for empty string.
    #[test]
    fn test_resolve_llama_server_path_empty() {
        let result = resolve_llama_server_path("");
        assert_eq!(result.to_string_lossy(), DEFAULT_LLAMA_SERVER);
    }

    /// Acceptance: resolve_llama_server_path strips quotes from valid path.
    #[test]
    fn test_resolve_llama_server_path_quotes() {
        // Non-empty, non-directory path with quotes
        let result = resolve_llama_server_path(r#""C:\some\path\llama-server.exe""#);
        assert_eq!(result.to_string_lossy(), r"C:\some\path\llama-server.exe");
    }

    /// Acceptance: resolve_llama_server_path whitespace trimming.
    #[test]
    fn test_resolve_llama_server_path_whitespace() {
        let result = resolve_llama_server_path("  ");
        assert_eq!(result.to_string_lossy(), DEFAULT_LLAMA_SERVER);
    }

    /// Acceptance: resolve_llama_server_path with raw='""' does NOT return default.
    ///
    /// Legacy behavior: ``str('""').strip()`` → ``'""'`` (not empty),
    /// so the default is NOT returned. Instead, ``Path('""'.strip('"'))`` →
    /// ``Path('')`` → current directory → ``./llama-server.exe``.
    /// Rust mirrors this: ``raw.trim()`` → ``"\"\""`` (not empty),
    /// so we proceed to ``PathBuf::from("")`` which is the current dir.
    #[test]
    fn test_resolve_llama_server_path_empty_quoted() {
        // raw = '""' — legacy does NOT treat this as empty
        let result = resolve_llama_server_path(r#"""#);
        // Should NOT be the default path
        assert_ne!(result.to_string_lossy(), DEFAULT_LLAMA_SERVER);
        // Should be an empty path (current dir) or just the filename
        // PathBuf::from("") on all platforms resolves to current dir
        assert!(result.to_string_lossy().is_empty() || result == std::path::PathBuf::from(""));
    }

    /// Acceptance: load_options_from_exe returns empty map for nonexistent exe.
    #[test]
    fn test_load_options_from_exe_nonexistent() {
        let path = Path::new("/nonexistent/llama-server.exe");
        let options = load_options_from_exe(path);
        assert!(options.is_empty());
    }

    /// Acceptance: empty help text returns empty map.
    #[test]
    fn test_parse_help_options_empty() {
        let options = parse_help_options("");
        assert!(options.is_empty());
    }

    /// Acceptance: help text with no option lines returns empty map.
    #[test]
    fn test_parse_help_options_no_options() {
        let help_text = r#"
This is just a regular text.
No options here.
Just some description.
"#;
        let options = parse_help_options(help_text);
        assert!(options.is_empty());
    }

    /// Acceptance: default value extraction handles complex defaults.
    #[test]
    fn test_extract_default_complex() {
        let desc = "set the cache type (default: f16)";
        let default_val = extract_default(desc);
        assert_eq!(default_val, "f16");

        let desc2 = "model path (default: /usr/share/models/model.gguf)";
        let default_val2 = extract_default(desc2);
        assert_eq!(default_val2, "/usr/share/models/model.gguf");

        let desc3 = "no default here";
        let default_val3 = extract_default(desc3);
        assert!(default_val3.is_empty());
    }

    /// Acceptance: split_by_double_space mirrors legacy regex behavior.
    #[test]
    fn test_split_by_double_space() {
        assert_eq!(split_by_double_space("--host HOST    description"), vec!["--host HOST", "description"]);
        assert_eq!(split_by_double_space("--host HOST description"), vec!["--host HOST description"]);
        assert_eq!(split_by_double_space("--host HOST"), vec!["--host HOST"]);
        assert_eq!(split_by_double_space("  spaced  out  text"), vec!["", "spaced  out  text"]);
    }

    /// Acceptance: collapse_whitespace normalizes spacing.
    #[test]
    fn test_collapse_whitespace() {
        assert_eq!(collapse_whitespace("hello   world"), "hello world");
        assert_eq!(collapse_whitespace("  trimmed  "), "trimmed");
        assert_eq!(collapse_whitespace("single"), "single");
        assert_eq!(collapse_whitespace("a  b  c  d"), "a b c d");
    }
}
