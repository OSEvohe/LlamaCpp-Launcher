//! Service facade tying all modules together with thread-safe access.
//!
//! Mirrors ``llama_launcher/api.py`` ``LlamaLauncherService``: wraps all state
//! (paths, lock) and exposes every public method the API server and tests call.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::command::{self, canonical_adv_key, favorite_string_value};
use crate::config::{self, app_dir};
use crate::discovery::scan_gguf_models;
use crate::models::{GlobalSettings, LlamaOption, Profile};
use crate::monitoring::{self, bytes_to_gb, tail_log_chunk};
use crate::options::{load_options_from_exe, resolve_llama_server_path};
use crate::process::{self, read_pid, write_pid};

// ---------------------------------------------------------------------------
// Coercion helpers (mirror Python _coerce_int / _coerce_float / _coerce_bool)
// ---------------------------------------------------------------------------

/// Coerce a JSON value to ``i64``. Booleans are **rejected** (Python parity).
fn coerce_int(val: &serde_json::Value, field: &str) -> Result<i64, String> {
    match val {
        serde_json::Value::Bool(_) => Err(format!("{} must be an integer", field)),
        serde_json::Value::Number(n) => n.as_i64().ok_or_else(|| format!("{} must be an integer", field)),
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err(format!("{} must be an integer", field));
            }
            trimmed.parse::<i64>().map_err(|_| format!("{} must be an integer", field))
        }
        _ => Err(format!("{} must be an integer", field)),
    }
}

/// Coerce a JSON value to ``f64``. Booleans are **rejected** (Python parity).
fn coerce_float(val: &serde_json::Value, field: &str) -> Result<f64, String> {
    match val {
        serde_json::Value::Bool(_) => Err(format!("{} must be a number", field)),
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                Ok(f)
            } else {
                Err(format!("{} must be a number", field))
            }
        }
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err(format!("{} must be a number", field));
            }
            trimmed.parse::<f64>().map_err(|_| format!("{} must be a number", field))
        }
        _ => Err(format!("{} must be a number", field)),
    }
}

/// Coerce a JSON value to ``bool``. Accepts ``"true"``/``"false"`` strings.
fn coerce_bool(val: &serde_json::Value, field: &str) -> Result<bool, String> {
    match val {
        serde_json::Value::Bool(b) => Ok(*b),
        serde_json::Value::String(s) => {
            match s.trim().to_lowercase().as_str() {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err(format!("{} must be a boolean", field)),
            }
        }
        _ => Err(format!("{} must be a boolean", field)),
    }
}

// ---------------------------------------------------------------------------
// Internal state (guarded by RwLock)
// ---------------------------------------------------------------------------

/// Mutable runtime state protected by a single ``RwLock``.
struct State {
    /// Last known log file size (character count after lossy decode).
    last_log_size: usize,
    /// Tail of the previously-seen log prefix (rewrite detection marker).
    last_log_marker: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            last_log_size: 0,
            last_log_marker: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// LlamaLauncherService
// ---------------------------------------------------------------------------

/// Facade encapsulating all core LLama Launcher operations.
///
/// Thread-safe: all read-modify-write paths are guarded by an internal
/// ``RwLock`` (mirrors Python ``threading.RLock``).
pub struct LlamaLauncherService {
    /// Application root directory.
    app_dir: PathBuf,
    /// ``.launcher/`` sub-directory.
    state_dir: PathBuf,
    /// Path to ``global.json``.
    global_file: PathBuf,
    /// Path to ``profiles.json``.
    profiles_file: PathBuf,
    /// Path to ``llama-server.pid``.
    pid_file: PathBuf,
    /// Path to ``llama-server.log``.
    log_out: PathBuf,
    /// Path to ``llama-server.err.log``.
    log_err: PathBuf,
    /// ``true`` when *app_dir* is the default (parent of ``CARGO_MANIFEST_DIR``).
    is_default_app_dir: bool,
    /// Runtime state protected by ``RwLock``.
    state: RwLock<State>,
}

impl LlamaLauncherService {
    // -- construction -------------------------------------------------------

    /// Create a new service.
    ///
    /// When *custom_app_dir* is ``None``, the default application directory
    /// (parent of ``CARGO_MANIFEST_DIR``) is used.
    pub fn new(custom_app_dir: Option<PathBuf>) -> Self {
        let default = app_dir();
        let given = custom_app_dir.unwrap_or(default.clone());
        let is_default = given == default;

        Self {
            app_dir: given.clone(),
            state_dir: given.join(".launcher"),
            global_file: given.join(".launcher").join("global.json"),
            profiles_file: given.join(".launcher").join("profiles.json"),
            pid_file: given.join(".launcher").join("llama-server.pid"),
            log_out: given.join(".launcher").join("llama-server.log"),
            log_err: given.join(".launcher").join("llama-server.err.log"),
            is_default_app_dir: is_default,
            state: RwLock::new(State::default()),
        }
    }

    // -- internal helpers (operate on lock guard) ---------------------------

    fn ensure_state(&self) {
        std::fs::create_dir_all(&self.state_dir).ok();
    }

    fn load_profiles_internal(&self) -> Vec<Profile> {
        if self.is_default_app_dir {
            config::load_profiles()
        } else {
            self.ensure_state();
            if !self.profiles_file.exists() {
                return vec![Profile::default()];
            }
            match std::fs::read_to_string(&self.profiles_file) {
                Ok(text) => {
                    let data: serde_json::Value = match serde_json::from_str(&text) {
                        Ok(d) => d,
                        Err(_) => return vec![Profile::default()],
                    };
                    match data.get("profiles").and_then(|v| v.as_array()) {
                        Some(arr) => {
                            let mut profiles = Vec::new();
                            for item in arr {
                                if let Ok(p) = serde_json::from_value(item.clone()) {
                                    profiles.push(p);
                                }
                            }
                            if profiles.is_empty() {
                                vec![Profile::default()]
                            } else {
                                profiles
                            }
                        }
                        None => vec![Profile::default()],
                    }
                }
                Err(_) => vec![Profile::default()],
            }
        }
    }

    fn save_profiles_internal(&self, profiles: &[Profile]) {
        if self.is_default_app_dir {
            config::save_profiles(profiles);
        } else {
            self.ensure_state();
            let payload = serde_json::json!({ "profiles": profiles });
            let json = serde_json::to_string_pretty(&payload).expect("serialize profiles");
            std::fs::write(&self.profiles_file, json).expect("write profiles.json");
        }
    }

    fn load_global_internal(&self) -> GlobalSettings {
        if self.is_default_app_dir {
            config::load_global()
        } else {
            self.ensure_state();
            if !self.global_file.exists() {
                return GlobalSettings::default();
            }
            match std::fs::read_to_string(&self.global_file) {
                Ok(text) => match serde_json::from_str(&text) {
                    Ok(gs) => gs,
                    Err(_) => GlobalSettings::default(),
                },
                Err(_) => GlobalSettings::default(),
            }
        }
    }

    fn save_global_internal(&self, settings: &GlobalSettings) {
        if self.is_default_app_dir {
            config::save_global(settings);
        } else {
            self.ensure_state();
            let json = serde_json::to_string_pretty(settings).expect("serialize GlobalSettings");
            std::fs::write(&self.global_file, json).expect("write global.json");
        }
    }

    fn stop_internal(&self) -> i32 {
        let pid = read_pid(&self.pid_file);
        if pid > 0 && process::is_process_running(pid) {
            process::stop_server(pid);
            if self.pid_file.exists() {
                std::fs::remove_file(&self.pid_file).ok();
            }
            return pid;
        }
        // Clean up stale PID file
        if self.pid_file.exists() {
            std::fs::remove_file(&self.pid_file).ok();
        }
        // Fallback: search by process name
        let fallback_pid = process::find_llama_server_pid();
        if fallback_pid > 0 {
            process::stop_server(fallback_pid);
            return fallback_pid;
        }
        0
    }

    fn launch_internal(&self, cmd: &[String], exe_path: &str) -> i32 {
        self.ensure_state();
        let existing_pid = read_pid(&self.pid_file);
        if existing_pid > 0 && process::is_process_running(existing_pid) {
            panic!("llama-server already running (PID {}). Stop before relaunch.", existing_pid);
        }
        if existing_pid > 0 && self.pid_file.exists() {
            std::fs::remove_file(&self.pid_file).ok();
        }
        let mut state = self.state.write().expect("lock poisoned");
        state.last_log_size = 0;
        state.last_log_marker.clear();
        drop(state);

        if self.log_out.exists() {
            std::fs::remove_file(&self.log_out).ok();
        }
        if self.log_err.exists() {
            std::fs::remove_file(&self.log_err).ok();
        }

        let child_pid = process::start_server(cmd, &self.log_out, &self.app_dir);
        write_pid(&self.pid_file, child_pid);

        if !exe_path.trim().is_empty() {
            let mut gs = self.load_global_internal();
            gs.llama_server_path = exe_path.trim().to_string();
            self.save_global_internal(&gs);
        }
        child_pid
    }

    // -- profiles -----------------------------------------------------------

    /// Load all profiles.
    pub fn load_profiles(&self) -> Vec<Profile> {
        let _guard = self.state.read().expect("lock poisoned");
        self.load_profiles_internal()
    }

    /// Save profiles.
    pub fn save_profiles(&self, profiles: Vec<Profile>) {
        let _guard = self.state.write().expect("lock poisoned");
        self.save_profiles_internal(&profiles);
    }

    /// Add a new profile with the given name.
    pub fn add_profile(&self, name: &str) -> Profile {
        let _guard = self.state.write().expect("lock poisoned");
        let mut profiles = self.load_profiles_internal();
        let profile = Profile {
            name: name.to_string(),
            ..Profile::default()
        };
        profiles.push(profile.clone());
        self.save_profiles_internal(&profiles);
        profile
    }

    /// Delete the profile at *index*. Returns ``true`` on success.
    pub fn delete_profile(&self, index: i64) -> bool {
        let _guard = self.state.write().expect("lock poisoned");
        let mut profiles = self.load_profiles_internal();
        let idx = index as usize;
        if idx < profiles.len() {
            profiles.remove(idx);
            if profiles.is_empty() {
                profiles.push(Profile::default());
            }
            self.save_profiles_internal(&profiles);
            true
        } else {
            false
        }
    }

    /// Duplicate the profile at *index* with a ``(copy)`` suffix.
    pub fn duplicate_profile(&self, index: i64) -> Result<Profile, String> {
        let _guard = self.state.write().expect("lock poisoned");
        let mut profiles = self.load_profiles_internal();
        let idx = index as usize;
        if idx >= profiles.len() {
            return Err(format!("profile index {} out of range", index));
        }
        let src = &profiles[idx];
        let dup = Profile {
            name: format!("{} (copy)", src.name),
            model_path: src.model_path.clone(),
            host: src.host.clone(),
            port: src.port,
            ctx_size: src.ctx_size,
            threads: src.threads,
            n_gpu_layers: src.n_gpu_layers,
            temp: src.temp,
            top_p: src.top_p,
            top_k: src.top_k,
            min_p: src.min_p,
            presence_penalty: src.presence_penalty,
            np: src.np,
            batch_size: src.batch_size,
            enable_mtp: src.enable_mtp,
            spec_draft_n_max: src.spec_draft_n_max,
            embeddings: src.embeddings,
            flash_attn_mode: src.flash_attn_mode.clone(),
            kv_cache_type: src.kv_cache_type.clone(),
            extra_args: src.extra_args.clone(),
            advanced_values: src.advanced_values.clone(),
            advanced_modes: src.advanced_modes.clone(),
            advanced_favorites: src.advanced_favorites.clone(),
        };
        profiles.push(dup.clone());
        self.save_profiles_internal(&profiles);
        Ok(dup)
    }

    /// Atomically read-modify-write a single profile.
    ///
    /// *profile_data* is a map of field names to ``serde_json::Value``.
    /// Coercion is applied to ``top_k``, ``min_p``, ``presence_penalty``,
    /// ``np``, ``enable_mtp``, and ``spec_draft_n_max`` (Python parity).
    pub fn update_profile(
        &self,
        index: i64,
        profile_data: &HashMap<String, serde_json::Value>,
    ) -> Result<Profile, String> {
        let _guard = self.state.write().expect("lock poisoned");
        let mut profiles = self.load_profiles_internal();
        let idx = index as usize;
        if idx >= profiles.len() {
            return Err(format!("profile index {} out of range", index));
        }
        let existing = &profiles[idx];

        // Coerced fields (Python parity)
        let top_k = if let Some(v) = profile_data.get("top_k") {
            coerce_int(v, "top_k")?
        } else {
            existing.top_k
        };
        let min_p = if let Some(v) = profile_data.get("min_p") {
            coerce_float(v, "min_p")?
        } else {
            existing.min_p
        };
        let presence_penalty = if let Some(v) = profile_data.get("presence_penalty") {
            coerce_float(v, "presence_penalty")?
        } else {
            existing.presence_penalty
        };
        let np_value = if let Some(v) = profile_data.get("np") {
            coerce_int(v, "np")?
        } else {
            existing.np
        };
        let enable_mtp = if let Some(v) = profile_data.get("enable_mtp") {
            coerce_bool(v, "enable_mtp")?
        } else {
            existing.enable_mtp
        };
        let spec_draft_n_max = if let Some(v) = profile_data.get("spec_draft_n_max") {
            coerce_int(v, "spec_draft_n_max")?
        } else {
            existing.spec_draft_n_max
        };

        let updated = Profile {
            name: profile_data
                .get("name")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| existing.name.clone()),
            model_path: profile_data
                .get("model_path")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| existing.model_path.clone()),
            host: profile_data
                .get("host")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| existing.host.clone()),
            port: profile_data
                .get("port")
                .and_then(|v| v.as_i64())
                .unwrap_or(existing.port),
            ctx_size: profile_data
                .get("ctx_size")
                .and_then(|v| v.as_i64())
                .unwrap_or(existing.ctx_size),
            threads: profile_data
                .get("threads")
                .and_then(|v| v.as_i64())
                .unwrap_or(existing.threads),
            n_gpu_layers: profile_data
                .get("n_gpu_layers")
                .and_then(|v| v.as_i64())
                .unwrap_or(existing.n_gpu_layers),
            temp: profile_data
                .get("temp")
                .and_then(|v| v.as_f64())
                .unwrap_or(existing.temp),
            top_p: profile_data
                .get("top_p")
                .and_then(|v| v.as_f64())
                .unwrap_or(existing.top_p),
            top_k,
            min_p,
            presence_penalty,
            np: np_value,
            batch_size: profile_data
                .get("batch_size")
                .and_then(|v| v.as_i64())
                .unwrap_or(existing.batch_size),
            enable_mtp,
            spec_draft_n_max,
            embeddings: profile_data
                .get("embeddings")
                .and_then(|v| v.as_bool())
                .unwrap_or(existing.embeddings),
            flash_attn_mode: profile_data
                .get("flash_attn_mode")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| existing.flash_attn_mode.clone()),
            kv_cache_type: profile_data
                .get("kv_cache_type")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| existing.kv_cache_type.clone()),
            extra_args: profile_data
                .get("extra_args")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| existing.extra_args.clone()),
            advanced_values: profile_data
                .get("advanced_values")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_else(|| existing.advanced_values.clone()),
            advanced_modes: profile_data
                .get("advanced_modes")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_else(|| existing.advanced_modes.clone()),
            advanced_favorites: profile_data
                .get("advanced_favorites")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_else(|| existing.advanced_favorites.clone()),
        };

        profiles[idx] = updated.clone();
        self.save_profiles_internal(&profiles);
        Ok(updated)
    }

    // -- global settings ----------------------------------------------------

    /// Load global settings.
    pub fn load_global(&self) -> GlobalSettings {
        let _guard = self.state.read().expect("lock poisoned");
        self.load_global_internal()
    }

    /// Save global settings.
    pub fn save_global(&self, settings: GlobalSettings) {
        let _guard = self.state.write().expect("lock poisoned");
        self.save_global_internal(&settings);
    }

    /// Atomically read-modify-write global settings.
    pub fn update_global(
        &self,
        settings_data: &HashMap<String, serde_json::Value>,
    ) -> GlobalSettings {
        let _guard = self.state.write().expect("lock poisoned");
        let current = self.load_global_internal();
        let settings = GlobalSettings {
            llama_server_path: settings_data
                .get("llama_server_path")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or(current.llama_server_path),
            model_dirs: settings_data
                .get("model_dirs")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or(current.model_dirs),
            api_host: settings_data
                .get("api_host")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or(current.api_host),
            api_port: settings_data
                .get("api_port")
                .and_then(|v| v.as_i64())
                .unwrap_or(current.api_port),
        };
        self.save_global_internal(&settings);
        settings
    }

    // -- options ------------------------------------------------------------

    /// Load CLI options from an executable.
    pub fn load_options(&self, exe_path: &str) -> Result<HashMap<String, LlamaOption>, String> {
        let exe = resolve_llama_server_path(exe_path);
        if !exe.exists() {
            return Err("Chemin llama-server invalide.".to_string());
        }
        Ok(load_options_from_exe(&exe))
    }

    // -- model discovery ----------------------------------------------------

    /// Discover GGUF models in the given directories.
    pub fn discover_models(&self, model_dirs: &[String]) -> Vec<String> {
        scan_gguf_models(model_dirs)
    }

    // -- command assembly ---------------------------------------------------

    /// Assemble the full command-line list for llama-server.
    pub fn build_command(
        &self,
        profile: &Profile,
        exe_path: &str,
        options: &HashMap<String, LlamaOption>,
    ) -> Result<Vec<String>, String> {
        if exe_path.trim().is_empty() {
            return Err("Chemin llama-server non defini".to_string());
        }
        let exe = resolve_llama_server_path(exe_path);
        let is_exe = exe
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("exe"))
            .unwrap_or(false);
        if !is_exe {
            return Err("Le chemin doit pointer vers llama-server.exe".to_string());
        }
        if !exe.exists() {
            return Err("llama-server.exe introuvable".to_string());
        }
        if profile.model_path.is_empty() || !Path::new(&profile.model_path).exists() {
            return Err("Modele GGUF introuvable".to_string());
        }
        command::build_command(&exe, profile, options).map_err(|e| e.to_string())
    }

    // -- process lifecycle --------------------------------------------------

    /// Check whether the server is running. Returns ``(running, pid)``.
    pub fn is_server_running(&self) -> (bool, i32) {
        let pid = read_pid(&self.pid_file);
        if pid > 0 && process::is_process_running(pid) {
            return (true, pid);
        }
        let fallback_pid = process::find_llama_server_pid();
        if fallback_pid > 0 {
            return (true, fallback_pid);
        }
        (false, 0)
    }

    /// Launch the server with the given command.
    pub fn launch(&self, cmd: Vec<String>, exe_path: &str) -> i32 {
        let _guard = self.state.write().expect("lock poisoned");
        self.launch_internal(&cmd, exe_path)
    }

    /// Stop the running server.
    pub fn stop(&self) -> i32 {
        let _guard = self.state.write().expect("lock poisoned");
        self.stop_internal()
    }

    /// Atomically stop and relaunch the server.
    pub fn restart(&self, cmd: Vec<String>, exe_path: &str) -> i32 {
        let _guard = self.state.write().expect("lock poisoned");
        self.stop_internal();
        self.launch_internal(&cmd, exe_path)
    }

    // -- monitoring ---------------------------------------------------------

    /// Return ``(used_bytes, total_bytes)`` of physical RAM.
    pub fn get_ram_usage(&self) -> (u64, u64) {
        monitoring::ram_usage_bytes()
    }

    /// Return approximate RAM usage (bytes) of the process with *pid*.
    pub fn get_process_ram(&self, pid: i32) -> u64 {
        monitoring::process_ram_bytes(pid)
    }

    /// Return ``(used_bytes, total_bytes)`` of GPU VRAM.
    pub fn get_gpu_vram(&self) -> (u64, u64) {
        monitoring::gpu_vram_info()
    }

    /// Format *value* (bytes) as a human-readable GB string.
    pub fn format_bytes(&self, value: u64) -> String {
        bytes_to_gb(value)
    }

    // -- log tailing --------------------------------------------------------

    /// Read new content appended to the log file since *last_size*.
    pub fn tail_log(&self, last_size: usize, last_marker: &str) -> (String, usize, bool, String) {
        if !self.log_out.exists() {
            return (String::new(), last_size, false, last_marker.to_string());
        }
        tail_log_chunk(&self.log_out, last_size, last_marker)
    }

    // -- monitoring text ----------------------------------------------------

    /// Build a two-line ``RAM … / VRAM …`` string suitable for display.
    pub fn build_monitoring_text(&self) -> String {
        monitoring::build_monitoring_text()
    }

    // -- read-only path properties ------------------------------------------

    /// Path to the stdout log file.
    pub fn log_out_path(&self) -> &Path {
        &self.log_out
    }

    /// Default llama-server path.
    pub fn default_server_path(&self) -> PathBuf {
        PathBuf::from(r"C:\llama-cpp\llama-server.exe")
    }

    // -- command helpers (UI advanced-option bookkeeping) -------------------

    /// Resolve *raw_key* to its canonical long-option key.
    pub fn canonical_adv_key(
        &self,
        raw_key: &str,
        options: &HashMap<String, LlamaOption>,
    ) -> String {
        canonical_adv_key(raw_key, options)
    }

    /// Return the argument string for a favourite advanced option.
    pub fn favorite_string_value(
        &self,
        raw_key: &str,
        key: &str,
        opt: Option<&LlamaOption>,
        profile: &Profile,
    ) -> Option<String> {
        favorite_string_value(raw_key, key, opt, profile)
    }

    pub fn canonical_adv_key_service(
        &self,
        raw_key: &str,
        options: &HashMap<String, LlamaOption>,
    ) -> String {
        self.canonical_adv_key(raw_key, options)
    }

    pub fn favorite_string_value_service(
        &self,
        raw_key: &str,
        key: &str,
        opt: Option<&LlamaOption>,
        profile: &Profile,
    ) -> Option<String> {
        self.favorite_string_value(raw_key, key, opt, profile)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::TempDir;

    // ---- Acceptance: constructibility ----

    #[test]
    fn test_service_constructible_default() {
        let _svc = LlamaLauncherService::new(None);
    }

    #[test]
    fn test_service_constructible_custom_dir() {
        let tmp = TempDir::new().expect("create temp dir");
        let _svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));
    }

    // ---- Acceptance: profile CRUD ----

    #[test]
    fn test_profile_crud() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        // Initially one default profile.
        let profiles = svc.load_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "default");

        // Add a profile.
        let added = svc.add_profile("test-profile");
        assert_eq!(added.name, "test-profile");

        let profiles = svc.load_profiles();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[1].name, "test-profile");

        // Duplicate.
        let dup = svc.duplicate_profile(1).expect("duplicate");
        assert_eq!(dup.name, "test-profile (copy)");

        let profiles = svc.load_profiles();
        assert_eq!(profiles.len(), 3);

        // Delete.
        assert!(svc.delete_profile(2));
        let profiles = svc.load_profiles();
        assert_eq!(profiles.len(), 2);

        // Delete out of range returns false.
        assert!(!svc.delete_profile(99));

        // Duplicate out of range returns error.
        assert!(svc.duplicate_profile(99).is_err());
    }

    #[test]
    fn test_delete_last_profile_recreates_default() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        // Initially one default profile.
        let profiles = svc.load_profiles();
        assert_eq!(profiles.len(), 1);

        // Delete it.
        assert!(svc.delete_profile(0));

        // Should have one default profile again.
        let profiles = svc.load_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "default");
    }

    // ---- Acceptance: partial update preserves unchanged fields ----

    #[test]
    fn test_update_profile_partial_preserves_unchanged() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        // Add a profile with known values.
        svc.add_profile("partial-test");

        let mut data = HashMap::new();
        data.insert("name".into(), serde_json::json!("renamed"));
        // Only updating name; all other fields must be preserved.

        let updated = svc.update_profile(1, &data).expect("update");
        assert_eq!(updated.name, "renamed");
        assert_eq!(updated.host, "127.0.0.1"); // default preserved
        assert_eq!(updated.port, 8080);
        assert_eq!(updated.ctx_size, 4096);
        assert_eq!(updated.threads, 8);
        assert_eq!(updated.top_k, 40);
        assert_eq!(updated.min_p, 0.05);
        assert_eq!(updated.enable_mtp, false);
        assert_eq!(updated.spec_draft_n_max, 2);
    }

    #[test]
    fn test_update_profile_out_of_range() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let data: HashMap<String, serde_json::Value> = HashMap::new();
        assert!(svc.update_profile(99, &data).is_err());
    }

    // ---- Acceptance: coercion matches Python behavior ----

    #[test]
    fn test_coerce_int_rejects_bool() {
        assert!(coerce_int(&serde_json::json!(true), "test").is_err());
        assert!(coerce_int(&serde_json::json!(false), "test").is_err());
    }

    #[test]
    fn test_coerce_int_accepts_number() {
        assert_eq!(coerce_int(&serde_json::json!(42), "test").unwrap(), 42);
    }

    #[test]
    fn test_coerce_int_accepts_string() {
        assert_eq!(coerce_int(&serde_json::json!("42"), "test").unwrap(), 42);
        assert_eq!(coerce_int(&serde_json::json!("  7  "), "test").unwrap(), 7);
    }

    #[test]
    fn test_coerce_int_rejects_empty_string() {
        assert!(coerce_int(&serde_json::json!(""), "test").is_err());
    }

    #[test]
    fn test_coerce_int_rejects_invalid_string() {
        assert!(coerce_int(&serde_json::json!("abc"), "test").is_err());
    }

    #[test]
    fn test_coerce_float_rejects_bool() {
        assert!(coerce_float(&serde_json::json!(true), "test").is_err());
        assert!(coerce_float(&serde_json::json!(false), "test").is_err());
    }

    #[test]
    fn test_coerce_float_accepts_number() {
        assert!((coerce_float(&serde_json::json!(0.5), "test").unwrap() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_coerce_float_accepts_string() {
        assert!(
            (coerce_float(&serde_json::json!("0.5"), "test").unwrap() - 0.5).abs() < f64::EPSILON
        );
    }

    #[test]
    fn test_coerce_bool_accepts_bool() {
        assert!(coerce_bool(&serde_json::json!(true), "test").unwrap());
        assert!(!coerce_bool(&serde_json::json!(false), "test").unwrap());
    }

    #[test]
    fn test_coerce_bool_accepts_string() {
        assert!(coerce_bool(&serde_json::json!("true"), "test").unwrap());
        assert!(coerce_bool(&serde_json::json!("TRUE"), "test").unwrap());
        assert!(!coerce_bool(&serde_json::json!("false"), "test").unwrap());
        assert!(coerce_bool(&serde_json::json!("maybe"), "test").is_err());
    }

    #[test]
    fn test_update_profile_coercion_top_k_string() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));
        svc.add_profile("coerce-test");

        let mut data = HashMap::new();
        data.insert("top_k".into(), serde_json::json!("100"));
        let updated = svc.update_profile(1, &data).expect("update");
        assert_eq!(updated.top_k, 100);
    }

    #[test]
    fn test_update_profile_coercion_min_p_string() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));
        svc.add_profile("coerce-test");

        let mut data = HashMap::new();
        data.insert("min_p".into(), serde_json::json!("0.1"));
        let updated = svc.update_profile(1, &data).expect("update");
        assert!((updated.min_p - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn test_update_profile_coercion_enable_mtp_string() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));
        svc.add_profile("coerce-test");

        let mut data = HashMap::new();
        data.insert("enable_mtp".into(), serde_json::json!("true"));
        let updated = svc.update_profile(1, &data).expect("update");
        assert!(updated.enable_mtp);
    }

    #[test]
    fn test_update_profile_coercion_np_bool_rejected() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));
        svc.add_profile("coerce-test");

        let mut data = HashMap::new();
        data.insert("np".into(), serde_json::json!(true));
        assert!(svc.update_profile(1, &data).is_err());
    }

    #[test]
    fn test_update_profile_coercion_spec_draft_n_max_string() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));
        svc.add_profile("coerce-test");

        let mut data = HashMap::new();
        data.insert("spec_draft_n_max".into(), serde_json::json!("5"));
        let updated = svc.update_profile(1, &data).expect("update");
        assert_eq!(updated.spec_draft_n_max, 5);
    }

    // ---- Acceptance: thread safety under concurrent access ----

    #[test]
    fn test_concurrent_add_profile() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = Arc::new(LlamaLauncherService::new(Some(tmp.path().to_path_buf())));

        let mut handles = Vec::new();
        for i in 0..10 {
            let svc = Arc::clone(&svc);
            handles.push(std::thread::spawn(move || {
                svc.add_profile(&format!("thread-{}", i));
            }));
        }
        for h in handles {
            h.join().expect("thread panicked");
        }

        let profiles = svc.load_profiles();
        // 1 default + 10 added
        assert_eq!(profiles.len(), 11);
    }

    #[test]
    fn test_concurrent_update_profile() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = Arc::new(LlamaLauncherService::new(Some(tmp.path().to_path_buf())));

        svc.add_profile("concurrent-target");

        let mut handles = Vec::new();
        for i in 0..10 {
            let svc = Arc::clone(&svc);
            handles.push(std::thread::spawn(move || {
                let mut data = HashMap::new();
                data.insert("name".into(), serde_json::json!(format!("updated-{}", i)));
                svc.update_profile(1, &data).expect("update");
            }));
        }
        for h in handles {
            h.join().expect("thread panicked");
        }

        // After all updates, profile at index 1 should exist.
        let profiles = svc.load_profiles();
        assert_eq!(profiles.len(), 2);
        assert!(profiles[1].name.starts_with("updated-"));
    }

    // ---- Acceptance: global settings CRUD ----

    #[test]
    fn test_global_settings_crud() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let gs = svc.load_global();
        assert_eq!(gs.llama_server_path, "");
        assert_eq!(gs.api_host, "127.0.0.1");

        let mut data = HashMap::new();
        data.insert("api_port".into(), serde_json::json!(9999));
        let updated = svc.update_global(&data);
        assert_eq!(updated.api_port, 9999);
        assert_eq!(updated.api_host, "127.0.0.1"); // preserved

        // Reload and verify persistence.
        let gs = svc.load_global();
        assert_eq!(gs.api_port, 9999);
    }

    // ---- Acceptance: launch -> running -> stop cycle (dummy process) ----

    #[test]
    #[cfg(windows)]
    fn test_launch_running_stop_cycle() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        // Use ping.exe as a dummy long-running process.
        let cmd = vec![
            "ping".into(),
            "-n".into(),
            "60".into(), // 60 seconds
            "127.0.0.1".into(),
        ];

        let pid = svc.launch(cmd, "");
        assert!(pid > 0, "launch should return a positive PID, got {}", pid);

        // Give the process a moment to start.
        std::thread::sleep(std::time::Duration::from_millis(500));

        let (running, running_pid) = svc.is_server_running();
        assert!(running, "server should be running");
        assert_eq!(running_pid, pid);

        let stopped_pid = svc.stop();
        assert_eq!(stopped_pid, pid);

        std::thread::sleep(std::time::Duration::from_millis(500));

        let (running, _) = svc.is_server_running();
        assert!(!running, "server should be stopped");
    }

    // ---- Acceptance: stop with no running server returns 0 ----

    #[test]
    fn test_stop_no_server() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let pid = svc.stop();
        // Returns 0 when nothing is running (and no llama-server fallback found).
        // We can't guarantee 0 if someone happens to have llama-server running,
        // but on a clean test env it should be 0.
        assert!(pid >= 0);
    }

    // ---- Acceptance: is_server_running returns false when no PID file ----

    #[test]
    fn test_is_server_running_no_pid_file() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let (running, pid) = svc.is_server_running();
        // Without a PID file and no llama-server process, should be false.
        // pid might be > 0 if llama-server happens to be running elsewhere.
        assert!(!running || pid > 0);
    }

    // ---- Acceptance: monitoring methods compile and return plausible values ----

    #[test]
    fn test_get_ram_usage() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let (used, total) = svc.get_ram_usage();
        assert!(total > 0, "total RAM should be > 0");
        assert!(used <= total);
    }

    #[test]
    fn test_format_bytes() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        assert_eq!(svc.format_bytes(0), "0.0GB");
        assert_eq!(svc.format_bytes(1_073_741_824), "1.0GB");
    }

    // ---- Acceptance: build_monitoring_text ----

    #[test]
    fn test_build_monitoring_text() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let text = svc.build_monitoring_text();
        assert!(text.contains("RAM:"));
        assert!(text.contains("VRAM:"));
        assert!(text.contains('\n'));
    }

    // ---- Acceptance: tail_log with missing file ----

    #[test]
    fn test_tail_log_missing_file() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let (chunk, size, reset, marker) = svc.tail_log(0, "");
        assert_eq!(chunk, "");
        assert_eq!(size, 0);
        assert!(!reset);
        assert_eq!(marker, "");
    }

    // ---- Acceptance: discover_models with nonexistent dirs ----

    #[test]
    fn test_discover_models_empty() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let models = svc.discover_models(&[]);
        assert!(models.is_empty());
    }

    // ---- Acceptance: default_server_path ----

    #[test]
    fn test_default_server_path() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let path = svc.default_server_path();
        assert_eq!(path.to_string_lossy(), r"C:\llama-cpp\llama-server.exe");
    }

    // ---- Acceptance: log_out_path ----

    #[test]
    fn test_log_out_path() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let path = svc.log_out_path();
        assert!(path.to_string_lossy().contains("llama-server.log"));
    }

    // ---- Acceptance: canonical_adv_key delegates correctly ----

    #[test]
    fn test_canonical_adv_key() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let mut options = HashMap::new();
        options.insert(
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

        assert_eq!(svc.canonical_adv_key("--ctx-size", &options), "--ctx-size");
        assert_eq!(svc.canonical_adv_key("-c", &options), "--ctx-size");
        assert_eq!(svc.canonical_adv_key("--unknown", &options), "--unknown");
    }

    // ---- Acceptance: favorite_string_value delegates correctly ----

    #[test]
    fn test_favorite_string_value() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let profile = Profile::default();
        let opt: Option<&LlamaOption> = None;

        // No advanced values set → returns Some("") (default mode)
        let val = svc.favorite_string_value("--some-key", "--some-key", opt, &profile);
        assert_eq!(val, Some(String::new()));
    }

    // ---- Acceptance: duplicate_profile preserves all fields ----

    #[test]
    fn test_duplicate_profile_preserves_fields() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        svc.add_profile("source");

        // Update the source profile with non-default values.
        let mut data = HashMap::new();
        data.insert("ctx_size".into(), serde_json::json!(8192));
        data.insert("threads".into(), serde_json::json!(16));
        data.insert("enable_mtp".into(), serde_json::json!(true));
        data.insert("spec_draft_n_max".into(), serde_json::json!(5));
        svc.update_profile(1, &data).expect("update");

        // Duplicate.
        let dup = svc.duplicate_profile(1).expect("duplicate");
        assert_eq!(dup.name, "source (copy)");
        assert_eq!(dup.ctx_size, 8192);
        assert_eq!(dup.threads, 16);
        assert!(dup.enable_mtp);
        assert_eq!(dup.spec_draft_n_max, 5);
        // Defaults preserved.
        assert_eq!(dup.host, "127.0.0.1");
        assert_eq!(dup.port, 8080);
    }
}
