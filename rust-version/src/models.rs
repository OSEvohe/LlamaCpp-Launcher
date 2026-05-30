use serde::{de::Deserializer, Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static PROFILE_UID_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn new_profile_uid() -> String {
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let counter = PROFILE_UID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("p-{:x}{:x}", now_nanos, counter)
}

/// CLI option descriptor (mirrors legacy ``LlamaOption`` dataclass).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LlamaOption {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub arity: i64,
    #[serde(rename = "default", default)]
    pub default_value: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub positive_flag: String,
    #[serde(default)]
    pub negative_flag: String,
}

// ---------------------------------------------------------------------------
// GlobalSettings — custom Deserialize so missing JSON keys are filled from
// the struct-level Default (identical to legacy GlobalSettings(**data)).
// ---------------------------------------------------------------------------

/// Global launcher settings (mirrors legacy ``GlobalSettings`` dataclass).
#[derive(Serialize, Debug, Clone)]
pub struct GlobalSettings {
    pub llama_server_path: String,
    pub model_dirs: Vec<String>,
    pub api_host: String,
    pub api_port: i64,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            llama_server_path: String::new(),
            model_dirs: Vec::new(),
            api_host: "127.0.0.1".into(),
            api_port: 0,
        }
    }
}

impl<'de> Deserialize<'de> for GlobalSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let map = HashMap::<String, serde_json::Value>::deserialize(deserializer)?;
        let mut gs = GlobalSettings::default();

        if let Some(v) = map.get("llama_server_path").and_then(|v| v.as_str()) {
            gs.llama_server_path = v.to_string();
        }
        if let Some(arr) = map.get("model_dirs").and_then(|v| v.as_array()) {
            gs.model_dirs = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
        if let Some(v) = map.get("api_host").and_then(|v| v.as_str()) {
            gs.api_host = v.to_string();
        }
        if let Some(v) = map.get("api_port").and_then(|v| v.as_i64()) {
            gs.api_port = v;
        }
        Ok(gs)
    }
}

// ---------------------------------------------------------------------------
// Profile — custom Deserialize so missing JSON keys are filled from the
// struct-level Default (identical to legacy Profile(**item)).
// ---------------------------------------------------------------------------

/// Named profile for launching llama-server (mirrors legacy ``Profile`` dataclass).
///
/// Deserialization starts from ``Profile::default()`` and patches only the
/// keys present in the JSON — matching legacy ``Profile(**item)`` behaviour
/// where dataclass defaults fill any absent keys.
#[derive(Serialize, Debug, Clone)]
pub struct Profile {
    pub uid: String,
    pub name: String,
    pub model_path: String,
    pub host: String,
    pub port: i64,
    pub ctx_size: i64,
    pub threads: i64,
    pub n_gpu_layers: i64,
    pub temp: f64,
    pub top_p: f64,
    pub top_k: i64,
    pub min_p: f64,
    pub presence_penalty: f64,
    pub np: i64,
    pub batch_size: i64,
    pub enable_mtp: bool,
    pub spec_draft_n_max: i64,
    pub embeddings: bool,
    pub flash_attn_mode: String,
    pub kv_cache_type: String,
    pub extra_args: String,
    pub advanced_values: HashMap<String, String>,
    pub advanced_modes: HashMap<String, String>,
    pub advanced_favorites: Vec<String>,
    pub start_on_boot: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            uid: new_profile_uid(),
            name: "default".into(),
            model_path: String::new(),
            host: "127.0.0.1".into(),
            port: 8080,
            ctx_size: 4096,
            threads: 8,
            n_gpu_layers: 0,
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
}

impl<'de> Deserialize<'de> for Profile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let map = HashMap::<String, serde_json::Value>::deserialize(deserializer)?;
        let mut p = Profile::default();

        if let Some(v) = map.get("uid").and_then(|v| v.as_str()) {
            p.uid = v.to_string();
        }

        if let Some(v) = map.get("name").and_then(|v| v.as_str()) {
            p.name = v.to_string();
        }
        if let Some(v) = map.get("model_path").and_then(|v| v.as_str()) {
            p.model_path = v.to_string();
        }
        if let Some(v) = map.get("host").and_then(|v| v.as_str()) {
            p.host = v.to_string();
        }
        if let Some(v) = map.get("port").and_then(|v| v.as_i64()) {
            p.port = v;
        }
        if let Some(v) = map.get("ctx_size").and_then(|v| v.as_i64()) {
            p.ctx_size = v;
        }
        if let Some(v) = map.get("threads").and_then(|v| v.as_i64()) {
            p.threads = v;
        }
        if let Some(v) = map.get("n_gpu_layers").and_then(|v| v.as_i64()) {
            p.n_gpu_layers = v;
        }
        if let Some(v) = map.get("temp").and_then(|v| v.as_f64()) {
            p.temp = v;
        }
        if let Some(v) = map.get("top_p").and_then(|v| v.as_f64()) {
            p.top_p = v;
        }
        if let Some(v) = map.get("top_k").and_then(|v| v.as_i64()) {
            p.top_k = v;
        }
        if let Some(v) = map.get("min_p").and_then(|v| v.as_f64()) {
            p.min_p = v;
        }
        if let Some(v) = map.get("presence_penalty").and_then(|v| v.as_f64()) {
            p.presence_penalty = v;
        }
        if let Some(v) = map.get("np").and_then(|v| v.as_i64()) {
            p.np = v;
        }
        if let Some(v) = map.get("batch_size").and_then(|v| v.as_i64()) {
            p.batch_size = v;
        }
        if let Some(v) = map.get("enable_mtp") {
            if let Some(b) = v.as_bool() {
                p.enable_mtp = b;
            } else if let Some(s) = v.as_str() {
                p.enable_mtp = s.trim().to_lowercase() == "true";
            }
        }
        if let Some(v) = map.get("spec_draft_n_max") {
            if let Some(n) = v.as_i64() {
                p.spec_draft_n_max = n;
            } else if let Some(s) = v.as_str() {
                if let Ok(n) = s.parse::<i64>() {
                    p.spec_draft_n_max = n;
                }
            }
        }
        if let Some(v) = map.get("embeddings").and_then(|v| v.as_bool()) {
            p.embeddings = v;
        }
        if let Some(v) = map.get("flash_attn_mode").and_then(|v| v.as_str()) {
            p.flash_attn_mode = v.to_string();
        }
        if let Some(v) = map.get("kv_cache_type").and_then(|v| v.as_str()) {
            p.kv_cache_type = v.to_string();
        }
        if let Some(v) = map.get("extra_args").and_then(|v| v.as_str()) {
            p.extra_args = v.to_string();
        }
        if let Some(obj) = map.get("advanced_values").and_then(|v| v.as_object()) {
            p.advanced_values = obj
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect();
        }
        if let Some(obj) = map.get("advanced_modes").and_then(|v| v.as_object()) {
            p.advanced_modes = obj
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect();
        }
        if let Some(arr) = map.get("advanced_favorites").and_then(|v| v.as_array()) {
            p.advanced_favorites = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
        if let Some(v) = map.get("start_on_boot") {
            if let Some(b) = v.as_bool() {
                p.start_on_boot = b;
            } else if let Some(s) = v.as_str() {
                p.start_on_boot = s.trim().to_lowercase() == "true";
            }
        }
        Ok(p)
    }
}
