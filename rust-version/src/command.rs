//! Command-line assembly helpers mirroring ``llama_launcher/command.py``.
//!
//! Assembles the full argument list for ``llama-server`` from a ``Profile``
//! and the parsed ``LlamaOption`` map.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use crate::models::{LlamaOption, Profile};

/// Error returned by ``shlex_split`` when the input string contains an
/// unmatched opening quote.
///
/// Mirrors legacy ``shlex.split(s, posix=False)`` behavior which raises
/// ``ValueError: No closing quotation``.
#[derive(Debug, PartialEq, Eq)]
pub struct ShlexError {
    /// The quote character that was left unclosed.
    pub quote_char: char,
}

impl fmt::Display for ShlexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "No closing quotation")
    }
}

impl std::error::Error for ShlexError {}

/// Resolve *raw_key* to its canonical long-option key.
///
/// If *raw_key* is already a key in *options* it is returned as-is.
/// Otherwise the first option whose aliases contain *raw_key* is used.
pub fn canonical_adv_key(raw_key: &str, options: &HashMap<String, LlamaOption>) -> String {
    if options.contains_key(raw_key) {
        return raw_key.to_string();
    }
    for (key, opt) in options {
        if opt.aliases.iter().any(|a| a == raw_key) {
            return key.clone();
        }
    }
    raw_key.to_string()
}

/// Return the argument string for a favourite advanced option.
///
/// Checks ``profile.advanced_values`` first (by raw key, then canonical key),
/// then falls back to legacy ``advanced_modes`` with optional negative flag.
///
/// Returns ``None`` when the option should be omitted entirely
/// (legacy ``off`` with no negative alias).
pub fn favorite_string_value(
    raw_key: &str,
    key: &str,
    opt: Option<&LlamaOption>,
    profile: &Profile,
) -> Option<String> {
    // Check advanced_values by raw key first
    if let Some(val) = profile.advanced_values.get(raw_key) {
        return Some(val.clone());
    }
    // Then by canonical key
    if let Some(val) = profile.advanced_values.get(key) {
        return Some(val.clone());
    }

    // Fallback to legacy advanced_modes
    let default_mode = "default".to_string();
    let mode = profile
        .advanced_modes
        .get(raw_key)
        .or_else(|| profile.advanced_modes.get(key))
        .unwrap_or(&default_mode);

    match mode.as_str() {
        "on" => Some(String::new()),
        "off" => {
            if let Some(opt) = opt {
                if !opt.negative_flag.is_empty() {
                    return Some(opt.negative_flag.clone());
                }
            }
            None
        }
        _ => Some(String::new()),
    }
}

/// Assemble the full command-line list for llama-server.
///
/// This is the pure business-logic part: it does NOT interact with the UI.
/// Mirrors legacy ``build_command()`` behavior.
///
/// Returns ``Err(ShlexError)`` if any extra_args or advanced_values contain
/// an unmatched opening quote (matching legacy ``ValueError`` propagation).
pub fn build_command(
    exe: &Path,
    profile: &Profile,
    options: &HashMap<String, LlamaOption>,
) -> Result<Vec<String>, ShlexError> {
    let mut cmd: Vec<String> = Vec::new();

    // Base arguments (fixed order, matching legacy behavior)
    cmd.push(exe.to_string_lossy().to_string());
    cmd.push("--model".into());
    cmd.push(profile.model_path.clone());
    cmd.push("--host".into());
    cmd.push(profile.host.clone());
    cmd.push("--port".into());
    cmd.push(profile.port.to_string());
    cmd.push("--ctx-size".into());
    cmd.push(profile.ctx_size.to_string());
    cmd.push("--threads".into());
    cmd.push(profile.threads.to_string());
    cmd.push("--n-gpu-layers".into());
    cmd.push(profile.n_gpu_layers.to_string());
    cmd.push("--temp".into());
    cmd.push(float_to_py_string(profile.temp));
    cmd.push("--top-p".into());
    cmd.push(float_to_py_string(profile.top_p));
    cmd.push("--top-k".into());
    cmd.push(profile.top_k.to_string());
    cmd.push("--min-p".into());
    cmd.push(float_to_py_string(profile.min_p));
    cmd.push("--presence-penalty".into());
    cmd.push(float_to_py_string(profile.presence_penalty));
    cmd.push("--parallel".into());
    cmd.push(profile.np.to_string());
    cmd.push("--batch-size".into());
    cmd.push(profile.batch_size.to_string());
    cmd.push("--flash-attn".into());
    cmd.push(profile.flash_attn_mode.clone());
    cmd.push("--cache-type-k".into());
    cmd.push(profile.kv_cache_type.clone());
    cmd.push("--cache-type-v".into());
    cmd.push(profile.kv_cache_type.clone());

    // MTP flags
    if profile.enable_mtp {
        cmd.push("--spec-type".into());
        cmd.push("draft-mtp".into());
        cmd.push("--spec-draft-n-max".into());
        cmd.push(profile.spec_draft_n_max.to_string());
    }

    // Embeddings
    if profile.embeddings {
        cmd.push("--embeddings".into());
    }

    // Advanced favorites
    for raw_key in &profile.advanced_favorites {
        let ckey = canonical_adv_key(raw_key, options);

        // MTP dedup: skip MTP-dedicated flags already emitted via enable_mtp
        if profile.enable_mtp && (ckey == "--spec-type" || ckey == "--spec-draft-n-max") {
            continue;
        }

        let opt = options.get(&ckey);
        let val = favorite_string_value(raw_key, &ckey, opt, profile);

        if val.is_none() {
            continue;
        }

        let mut val = val.unwrap();
        val = val.trim().to_string();

        if val.starts_with("--") {
            // Value is a negative flag or similar; split it
            cmd.extend(shlex_split(&val)?);
        } else {
            cmd.push(ckey);
            if !val.is_empty() {
                cmd.extend(shlex_split(&val)?);
            }
        }
    }

    // Extra args
    if !profile.extra_args.is_empty() {
        cmd.extend(shlex_split(&profile.extra_args)?);
    }

    Ok(cmd)
}

// ---------------------------------------------------------------------------
// shlex-like splitting (posix=False, matching legacy behavior)
// ---------------------------------------------------------------------------

/// Split a string using shlex-like rules (posix=False).
///
/// Mirrors legacy ``shlex.split(s, posix=False)`` behavior:
/// - Space/tab/newline/CR are whitespace delimiters.
/// - Both single ``'`` and double ``"`` quotes are grouping characters.
/// - Quotes are PRESERVED in output (not stripped).
/// - Backslash is NEVER an escape character (posix=False).
/// - A quote char only opens a group when it's the first char of a new token.
/// - A quote char closes a group only when it matches the opening type.
/// - When a group closes, the token is completed immediately (pushed).
/// - A quote char appearing mid-token (non-first position) is a regular char.
/// - A different quote type inside a group is a regular char.
/// - Unmatched opening quote returns ``Err(ShlexError)`` (legacy raises ``ValueError``).
fn shlex_split(s: &str) -> Result<Vec<String>, ShlexError> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current: String = String::new();
    // None = not in a group, Some(c) = inside a group opened by c (" or ')
    let mut quote_type: Option<char> = None;

    for c in s.chars() {
        match c {
            '"' | '\'' => {
                current.push(c);
                if let Some(opening) = quote_type {
                    // Inside a group: only matching type closes it
                    if opening == c {
                        quote_type = None;
                        // Closing quote completes the token immediately
                        tokens.push(current.clone());
                        current.clear();
                    }
                    // else: different quote type, treated as regular char
                } else {
                    // Not in a group: only opens if at start of token
                    if current.len() == 1 {
                        // This quote IS the first char of the token
                        quote_type = Some(c);
                    }
                    // else: mid-token quote, treated as regular char
                }
            }
            ' ' | '\t' | '\n' | '\r' => {
                if quote_type.is_some() {
                    current.push(c);
                } else if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(c);
            }
        }
    }

    // Unmatched opening quote: error (legacy raises ValueError)
    if let Some(opening) = quote_type {
        return Err(ShlexError { quote_char: opening });
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    Ok(tokens)
}

/// Format a float to match legacy ``str(float)`` behavior.
///
/// The legacy format always includes a decimal point (e.g., ``str(0.0)`` → ``"0.0"``),
/// while Rust's ``f64::to_string()`` omits it for whole numbers (``0.0`` → ``"0"``).
fn float_to_py_string(f: f64) -> String {
    if f.is_infinite() || f.is_nan() {
        return f.to_string();
    }
    let s = f.to_string();
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{}.0", s)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to build a minimal options map for tests
    fn make_test_options() -> HashMap<String, LlamaOption> {
        let mut opts = HashMap::new();

        opts.insert(
            "--flash-attn".into(),
            LlamaOption {
                key: "--flash-attn".into(),
                aliases: vec!["--flash-attn".into(), "-fa".into()],
                arity: 0,
                default_value: "off".into(),
                description: "flash attention mode".into(),
                positive_flag: "--flash-attn".into(),
                negative_flag: String::new(),
            },
        );

        opts.insert(
            "--spec-type".into(),
            LlamaOption {
                key: "--spec-type".into(),
                aliases: vec!["--spec-type".into()],
                arity: 1,
                default_value: String::new(),
                description: "speculative decoding type".into(),
                positive_flag: String::new(),
                negative_flag: String::new(),
            },
        );

        opts.insert(
            "--spec-draft-n-max".into(),
            LlamaOption {
                key: "--spec-draft-n-max".into(),
                aliases: vec!["--spec-draft-n-max".into()],
                arity: 1,
                default_value: "2".into(),
                description: "max draft tokens".into(),
                positive_flag: String::new(),
                negative_flag: String::new(),
            },
        );

        opts.insert(
            "--verbose".into(),
            LlamaOption {
                key: "--verbose".into(),
                aliases: vec!["--verbose".into(), "-v".into()],
                arity: 0,
                default_value: String::new(),
                description: "verbose output".into(),
                positive_flag: "--verbose".into(),
                negative_flag: "--no-verbose".into(),
            },
        );

        opts.insert(
            "--ctx-size".into(),
            LlamaOption {
                key: "--ctx-size".into(),
                aliases: vec!["--ctx-size".into(), "-c".into()],
                arity: 1,
                default_value: "4096".into(),
                description: "context size".into(),
                positive_flag: String::new(),
                negative_flag: String::new(),
            },
        );

        opts
    }

    fn make_test_profile() -> Profile {
        Profile {
            uid: "test-uid".into(),
            name: "test".into(),
            model_path: "/models/test.gguf".into(),
            host: "127.0.0.1".into(),
            port: 8080,
            ctx_size: 4096,
            threads: 8,
            n_gpu_layers: 35,
            temp: 0.7,
            top_p: 0.95,
            top_k: 40,
            min_p: 0.05,
            presence_penalty: 0.0,
            np: 1,
            batch_size: 512,
            enable_mtp: false,
            spec_draft_n_max: 2,
            embeddings: false,
            flash_attn_mode: "off".into(),
            kv_cache_type: "f16".into(),
            extra_args: String::new(),
            advanced_values: HashMap::new(),
            advanced_modes: HashMap::new(),
            advanced_favorites: Vec::new(),
            start_on_boot: false,
        }
    }

    /// Acceptance: build_command produces the expected legacy argument list.
    #[test]
    fn test_build_command_basic() {
        let exe = Path::new("/usr/bin/llama-server");
        let profile = make_test_profile();
        let options = make_test_options();

        let cmd = build_command(exe, &profile, &options).unwrap();

        // First element is the exe path
        assert_eq!(cmd[0], "/usr/bin/llama-server");

        // Verify the fixed argument order
        assert_eq!(cmd[1], "--model");
        assert_eq!(cmd[2], "/models/test.gguf");
        assert_eq!(cmd[3], "--host");
        assert_eq!(cmd[4], "127.0.0.1");
        assert_eq!(cmd[5], "--port");
        assert_eq!(cmd[6], "8080");
        assert_eq!(cmd[7], "--ctx-size");
        assert_eq!(cmd[8], "4096");
        assert_eq!(cmd[9], "--threads");
        assert_eq!(cmd[10], "8");
        assert_eq!(cmd[11], "--n-gpu-layers");
        assert_eq!(cmd[12], "35");
        assert_eq!(cmd[13], "--temp");
        assert_eq!(cmd[14], "0.7");
        assert_eq!(cmd[15], "--top-p");
        assert_eq!(cmd[16], "0.95");
        assert_eq!(cmd[17], "--top-k");
        assert_eq!(cmd[18], "40");
        assert_eq!(cmd[19], "--min-p");
        assert_eq!(cmd[20], "0.05");
        assert_eq!(cmd[21], "--presence-penalty");
        assert_eq!(cmd[22], "0.0");
        assert_eq!(cmd[23], "--parallel");
        assert_eq!(cmd[24], "1");
        assert_eq!(cmd[25], "--batch-size");
        assert_eq!(cmd[26], "512");
        assert_eq!(cmd[27], "--flash-attn");
        assert_eq!(cmd[28], "off");
        assert_eq!(cmd[29], "--cache-type-k");
        assert_eq!(cmd[30], "f16");
        assert_eq!(cmd[31], "--cache-type-v");
        assert_eq!(cmd[32], "f16");

        // No MTP flags (enable_mtp = false)
        assert!(!cmd.iter().any(|s| s == "--spec-type"));
        assert!(!cmd.iter().any(|s| s == "--spec-draft-n-max"));

        // No embeddings
        assert!(!cmd.iter().any(|s| s == "--embeddings"));

        // Exactly 33 elements (base args, no extras)
        assert_eq!(cmd.len(), 33);
    }

    /// Acceptance: MTP flags appear when enable_mtp=true.
    #[test]
    fn test_build_command_mtp_enabled() {
        let mut profile = make_test_profile();
        profile.enable_mtp = true;
        profile.spec_draft_n_max = 4;

        let exe = Path::new("/usr/bin/llama-server");
        let options = make_test_options();

        let cmd = build_command(exe, &profile, &options).unwrap();

        // Find MTP flags
        let spec_type_idx = cmd.iter().position(|s| s == "--spec-type");
        assert!(spec_type_idx.is_some());
        let idx = spec_type_idx.unwrap();
        assert_eq!(cmd[idx + 1], "draft-mtp");

        let spec_draft_idx = cmd.iter().position(|s| s == "--spec-draft-n-max");
        assert!(spec_draft_idx.is_some());
        let idx = spec_draft_idx.unwrap();
        assert_eq!(cmd[idx + 1], "4");
    }

    /// Acceptance: MTP flags are deduplicated when also in advanced_favorites.
    #[test]
    fn test_build_command_mtp_dedup() {
        let mut profile = make_test_profile();
        profile.enable_mtp = true;
        profile.spec_draft_n_max = 4;

        // Add MTP flags to advanced_favorites (should be deduplicated)
        profile.advanced_favorites = vec![
            "--spec-type".into(),
            "--spec-draft-n-max".into(),
        ];
        let mut av = HashMap::new();
        av.insert("--spec-type".into(), "draft-mtp".into());
        av.insert("--spec-draft-n-max".into(), "4".into());
        profile.advanced_values = av;

        let exe = Path::new("/usr/bin/llama-server");
        let options = make_test_options();

        let cmd = build_command(exe, &profile, &options).unwrap();

        // --spec-type should appear exactly once
        let spec_type_count = cmd.iter().filter(|s| *s == "--spec-type").count();
        assert_eq!(spec_type_count, 1);

        // --spec-draft-n-max should appear exactly once
        let spec_draft_count = cmd.iter().filter(|s| *s == "--spec-draft-n-max").count();
        assert_eq!(spec_draft_count, 1);
    }

    /// Acceptance: MTP flags are omitted when enable_mtp=false, even if in favorites.
    #[test]
    fn test_build_command_mtp_disabled_skipped() {
        let mut profile = make_test_profile();
        profile.enable_mtp = false;

        // Even with MTP in favorites, they should NOT be emitted
        // (because enable_mtp=false means the MTP block is skipped,
        //  and the favorites are only deduplicated when enable_mtp=true)
        // Actually wait — the dedup only skips when enable_mtp is true.
        // When enable_mtp is false, the favorites ARE processed normally.
        // Re-check legacy behavior...
        //
        // Legacy behavior:
        //   if profile.enable_mtp and ckey in ("--spec-type", "--spec-draft-n-max"):
        //       continue
        // So when enable_mtp=false, the check is false and the favorites ARE processed.
        // This means if someone has --spec-type in favorites but enable_mtp=false,
        // it WILL be emitted from the favorites loop.
        //
        // But in practice, the MTP migration in config.py removes these from favorites
        // when enable_mtp is set. So this is an edge case.

        profile.advanced_favorites = vec!["--spec-type".into()];
        let mut av = HashMap::new();
        av.insert("--spec-type".into(), "draft-mtp".into());
        profile.advanced_values = av;

        let exe = Path::new("/usr/bin/llama-server");
        let options = make_test_options();

        let cmd = build_command(exe, &profile, &options).unwrap();

        // --spec-type appears once from favorites (not from MTP block)
        let spec_type_count = cmd.iter().filter(|s| *s == "--spec-type").count();
        assert_eq!(spec_type_count, 1);
    }

    /// Acceptance: embeddings flag is added when enabled.
    #[test]
    fn test_build_command_embeddings() {
        let mut profile = make_test_profile();
        profile.embeddings = true;

        let exe = Path::new("/usr/bin/llama-server");
        let options = make_test_options();

        let cmd = build_command(exe, &profile, &options).unwrap();

        assert!(cmd.contains(&"--embeddings".to_string()));
    }

    /// Acceptance: advanced_favorites with values are appended correctly.
    #[test]
    fn test_build_command_advanced_favorites_with_values() {
        let mut profile = make_test_profile();
        profile.advanced_favorites = vec!["--verbose".into()];
        let mut av = HashMap::new();
        av.insert("--verbose".into(), "true".into());
        profile.advanced_values = av;

        let exe = Path::new("/usr/bin/llama-server");
        let options = make_test_options();

        let cmd = build_command(exe, &profile, &options).unwrap();

        // --verbose should be followed by "true"
        let verbose_idx = cmd.iter().position(|s| s == "--verbose").unwrap();
        assert_eq!(cmd[verbose_idx + 1], "true");
    }

    /// Acceptance: advanced_favorites with negative flag via legacy modes.
    #[test]
    fn test_build_command_legacy_mode_off_negative() {
        let mut profile = make_test_profile();
        profile.advanced_favorites = vec!["--verbose".into()];
        let mut am = HashMap::new();
        am.insert("--verbose".into(), "off".into());
        profile.advanced_modes = am;

        let exe = Path::new("/usr/bin/llama-server");
        let options = make_test_options();

        let cmd = build_command(exe, &profile, &options).unwrap();

        // Should emit --no-verbose (the negative flag)
        assert!(cmd.contains(&"--no-verbose".to_string()));
        // Should NOT emit --verbose
        assert!(!cmd.contains(&"--verbose".to_string()));
    }

    /// Acceptance: legacy mode "off" with no negative flag omits the option.
    #[test]
    fn test_build_command_legacy_mode_off_no_negative() {
        let mut profile = make_test_profile();
        profile.advanced_favorites = vec!["--flash-attn".into()];
        let mut am = HashMap::new();
        am.insert("--flash-attn".into(), "off".into());
        profile.advanced_modes = am;

        let exe = Path::new("/usr/bin/llama-server");
        let options = make_test_options();

        let cmd = build_command(exe, &profile, &options).unwrap();

        // --flash-attn has no negative_flag, so the option should be omitted
        // (only the base --flash-attn from the fixed args should be present)
        // Count occurrences: exactly 1 (from base args)
        let count = cmd.iter().filter(|s| *s == "--flash-attn").count();
        assert_eq!(count, 1);
    }

    /// Acceptance: canonical_adv_key resolves aliases to canonical keys.
    #[test]
    fn test_canonical_adv_key_alias_resolution() {
        let options = make_test_options();

        // Direct key lookup
        assert_eq!(canonical_adv_key("--flash-attn", &options), "--flash-attn");

        // Alias resolution
        assert_eq!(canonical_adv_key("-fa", &options), "--flash-attn");
        assert_eq!(canonical_adv_key("-v", &options), "--verbose");
        assert_eq!(canonical_adv_key("-c", &options), "--ctx-size");

        // Unknown key returned as-is
        assert_eq!(canonical_adv_key("--unknown", &options), "--unknown");
    }

    /// Acceptance: canonical_adv_key with empty options returns key as-is.
    #[test]
    fn test_canonical_adv_key_empty_options() {
        let options: HashMap<String, LlamaOption> = HashMap::new();
        assert_eq!(canonical_adv_key("--anything", &options), "--anything");
    }

    /// Acceptance: extra_args are shlex-split and appended.
    #[test]
    fn test_build_command_extra_args() {
        let mut profile = make_test_profile();
        profile.extra_args = "--some-flag value1 --another-flag".into();

        let exe = Path::new("/usr/bin/llama-server");
        let options = make_test_options();

        let cmd = build_command(exe, &profile, &options).unwrap();

        // Last 3 elements should be the extra args
        assert_eq!(cmd[cmd.len() - 3], "--some-flag");
        assert_eq!(cmd[cmd.len() - 2], "value1");
        assert_eq!(cmd[cmd.len() - 1], "--another-flag");
    }

    /// Acceptance: extra_args with quoted values (quotes preserved in posix=False).
    #[test]
    fn test_build_command_extra_args_quoted() {
        let mut profile = make_test_profile();
        profile.extra_args = r#"--prompt "Hello, World!""#.into();

        let exe = Path::new("/usr/bin/llama-server");
        let options = make_test_options();

        let cmd = build_command(exe, &profile, &options).unwrap();

        // shlex.split(posix=False) preserves quotes in output
        assert_eq!(cmd[cmd.len() - 2], "--prompt");
        assert_eq!(cmd[cmd.len() - 1], r#""Hello, World!""#);
    }

    /// Acceptance: shlex_split handles basic cases (posix=False).
    #[test]
    fn test_shlex_split_basic() {
        assert_eq!(shlex_split("a b c").unwrap(), vec!["a", "b", "c"]);
        assert_eq!(shlex_split("  a  b  ").unwrap(), vec!["a", "b"]);
        assert_eq!(shlex_split("").unwrap(), Vec::<String>::new());
        assert_eq!(shlex_split("single").unwrap(), vec!["single"]);
    }

    /// Acceptance: shlex_split preserves quotes in output (posix=False parity).
    #[test]
    fn test_shlex_split_quoted() {
        // Example: shlex.split('"hello world"', posix=False) => ['"hello world"']
        assert_eq!(shlex_split(r#""hello world""#).unwrap(), vec![r#""hello world""#]);
        // Example: shlex.split('a "b c" d', posix=False) => ['a', '"b c"', 'd']
        assert_eq!(
            shlex_split(r#"a "b c" d"#).unwrap(),
            vec!["a", r#""b c""#, "d"]
        );
        // Empty quotes produce a token
        assert_eq!(shlex_split("\"\"").unwrap(), vec!["\"\""]);
        // Unmatched closing quote is fine (treated as regular char)
        assert_eq!(shlex_split(r#"hello""#).unwrap(), vec!["hello\""]);
        // Empty single quotes produce a token
        assert_eq!(shlex_split("''").unwrap(), vec!["''"]);
    }

    /// Acceptance: shlex_split treats backslash as regular char (posix=False).
    #[test]
    fn test_shlex_split_backslash_not_escape() {
        // Backslash is NEVER an escape in posix=False
        // Quote mid-token (after \) is a regular char, not grouping
        assert_eq!(
            shlex_split(r#"say \"hi\""#).unwrap(),
            vec!["say", r#"\"hi\""#]
        );
        // Lone backslash preserved
        assert_eq!(shlex_split(r#"\"#).unwrap(), vec![r#"\"#]);
        // Windows path with backslashes preserved
        assert_eq!(
            shlex_split(r#"C:\Users\test\file.gguf"#).unwrap(),
            vec![r#"C:\Users\test\file.gguf"#]
        );
        // Backslash before quote: backslash is first char, quote is mid-token → regular
        assert_eq!(
            shlex_split(r#"\\"hello"#).unwrap(),
            vec![r#"\\"hello"#]
        );
    }

    /// Acceptance: shlex_split handles single quotes as grouping chars (posix=False).
    #[test]
    fn test_shlex_split_single_quotes_grouping() {
        // Example: shlex.split("'hello world'", posix=False) => ["'hello world'"]
        assert_eq!(
            shlex_split("'hello world'").unwrap(),
            vec!["'hello world'"]
        );
        // Mixed single and double quotes
        assert_eq!(
            shlex_split(r#"a 'b c' d"#).unwrap(),
            vec!["a", "'b c'", "d"]
        );
        // Single quote in middle of word is a regular char (not at token start)
        assert_eq!(
            shlex_split("it's a test").unwrap(),
            vec!["it's", "a", "test"]
        );
    }

    /// Acceptance: shlex_split whitespace inside double quotes is preserved.
    #[test]
    fn test_shlex_split_whitespace_in_quotes() {
        assert_eq!(
            shlex_split(r#"a "b  c" d"#).unwrap(),
            vec!["a", r#""b  c""#, "d"]
        );
        // Tab inside quotes preserved
        assert_eq!(
            shlex_split("a \"b\tc\" d").unwrap(),
            vec!["a", "\"b\tc\"", "d"]
        );
    }

    /// Acceptance: shlex_split mixed quoted and unquoted tokens.
    #[test]
    fn test_shlex_split_mixed() {
        // Example: shlex.split('a "b c" d "e f"', posix=False)
        assert_eq!(
            shlex_split(r#"a "b c" d "e f""#).unwrap(),
            vec!["a", r#""b c""#, "d", r#""e f""#]
        );
        // Mixed single and double quotes
        assert_eq!(
            shlex_split(r#"a "b" c 'd'"#).unwrap(),
            vec!["a", r#""b""#, "c", "'d'"]
        );
    }

    /// Acceptance: shlex_split quote at non-first position is regular char (parity).
    #[test]
    fn test_shlex_split_mid_token_quote_is_regular() {
        // Example: shlex.split('"a"b"c"', posix=False) => ['"a"', 'b"c"']
        // The " after b is NOT at token start, so it's a regular char
        assert_eq!(
            shlex_split(r#""a"b"c""#).unwrap(),
            vec![r#""a""#, r#"b"c""#]
        );
        // Quote-only token followed by unquoted text
        // Example: shlex.split('"a""b"', posix=False) => ['"a"', '"b"']
        assert_eq!(
            shlex_split(r#""a""b""#).unwrap(),
            vec![r#""a""#, r#""b""#]
        );
    }

    /// Acceptance: shlex_split different quote type inside group is regular char.
    #[test]
    fn test_shlex_split_cross_type_quote_inside_group() {
        // Single quote inside double-quote group is a regular char
        // Example: shlex.split('"it\'s"', posix=False) => ['"it\'s"']
        assert_eq!(
            shlex_split(r#""it's""#).unwrap(),
            vec![r#""it's""#]
        );
        // Double quote inside single-quote group is a regular char
        // Example: shlex.split("'he said \"hi\"'", posix=False) => [''he said "hi"'']
        assert_eq!(
            shlex_split(r#"'he said "hi"'"#).unwrap(),
            vec![r#"'he said "hi"'"#]
        );
    }

    /// Acceptance: shlex_split returns Err on unmatched opening quote (legacy ValueError).
    #[test]
    fn test_shlex_split_unmatched_quote() {
        // Unmatched double quote at start of token
        let result = shlex_split(r#""hello"#);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.quote_char, '"');
        assert_eq!(format!("{}", err), "No closing quotation");

        // Unmatched single quote at start of token
        let result = shlex_split("'hello");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.quote_char, '\'');

        // Unmatched quote after whitespace
        let result = shlex_split("a b \"c");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.quote_char, '"');

        // Matched quotes followed by unmatched quote
        let result = shlex_split(r#""a" "b"#);
        assert!(result.is_err());

        // Unclosed quote with content inside
        let result = shlex_split(r#""hello world"#);
        assert!(result.is_err());

        // Mid-token quote is NOT unmatched (regular char, not grouping)
        // "hello" is matched, "it's" has mid-token quote → OK
        assert!(shlex_split(r#""hello" it's"#).is_ok());
    }

    /// Acceptance: build_command propagates ShlexError from extra_args.
    #[test]
    fn test_build_command_extra_args_unmatched_quote() {
        let mut profile = make_test_profile();
        profile.extra_args = r#"--prompt "Hello"#.into();

        let exe = Path::new("/usr/bin/llama-server");
        let options = make_test_options();

        let result = build_command(exe, &profile, &options);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.quote_char, '"');
    }

    /// Acceptance: favorite_string_value checks raw_key before canonical key.
    #[test]
    fn test_favorite_string_value_raw_key_priority() {
        let options = make_test_options();
        let mut profile = make_test_profile();

        // -fa is an alias of --flash-attn
        let mut av = HashMap::new();
        av.insert("-fa".into(), "raw_value".into());
        av.insert("--flash-attn".into(), "canonical_value".into());
        profile.advanced_values = av;

        let raw_key = "-fa";
        let key = "--flash-attn";
        let opt = options.get(key);

        // Should prefer raw_key value
        let val = favorite_string_value(raw_key, key, opt, &profile);
        assert_eq!(val, Some("raw_value".to_string()));
    }

    /// Acceptance: favorite_string_value falls back to canonical key.
    #[test]
    fn test_favorite_string_value_canonical_key_fallback() {
        let options = make_test_options();
        let mut profile = make_test_profile();

        let mut av = HashMap::new();
        av.insert("--flash-attn".into(), "canonical_value".into());
        profile.advanced_values = av;

        let raw_key = "-fa"; // alias, not in advanced_values
        let key = "--flash-attn";
        let opt = options.get(key);

        let val = favorite_string_value(raw_key, key, opt, &profile);
        assert_eq!(val, Some("canonical_value".to_string()));
    }

    /// Acceptance: favorite_string_value legacy mode "on" returns empty string.
    #[test]
    fn test_favorite_string_value_legacy_on() {
        let options = make_test_options();
        let mut profile = make_test_profile();

        let mut am = HashMap::new();
        am.insert("--flash-attn".into(), "on".into());
        profile.advanced_modes = am;

        let opt = options.get("--flash-attn");
        let val = favorite_string_value("--flash-attn", "--flash-attn", opt, &profile);
        assert_eq!(val, Some(String::new()));
    }

    /// Acceptance: favorite_string_value legacy mode "default" returns empty string.
    #[test]
    fn test_favorite_string_value_legacy_default() {
        let options = make_test_options();
        let profile = make_test_profile();

        let opt = options.get("--flash-attn");
        let val = favorite_string_value("--flash-attn", "--flash-attn", opt, &profile);
        assert_eq!(val, Some(String::new()));
    }

    /// Acceptance: favorite_string_value returns None for off with no negative flag.
    #[test]
    fn test_favorite_string_value_off_no_negative() {
        let options = make_test_options();
        let mut profile = make_test_profile();

        let mut am = HashMap::new();
        am.insert("--flash-attn".into(), "off".into());
        profile.advanced_modes = am;

        // --flash-attn has no negative_flag
        let opt = options.get("--flash-attn");
        let val = favorite_string_value("--flash-attn", "--flash-attn", opt, &profile);
        assert!(val.is_none());
    }

    /// Acceptance: full parity test — complex profile with mixed settings.
    #[test]
    fn test_build_command_complex_profile() {
        let mut profile = make_test_profile();
        profile.enable_mtp = true;
        profile.spec_draft_n_max = 3;
        profile.embeddings = true;
        profile.extra_args = "--log-disable".into();

        // Advanced favorites: mix of direct values, aliases, and legacy modes
        profile.advanced_favorites = vec![
            "-c".into(),          // alias for --ctx-size
            "--verbose".into(),   // with negative flag
            "--spec-type".into(), // MTP dedup test
        ];

        let mut av = HashMap::new();
        av.insert("-c".into(), "8192".into());
        profile.advanced_values = av;

        let mut am = HashMap::new();
        am.insert("--verbose".into(), "off".into()); // legacy mode off → --no-verbose
        profile.advanced_modes = am;

        let exe = Path::new("/usr/bin/llama-server");
        let options = make_test_options();

        let cmd = build_command(exe, &profile, &options).unwrap();

        // MTP flags exactly once
        assert_eq!(cmd.iter().filter(|s| *s == "--spec-type").count(), 1);
        assert_eq!(cmd.iter().filter(|s| *s == "--spec-draft-n-max").count(), 1);

        // MTP value is 3 (from profile, not from favorites)
        let mtp_idx = cmd.iter().position(|s| s == "--spec-draft-n-max").unwrap();
        assert_eq!(cmd[mtp_idx + 1], "3");

        // Embeddings present
        assert!(cmd.contains(&"--embeddings".to_string()));

        // Alias -c resolved to --ctx-size with value 8192
        // Base --ctx-size is at index 7; the alias one comes from advanced_favorites.
        let ctx_positions: Vec<usize> = cmd.iter()
            .enumerate()
            .filter(|(_, s)| *s == "--ctx-size")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(ctx_positions.len(), 2); // base + alias
        assert_eq!(cmd[ctx_positions[1] + 1], "8192");

        // --no-verbose from legacy mode
        assert!(cmd.contains(&"--no-verbose".to_string()));

        // Extra args
        assert!(cmd.contains(&"--log-disable".to_string()));
    }
}
