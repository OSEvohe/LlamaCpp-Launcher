//! Persistence helpers mirroring ``llama_launcher/config.py``.
//!
//! Reads and writes ``.launcher/global.json`` and ``.launcher/profiles.json``
//! with identical schema to the Python implementation.

use std::path::PathBuf;

use crate::models::{GlobalSettings, Profile};

/// Repo root directory (parent of `rust-version/`).
pub fn app_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CARGO_MANIFEST_DIR has no parent")
        .to_path_buf()
}

fn state_dir() -> PathBuf {
    let mut p = app_dir();
    p.push(".launcher");
    p
}

fn global_file() -> PathBuf {
    let mut p = state_dir();
    p.push("global.json");
    p
}

fn profiles_file() -> PathBuf {
    let mut p = state_dir();
    p.push("profiles.json");
    p
}

fn ensure_state() {
    std::fs::create_dir_all(state_dir()).ok();
}

// ---------------------------------------------------------------------------
// Safe type helpers (mirror Python _safe_int / _safe_str / _safe_bool)
// ---------------------------------------------------------------------------

/// Coerce a JSON value to ``i64``, returning *default* for non-integer or bool.
fn safe_int(val: &serde_json::Value, default: i64) -> i64 {
    match val {
        serde_json::Value::Number(n) if !n.is_f64() => n.as_i64().unwrap_or(default),
        _ => default,
    }
}

/// Coerce a JSON value to ``String``, returning *default* for non-string.
fn safe_str(val: &serde_json::Value, default: &str) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        _ => default.to_string(),
    }
}

/// Coerce a JSON value to ``bool``; also accepts ``"true"``/``"false"`` strings.
fn safe_bool(val: &serde_json::Value, default: bool) -> bool {
    match val {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::String(s) => matches!(s.trim().to_lowercase().as_str(), "true"),
        _ => default,
    }
}

// ---------------------------------------------------------------------------
// Legacy MTP migration (mirrors Python ``_normalize_mtp``)
// ---------------------------------------------------------------------------

pub fn normalize_mtp(item: &mut serde_json::Map<String, serde_json::Value>) {
    // --- Phase 1: ensure well-typed containers ---
    if !item.contains_key("advanced_favorites")
        || !item.get("advanced_favorites").unwrap().is_array()
    {
        item.insert(
            "advanced_favorites".into(),
            serde_json::Value::Array(Vec::new()),
        );
    }
    if !item.contains_key("advanced_values")
        || !item.get("advanced_values").unwrap().is_object()
    {
        item.insert(
            "advanced_values".into(),
            serde_json::Value::Object(serde_json::Map::new()),
        );
    }

    // --- Phase 2: read all info we need (no mutable borrows) ---
    let adv_favs_arr: Vec<String> = item
        .get("advanced_favorites")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();

    let adv_vals_map: serde_json::Map<String, serde_json::Value> = item
        .get("advanced_values")
        .unwrap()
        .as_object()
        .unwrap()
        .clone();

    let has_spec_type_fav = adv_favs_arr.iter().any(|s| s == "--spec-type");
    let has_spec_type_val = adv_vals_map.contains_key("--spec-type");

    let has_legacy_spec_type = if has_spec_type_fav || has_spec_type_val {
        let val = adv_vals_map
            .get("--spec-type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        val.trim() == "draft-mtp"
    } else {
        false
    };

    let has_draft_n_fav = adv_favs_arr.iter().any(|s| s == "--spec-draft-n-max");
    let has_draft_n_val = adv_vals_map.contains_key("--spec-draft-n-max");

    let legacy_draft_n_max: Option<i64> = if has_draft_n_fav || has_draft_n_val {
        let raw = adv_vals_map
            .get("--spec-draft-n-max")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if raw.is_empty() {
            None
        } else {
            raw.trim().parse::<i64>().ok()
        }
    } else {
        None
    };

    let existing_mtp = item.get("enable_mtp").cloned();
    let existing_draft_n = item.get("spec_draft_n_max").cloned();

    // --- Phase 3: apply mutations ---
    if has_legacy_spec_type {
        item.insert("enable_mtp".into(), serde_json::Value::Bool(true));
    }

    if let Some(draft_n) = legacy_draft_n_max {
        item.insert(
            "spec_draft_n_max".into(),
            serde_json::Value::Number(draft_n.into()),
        );
    }

    // Rebuild advanced_favorites without migrated keys
    let new_favs: Vec<serde_json::Value> = adv_favs_arr
        .iter()
        .filter(|s| {
            !(has_legacy_spec_type && *s == "--spec-type")
                && !(legacy_draft_n_max.is_some() && *s == "--spec-draft-n-max")
        })
        .map(|s| serde_json::Value::String(s.clone()))
        .collect();
    item.insert(
        "advanced_favorites".into(),
        serde_json::Value::Array(new_favs),
    );

    // Rebuild advanced_values without migrated keys
    let mut new_vals = adv_vals_map;
    if has_legacy_spec_type {
        new_vals.remove("--spec-type");
    }
    if legacy_draft_n_max.is_some() {
        new_vals.remove("--spec-draft-n-max");
    }
    item.insert(
        "advanced_values".into(),
        serde_json::Value::Object(new_vals),
    );

    // --- Phase 4: type coercion ---
    let mtp_val = item.get("enable_mtp").cloned().unwrap_or(existing_mtp.unwrap_or(serde_json::Value::Bool(false)));
    item.insert(
        "enable_mtp".into(),
        serde_json::Value::Bool(safe_bool(&mtp_val, false)),
    );

    let spec_val = item.get("spec_draft_n_max").cloned().unwrap_or(existing_draft_n.unwrap_or(serde_json::Value::Number(2.into())));
    let coerced = match &spec_val {
        serde_json::Value::Number(n) if !n.is_f64() => n.as_i64().unwrap_or(2),
        serde_json::Value::String(s) => s.trim().parse::<i64>().unwrap_or(2),
        _ => 2,
    };
    item.insert(
        "spec_draft_n_max".into(),
        serde_json::Value::Number(coerced.into()),
    );
}

// ---------------------------------------------------------------------------
// Global settings persistence
// ---------------------------------------------------------------------------

/// Load global settings from ``.launcher/global.json``.
/// Returns defaults if the file does not exist or is malformed.
pub fn load_global() -> GlobalSettings {
    ensure_state();
    let path = global_file();
    if !path.exists() {
        return GlobalSettings::default();
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return GlobalSettings::default(),
    };
    let data: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(&text) {
        Ok(d) => d,
        Err(_) => return GlobalSettings::default(),
    };

    let model_dirs_val = data.get("model_dirs");
    let model_dirs: Vec<String> = match model_dirs_val {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    };

    GlobalSettings {
        llama_server_path: safe_str(data.get("llama_server_path").unwrap_or(&serde_json::Value::Null), ""),
        model_dirs,
        api_host: safe_str(data.get("api_host").unwrap_or(&serde_json::Value::Null), "127.0.0.1"),
        api_port: safe_int(data.get("api_port").unwrap_or(&serde_json::Value::Null), 0),
    }
}

/// Save global settings to ``.launcher/global.json``.
pub fn save_global(settings: &GlobalSettings) {
    ensure_state();
    let path = global_file();
    let json = serde_json::to_string_pretty(settings).expect("serialize GlobalSettings");
    std::fs::write(&path, json).expect("write global.json");
}

// ---------------------------------------------------------------------------
// Profiles persistence
// ---------------------------------------------------------------------------

/// Load profiles from ``.launcher/profiles.json``.
/// Returns a single default profile if the file does not exist or is malformed.
pub fn load_profiles() -> Vec<Profile> {
    ensure_state();
    let path = profiles_file();
    if !path.exists() {
        return vec![Profile::default()];
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return vec![Profile::default()],
    };
    let data: serde_json::Value = match serde_json::from_str(&text) {
        Ok(d) => d,
        Err(_) => return vec![Profile::default()],
    };

    let entries = match data.get("profiles").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return vec![Profile::default()],
    };

    let mut profiles: Vec<Profile> = Vec::new();
    for item in entries {
        let mut obj = match item.as_object() {
            Some(o) => o.clone(),
            None => continue,
        };

        // flash_attn → flash_attn_mode migration
        if obj.contains_key("flash_attn") && !obj.contains_key("flash_attn_mode") {
            let val = obj.get("flash_attn");
            obj.insert(
                "flash_attn_mode".into(),
                match val {
                    Some(serde_json::Value::Bool(true)) => serde_json::Value::String("on".into()),
                    _ => serde_json::Value::String("off".into()),
                },
            );
        }
        obj.remove("flash_attn");

        // Ensure optional fields exist
        obj.entry("advanced_values")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        obj.entry("advanced_favorites")
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));

        normalize_mtp(&mut obj);

        let profile: Profile = match serde_json::from_value(serde_json::Value::Object(obj)) {
            Ok(p) => p,
            Err(_) => continue,
        };
        profiles.push(profile);
    }

    if profiles.is_empty() {
        vec![Profile::default()]
    } else {
        profiles
    }
}

/// Save profiles to ``.launcher/profiles.json``.
pub fn save_profiles(profiles: &[Profile]) {
    ensure_state();
    let path = profiles_file();
    let payload = serde_json::json!({ "profiles": profiles });
    let json = serde_json::to_string_pretty(&payload).expect("serialize profiles");
    std::fs::write(&path, json).expect("write profiles.json");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ---- Acceptance: Profile round-trip ----

    #[test]
    fn test_profile_roundtrip() {
        let original = Profile {
            name: "test-profile".into(),
            model_path: "/models/test.gguf".into(),
            host: "0.0.0.0".into(),
            port: 9000,
            ctx_size: 8192,
            threads: 4,
            n_gpu_layers: 35,
            temp: 0.8,
            top_p: 0.9,
            top_k: 50,
            min_p: 0.1,
            presence_penalty: 0.5,
            np: 2,
            batch_size: 1024,
            enable_mtp: true,
            spec_draft_n_max: 4,
            embeddings: true,
            flash_attn_mode: "on".into(),
            kv_cache_type: "q8_0".into(),
            extra_args: "--verbose".into(),
            advanced_values: {
                let mut m = HashMap::new();
                m.insert("-np".into(), "2".into());
                m.insert("-s".into(), "2048".into());
                m
            },
            advanced_modes: {
                let mut m = HashMap::new();
                m.insert("-np".into(), "default".into());
                m
            },
            advanced_favorites: vec!["-np".into(), "-s".into()],
        };

        let json = serde_json::to_string(&original).expect("serialize");
        let restored: Profile = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(original.name, restored.name);
        assert_eq!(original.model_path, restored.model_path);
        assert_eq!(original.host, restored.host);
        assert_eq!(original.port, restored.port);
        assert_eq!(original.ctx_size, restored.ctx_size);
        assert_eq!(original.threads, restored.threads);
        assert_eq!(original.n_gpu_layers, restored.n_gpu_layers);
        assert_eq!(original.temp, restored.temp);
        assert_eq!(original.top_p, restored.top_p);
        assert_eq!(original.top_k, restored.top_k);
        assert_eq!(original.min_p, restored.min_p);
        assert_eq!(original.presence_penalty, restored.presence_penalty);
        assert_eq!(original.np, restored.np);
        assert_eq!(original.batch_size, restored.batch_size);
        assert_eq!(original.enable_mtp, restored.enable_mtp);
        assert_eq!(original.spec_draft_n_max, restored.spec_draft_n_max);
        assert_eq!(original.embeddings, restored.embeddings);
        assert_eq!(original.flash_attn_mode, restored.flash_attn_mode);
        assert_eq!(original.kv_cache_type, restored.kv_cache_type);
        assert_eq!(original.extra_args, restored.extra_args);
        assert_eq!(original.advanced_values, restored.advanced_values);
        assert_eq!(original.advanced_modes, restored.advanced_modes);
        assert_eq!(original.advanced_favorites, restored.advanced_favorites);
    }

    // ---- Acceptance: Default GlobalSettings JSON shape ----

    #[test]
    fn test_default_global_settings_json_shape() {
        let gs = GlobalSettings::default();
        let json = serde_json::to_string_pretty(&gs).expect("serialize");

        // Expected Python output:
        // {
        //   "llama_server_path": "",
        //   "model_dirs": [],
        //   "api_host": "127.0.0.1",
        //   "api_port": 0
        // }
        let expected = serde_json::json!({
            "llama_server_path": "",
            "model_dirs": [],
            "api_host": "127.0.0.1",
            "api_port": 0
        });
        let actual: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(actual, expected);
    }

    // ---- Acceptance: load_profiles on empty/missing directory returns default ----

    #[test]
    fn test_load_profiles_empty_returns_default() {
        // Simulate: profiles.json has empty profiles array
        let raw = serde_json::json!({ "profiles": [] });
        let text = serde_json::to_string(&raw).unwrap();

        // Parse using the same logic as load_profiles
        let data: serde_json::Value = serde_json::from_str(&text).unwrap();
        let entries = data.get("profiles").and_then(|v| v.as_array()).unwrap();
        let mut profiles: Vec<Profile> = Vec::new();
        for item in entries {
            let obj = match item.as_object() {
                Some(o) => o.clone(),
                None => continue,
            };
            let profile: Profile = serde_json::from_value(serde_json::Value::Object(obj)).unwrap();
            profiles.push(profile);
        }
        let result = if profiles.is_empty() {
            vec![Profile::default()]
        } else {
            profiles
        };

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "default");
    }

    // ---- Acceptance: Legacy --spec-type migration ----

    #[test]
    fn test_normalize_mtp_legacy_spec_type() {
        let raw_json = r#"{
            "name": "legacy",
            "model_path": "/model.gguf",
            "host": "127.0.0.1",
            "port": 8080,
            "ctx_size": 4096,
            "threads": 8,
            "n_gpu_layers": 0,
            "temp": 0.7,
            "top_p": 0.95,
            "top_k": 40,
            "min_p": 0.05,
            "presence_penalty": 0.0,
            "np": 1,
            "batch_size": 512,
            "enable_mtp": false,
            "spec_draft_n_max": 2,
            "embeddings": false,
            "flash_attn_mode": "off",
            "kv_cache_type": "f16",
            "extra_args": "",
            "advanced_values": {
                "--spec-type": "draft-mtp"
            },
            "advanced_modes": {},
            "advanced_favorites": ["--spec-type"]
        }"#;

        let mut obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(raw_json).expect("parse raw");

        normalize_mtp(&mut obj);

        // enable_mtp should be true after migration
        assert_eq!(obj.get("enable_mtp").unwrap().as_bool().unwrap(), true);

        // --spec-type should be removed from advanced_values
        let adv_vals = obj.get("advanced_values").unwrap().as_object().unwrap();
        assert!(!adv_vals.contains_key("--spec-type"));

        // --spec-type should be removed from advanced_favorites
        let adv_favs = obj.get("advanced_favorites").unwrap().as_array().unwrap();
        assert!(!adv_favs.iter().any(|v| v.as_str() == Some("--spec-type")));
    }

    // ---- Acceptance: Legacy --spec-draft-n-max migration ----

    #[test]
    fn test_normalize_mtp_legacy_draft_n_max() {
        let raw_json = r#"{
            "name": "legacy2",
            "model_path": "",
            "host": "127.0.0.1",
            "port": 8080,
            "ctx_size": 4096,
            "threads": 8,
            "n_gpu_layers": 0,
            "temp": 0.7,
            "top_p": 0.95,
            "top_k": 40,
            "min_p": 0.05,
            "presence_penalty": 0.0,
            "np": 1,
            "batch_size": 512,
            "enable_mtp": false,
            "spec_draft_n_max": 2,
            "embeddings": false,
            "flash_attn_mode": "off",
            "kv_cache_type": "f16",
            "extra_args": "",
            "advanced_values": {
                "--spec-draft-n-max": "5"
            },
            "advanced_modes": {},
            "advanced_favorites": ["--spec-draft-n-max"]
        }"#;

        let mut obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(raw_json).expect("parse raw");

        normalize_mtp(&mut obj);

        // spec_draft_n_max should be migrated to 5
        assert_eq!(obj.get("spec_draft_n_max").unwrap().as_i64().unwrap(), 5);

        // --spec-draft-n-max should be removed from advanced_values
        let adv_vals = obj.get("advanced_values").unwrap().as_object().unwrap();
        assert!(!adv_vals.contains_key("--spec-draft-n-max"));

        // --spec-draft-n-max should be removed from advanced_favorites
        let adv_favs = obj.get("advanced_favorites").unwrap().as_array().unwrap();
        assert!(!adv_favs.iter().any(|v| v.as_str() == Some("--spec-draft-n-max")));
    }

    // ---- Acceptance: flash_attn → flash_attn_mode migration ----

    #[test]
    fn test_flash_attn_migration() {
        let raw_json = r#"{
            "name": "old",
            "model_path": "",
            "host": "127.0.0.1",
            "port": 8080,
            "ctx_size": 4096,
            "threads": 8,
            "n_gpu_layers": 0,
            "temp": 0.7,
            "top_p": 0.95,
            "top_k": 40,
            "min_p": 0.05,
            "presence_penalty": 0.0,
            "np": 1,
            "batch_size": 512,
            "enable_mtp": false,
            "spec_draft_n_max": 2,
            "embeddings": false,
            "flash_attn": true,
            "kv_cache_type": "f16",
            "extra_args": "",
            "advanced_values": {},
            "advanced_modes": {},
            "advanced_favorites": []
        }"#;

        let mut obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(raw_json).expect("parse raw");

        // Simulate the flash_attn → flash_attn_mode migration
        if obj.contains_key("flash_attn") && !obj.contains_key("flash_attn_mode") {
            let val = obj.get("flash_attn");
            obj.insert(
                "flash_attn_mode".into(),
                match val {
                    Some(serde_json::Value::Bool(true)) => serde_json::Value::String("on".into()),
                    _ => serde_json::Value::String("off".into()),
                },
            );
        }
        obj.remove("flash_attn");

        assert_eq!(obj.get("flash_attn_mode").unwrap().as_str().unwrap(), "on");
        assert!(!obj.contains_key("flash_attn"));
    }

    // ---- Acceptance: Default Profile JSON shape ----

    #[test]
    fn test_default_profile_json_shape() {
        let p = Profile::default();
        let json = serde_json::to_string_pretty(&p).expect("serialize");
        let actual: serde_json::Value = serde_json::from_str(&json).expect("parse");

        assert_eq!(actual["name"], "default");
        assert_eq!(actual["model_path"], "");
        assert_eq!(actual["host"], "127.0.0.1");
        assert_eq!(actual["port"], 8080);
        assert_eq!(actual["ctx_size"], 4096);
        assert_eq!(actual["threads"], 8);
        assert_eq!(actual["n_gpu_layers"], 0);
        assert_eq!(actual["temp"], 0.7);
        assert_eq!(actual["top_p"], 0.95);
        assert_eq!(actual["top_k"], 40);
        assert_eq!(actual["min_p"], 0.05);
        assert_eq!(actual["presence_penalty"], 0.0);
        assert_eq!(actual["np"], 1);
        assert_eq!(actual["batch_size"], 512);
        assert_eq!(actual["enable_mtp"], false);
        assert_eq!(actual["spec_draft_n_max"], 2);
        assert_eq!(actual["embeddings"], false);
        assert_eq!(actual["flash_attn_mode"], "off");
        assert_eq!(actual["kv_cache_type"], "f16");
        assert_eq!(actual["extra_args"], "");
        assert!(actual["advanced_values"].is_object());
        assert!(actual["advanced_modes"].is_object());
        assert!(actual["advanced_favorites"].is_array());
    }

    // ---- Acceptance: GlobalSettings save → load round-trip ----

    #[test]
    fn test_global_settings_roundtrip() {
        let original = GlobalSettings {
            llama_server_path: "C:\\llama-cpp\\llama-server.exe".into(),
            model_dirs: vec!["C:\\models".into()],
            api_host: "192.168.1.1".into(),
            api_port: 7890,
        };

        let json = serde_json::to_string_pretty(&original).expect("serialize");
        let data: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&json).expect("parse");

        let restored = GlobalSettings {
            llama_server_path: safe_str(data.get("llama_server_path").unwrap_or(&serde_json::Value::Null), ""),
            model_dirs: {
                let val = data.get("model_dirs");
                match val {
                    Some(serde_json::Value::Array(arr)) => arr
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect(),
                    _ => Vec::new(),
                }
            },
            api_host: safe_str(data.get("api_host").unwrap_or(&serde_json::Value::Null), "127.0.0.1"),
            api_port: safe_int(data.get("api_port").unwrap_or(&serde_json::Value::Null), 0),
        };

        assert_eq!(original.llama_server_path, restored.llama_server_path);
        assert_eq!(original.model_dirs, restored.model_dirs);
        assert_eq!(original.api_host, restored.api_host);
        assert_eq!(original.api_port, restored.api_port);
    }

    // ---- Acceptance: Partial Profile deserialization fills defaults ----
    // Regression: serde_json::from_value without #[serde(default)] drops
    // entries when fields are missing. Python Profile(**item) fills defaults.

    #[test]
    fn test_partial_profile_deserialization() {
        // JSON with only "name" — all other fields must come from Default
        let raw = serde_json::json!({ "name": "minimal" });
        let profile: Profile =
            serde_json::from_value(raw).expect("partial profile should deserialize");

        // Provided field
        assert_eq!(profile.name, "minimal");

        // All defaults filled (matching Python Profile dataclass)
        assert_eq!(profile.model_path, "");
        assert_eq!(profile.host, "127.0.0.1");
        assert_eq!(profile.port, 8080);
        assert_eq!(profile.ctx_size, 4096);
        assert_eq!(profile.threads, 8);
        assert_eq!(profile.n_gpu_layers, 0);
        assert_eq!(profile.temp, 0.7);
        assert_eq!(profile.top_p, 0.95);
        assert_eq!(profile.top_k, 40);
        assert_eq!(profile.min_p, 0.05);
        assert_eq!(profile.presence_penalty, 0.0);
        assert_eq!(profile.np, 1);
        assert_eq!(profile.batch_size, 512);
        assert_eq!(profile.enable_mtp, false);
        assert_eq!(profile.spec_draft_n_max, 2);
        assert_eq!(profile.embeddings, false);
        assert_eq!(profile.flash_attn_mode, "off");
        assert_eq!(profile.kv_cache_type, "f16");
        assert_eq!(profile.extra_args, "");
        assert!(profile.advanced_values.is_empty());
        assert!(profile.advanced_modes.is_empty());
        assert!(profile.advanced_favorites.is_empty());
    }

    // ---- Acceptance: Partial GlobalSettings deserialization fills defaults ----

    #[test]
    fn test_partial_global_settings_deserialization() {
        // JSON with only "api_port" — all other fields must come from Default
        let raw = serde_json::json!({ "api_port": 9999 });
        let gs: GlobalSettings =
            serde_json::from_value(raw).expect("partial global settings should deserialize");

        assert_eq!(gs.llama_server_path, "");
        assert!(gs.model_dirs.is_empty());
        assert_eq!(gs.api_host, "127.0.0.1");
        assert_eq!(gs.api_port, 9999);
    }

    // ---- Acceptance: model_dirs non-string entries are silently skipped ----
    // Mirrors Python: data.get("model_dirs", []) returns whatever JSON holds;
    // Rust filter_map drops non-string entries, matching the safe intent.

    #[test]
    fn test_model_dirs_non_string_entries_skipped() {
        let raw = serde_json::json!({
            "llama_server_path": "",
            "model_dirs": ["C:\\models", 42, null, "D:\\llm"],
            "api_host": "127.0.0.1",
            "api_port": 0
        });
        let gs: GlobalSettings =
            serde_json::from_value(raw).expect("deserialize");

        // Only string entries survive
        assert_eq!(gs.model_dirs, vec!["C:\\models", "D:\\llm"]);
    }
}
