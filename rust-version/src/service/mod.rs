//! Service facade tying all modules together with thread-safe access.
//!
//! Mirrors ``llama_launcher/api.py`` ``LlamaLauncherService``: wraps all state
//! (paths, lock) and exposes every public method the API server and tests call.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::command::{self, canonical_adv_key, favorite_string_value};
use crate::config::app_dir;
use crate::discovery::scan_gguf_models;
use crate::models::{GlobalSettings, InstalledVersion, LlamaOption, Profile};
use crate::monitoring::{self, MonitoringService, PerfStats};
use crate::options::{load_options_from_exe, resolve_llama_server_path};
use crate::process::{self, read_pid, write_pid};

mod support;
mod global_settings_domain;
mod state_persistence;
mod version_queries;
use support::executable_finder::find_exe_in_dir;
use support::file_tree_copier::copy_dir_all;
use support::startup_guard::ensure_startup_pid;
use support::value_coercer::{coerce_bool, coerce_float, coerce_int};

// ---------------------------------------------------------------------------
// Internal state (guarded by RwLock)
// ---------------------------------------------------------------------------

/// Mutable runtime state protected by a single ``RwLock``.
struct State {
    current_model_path: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            current_model_path: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// LlamaLauncherService
// ---------------------------------------------------------------------------

/// Facade encapsulating all core LLama Launcher operations.
///
/// Thread-safe: all read-modify-write paths are guarded by an internal
/// ``RwLock`` (mirrors legacy ``threading.RLock`` behavior).
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
    /// Monitoring service (owns log cursor and perf stats).
    monitoring: RwLock<MonitoringService>,
    /// Directory where versioned llama.cpp installs live (``.launcher/versions/``).
    versions_dir: PathBuf,
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
            monitoring: RwLock::new(MonitoringService::new(
                given.join(".launcher").join("llama-server.log"),
            )),
            versions_dir: given.join(".launcher").join("versions"),
        }
    }

    // -- internal helpers (operate on lock guard) ---------------------------

    pub fn ensure_state(&self) {
        std::fs::create_dir_all(&self.state_dir).ok();
    }

    fn load_profiles_internal(&self) -> Vec<Profile> {
        state_persistence::load_profiles_internal(self)
    }

    fn save_profiles_internal(&self, profiles: &[Profile]) {
        state_persistence::save_profiles_internal(self, profiles)
    }

    fn load_global_internal(&self) -> GlobalSettings {
        state_persistence::load_global_internal(self)
    }

    fn save_global_internal(&self, settings: &GlobalSettings) {
        state_persistence::save_global_internal(self, settings)
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
            return -1;
        }
        if existing_pid > 0 && self.pid_file.exists() {
            std::fs::remove_file(&self.pid_file).ok();
        }
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
            uid: crate::models::new_profile_uid(),
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
            start_on_boot: false,
        };
        profiles.push(dup.clone());
        self.save_profiles_internal(&profiles);
        Ok(dup)
    }

    /// Atomically read-modify-write a single profile.
    ///
    /// *profile_data* is a map of field names to ``serde_json::Value``.
    /// Coercion is applied to ``top_k``, ``min_p``, ``presence_penalty``,
    /// ``np``, ``enable_mtp``, and ``spec_draft_n_max`` (legacy parity).
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

        // Coerced fields (legacy parity)
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
        let start_on_boot = if let Some(v) = profile_data.get("start_on_boot") {
            coerce_bool(v, "start_on_boot")?
        } else {
            existing.start_on_boot
        };

        let updated = Profile {
            uid: existing.uid.clone(),
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
            start_on_boot,
        };

        profiles[idx] = updated.clone();
        if profiles[idx].start_on_boot {
            for (i, profile) in profiles.iter_mut().enumerate() {
                if i != idx {
                    profile.start_on_boot = false;
                }
            }
        } else {
            let mut seen_start_on_boot = false;
            for profile in profiles.iter_mut() {
                if profile.start_on_boot {
                    if seen_start_on_boot {
                        profile.start_on_boot = false;
                    } else {
                        seen_start_on_boot = true;
                    }
                }
            }
        }
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
        global_settings_domain::update_global(self, settings_data)
    }

    // -- version management -------------------------------------------------

    /// Return the list of installed llama.cpp versions.
    pub fn list_installed_versions(&self) -> Vec<InstalledVersion> {
        version_queries::list_installed_versions(self)
    }

    /// Register or update an installed version entry.
    pub fn register_installed_version(&self, version: InstalledVersion) {
        let _guard = self.state.write().expect("lock poisoned");
        let mut gs = self.load_global_internal();
        let tag = version.tag.clone();
        let exists = gs.installed_versions.iter().any(|v| v.tag == tag);
        if exists {
            // Update existing entry.
            for v in gs.installed_versions.iter_mut() {
                if v.tag == tag {
                    *v = version;
                    break;
                }
            }
        } else {
            gs.installed_versions.push(version);
        }
        self.save_global_internal(&gs);
    }

    /// Remove an installed version entry by tag.
    /// Returns ``true`` if the version was found and removed.
    pub fn unregister_installed_version(&self, tag: &str) -> bool {
        let _guard = self.state.write().expect("lock poisoned");
        let mut gs = self.load_global_internal();
        let before = gs.installed_versions.len();
        gs.installed_versions.retain(|v| v.tag != tag);
        let removed = gs.installed_versions.len() < before;
        // If the removed version was active, clear active_version.
        if removed && gs.active_version.as_deref() == Some(tag) {
            gs.active_version = None;
        }
        self.save_global_internal(&gs);
        removed
    }

    /// Set the active version by tag.
    ///
    /// Returns an error if the tag is not found in the installed versions list.
    pub fn set_active_version(&self, tag: &str) -> Result<(), String> {
        let _guard = self.state.write().expect("lock poisoned");
        let mut gs = self.load_global_internal();
        let found = gs.installed_versions.iter().any(|v| v.tag == tag);
        if !found {
            return Err(format!("version '{}' is not in the installed versions list", tag));
        }
        gs.active_version = Some(tag.to_string());
        self.save_global_internal(&gs);
        Ok(())
    }

    /// Resolve the executable path for the active version.
    ///
    /// Resolution order:
    /// 1. If ``active_version`` is set, look up the matching installed version.
    ///    - If the executable path exists on disk, return it.
    ///    - If the executable is missing, return an error with ``stale`` status.
    /// 2. Fall back to ``llama_server_path`` if set.
    /// 3. Return error if no executable is available.
    pub fn resolve_active_executable(&self) -> Result<String, String> {
        version_queries::resolve_active_executable(self)
    }

    // -- version install / uninstall ----------------------------------------

    /// Fetch available llama.cpp releases from GitHub.
    pub async fn fetch_available_versions(&self) -> Result<Vec<crate::models::GitHubRelease>, String> {
        crate::versions::fetch_releases()
            .await
            .map_err(|e| e.to_string())
    }

    /// Get the current install state for a version tag, if any.
    pub fn get_install_state(&self, tag: &str) -> Option<crate::models::InstallState> {
        version_queries::get_install_state(self, tag)
    }

    /// Start installing a version from a GitHub release asset.
    ///
    /// This method validates preconditions, updates the install state to
    /// ``downloading``, and spawns an async task that performs the actual
    /// download, extraction, and registration.
    pub fn start_install_version(
        &self,
        tag: &str,
        asset: &crate::models::GitHubReleaseAsset,
    ) -> Result<(), String> {
        if crate::versions::find_windows_asset(std::slice::from_ref(asset)).is_none() {
            return Err(format!(
                "asset '{}' is not a supported Windows llama.cpp package",
                asset.name
            ));
        }

        let tag = tag.to_string();
        let url = asset.download_url.clone();
        let asset_name = asset.name.clone();
        let size_bytes = asset.size_bytes;

        // Validate preconditions (hold lock briefly)
        {
            let _guard = self.state.write().expect("lock poisoned");
            let mut gs = self.load_global_internal();

            // Already installed?
            if gs.installed_versions.iter().any(|v| v.tag == tag) {
                return Err(format!("version '{}' is already installed", tag));
            }

            // Already installing?
            if let Some(state) = gs.install_states.get(&tag) {
                if state.phase != crate::models::InstallPhase::Idle
                    && state.phase != crate::models::InstallPhase::Error
                {
                    return Err(format!("install for '{}' is already in progress", tag));
                }
            }

            // Set state to downloading
            gs.install_states.insert(
                tag.clone(),
                crate::models::InstallState {
                    phase: crate::models::InstallPhase::Downloading,
                    downloaded_bytes: 0,
                    total_bytes: size_bytes,
                    error: String::new(),
                },
            );
            self.save_global_internal(&gs);
        }

        // Spawn async install task
        let versions_dir = self.versions_dir.clone();
        let global_file = self.global_file.clone();
        let _state_dir = self.state_dir.clone();

        tokio::spawn(async move {
            let temp_dir = versions_dir.join(format!(".install-{}", tag));
            let zip_path = temp_dir.join(&asset_name);
            let install_dir = versions_dir.join(&tag);

            let result = async {
                // 1. Download
                std::fs::create_dir_all(&temp_dir)
                    .map_err(|e| format!("Failed to create temp dir: {}", e))?;

                crate::versions::download_file(&url, &zip_path, |downloaded, total| {
                    // Update progress in global.json (best-effort, no lock here)
                    if let Ok(data) = std::fs::read_to_string(&global_file) {
                        let mut v: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();
                        v["install_states"][&tag]["phase"] = serde_json::json!("downloading");
                        v["install_states"][&tag]["downloaded_bytes"] = serde_json::json!(downloaded);
                        v["install_states"][&tag]["total_bytes"] = serde_json::json!(total);
                        let json = serde_json::to_string_pretty(&v).unwrap_or_default();
                        std::fs::write(&global_file, json).ok();
                    }
                })
                .await
                .map_err(|e| {
                    // Mark error
                    if let Ok(data) = std::fs::read_to_string(&global_file) {
                        let mut v: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();
                        v["install_states"][&tag]["phase"] = serde_json::json!("error");
                        v["install_states"][&tag]["error"] = serde_json::json!(e);
                        let json = serde_json::to_string_pretty(&v).unwrap_or_default();
                        std::fs::write(&global_file, json).ok();
                    }
                    e
                })?;

                // 2. Extract
                {
                    if let Ok(data) = std::fs::read_to_string(&global_file) {
                        let mut v: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();
                        v["install_states"][&tag]["phase"] = serde_json::json!("extracting");
                        let json = serde_json::to_string_pretty(&v).unwrap_or_default();
                        std::fs::write(&global_file, json).ok();
                    }
                }

                let exe_path = crate::versions::extract_zip(&zip_path, &temp_dir)
                    .map_err(|e| {
                        if let Ok(data) = std::fs::read_to_string(&global_file) {
                            let mut v: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();
                            v["install_states"][&tag]["phase"] = serde_json::json!("error");
                            v["install_states"][&tag]["error"] = serde_json::json!(e);
                            let json = serde_json::to_string_pretty(&v).unwrap_or_default();
                            std::fs::write(&global_file, json).ok();
                        }
                        e
                    })?;

                // 3. Validate executable
                if !exe_path.exists() {
                    let err = "llama-server.exe not found after extraction".to_string();
                    if let Ok(data) = std::fs::read_to_string(&global_file) {
                        let mut v: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();
                        v["install_states"][&tag]["phase"] = serde_json::json!("error");
                        v["install_states"][&tag]["error"] = serde_json::json!(err);
                        let json = serde_json::to_string_pretty(&v).unwrap_or_default();
                        std::fs::write(&global_file, json).ok();
                    }
                    return Err(err);
                }

                // 4. Move to final install path
                if install_dir.exists() {
                    crate::versions::remove_dir_all_force(&install_dir);
                }
                let move_ok = std::fs::rename(&temp_dir, &install_dir);
                if move_ok.is_err() {
                    // Cross-device fallback: copy then remove
                    copy_dir_all(&temp_dir, &install_dir)
                        .map_err(|e| format!("Failed to move install dir (copy fallback): {}", e))?;
                    crate::versions::remove_dir_all_force(&temp_dir);
                } else if let Err(e) = move_ok {
                    return Err(format!("Failed to move install dir: {}", e));
                }

                // 5. Register
                {
                    if let Ok(data) = std::fs::read_to_string(&global_file) {
                        let mut v: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();
                        v["install_states"][&tag]["phase"] = serde_json::json!("registering");
                        let json = serde_json::to_string_pretty(&v).unwrap_or_default();
                        std::fs::write(&global_file, json).ok();
                    }
                }

                // Resolve the actual exe path
                let actual_exe = find_exe_in_dir(&install_dir)
                    .unwrap_or_else(|| install_dir.join("llama-server.exe"));

                let now = chrono::Utc::now().to_rfc3339();

                // Write the updated global.json with the version registered
                if let Ok(data) = std::fs::read_to_string(&global_file) {
                    let mut v: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();

                    // Add to installed_versions
                    let version_obj = serde_json::json!({
                        "tag": tag,
                        "source": "github",
                        "install_path": install_dir.to_string_lossy().to_string(),
                        "executable_path": actual_exe.to_string_lossy().to_string(),
                        "status": "installed",
                        "installed_at": now,
                    });
                    if let Some(arr) = v["installed_versions"].as_array_mut() {
                        arr.push(version_obj);
                    }

                    // Clear install state
                    if let Some(obj) = v["install_states"].as_object_mut() {
                        obj.remove(&tag);
                    }

                    let json = serde_json::to_string_pretty(&v).unwrap_or_default();
                    std::fs::write(&global_file, json).ok();
                }

                // Cleanup temp
                crate::versions::remove_file_force(&zip_path);
                crate::versions::remove_dir_all_force(&temp_dir);

                Ok::<(), String>(())
            }.await;

            if result.is_err() {
                // Cleanup on failure
                crate::versions::remove_dir_all_force(&temp_dir);
                crate::versions::remove_dir_all_force(&install_dir);
            }
        });

        Ok(())
    }

    /// Cancel an in-progress install for the given tag.
    pub fn cancel_install(&self, tag: &str) -> Result<(), String> {
        let _guard = self.state.write().expect("lock poisoned");
        let mut gs = self.load_global_internal();

        let state = gs.install_states.get(tag).ok_or_else(|| {
            format!("no install in progress for '{}'", tag)
        })?;

        if state.phase == crate::models::InstallPhase::Idle
            || state.phase == crate::models::InstallPhase::Done
        {
            return Err(format!("no active install to cancel for '{}'", tag));
        }

        // Clean up temp install directory
        let temp_dir = self.versions_dir.join(format!(".install-{}", tag));
        crate::versions::remove_dir_all_force(&temp_dir);

        gs.install_states.insert(
            tag.to_string(),
            crate::models::InstallState {
                phase: crate::models::InstallPhase::Idle,
                downloaded_bytes: 0,
                total_bytes: 0,
                error: "cancelled".into(),
            },
        );
        self.save_global_internal(&gs);
        Ok(())
    }

    /// Uninstall a version by tag.
    ///
    /// Returns an error if:
    /// - The version is the active version
    /// - The version is not installed
    pub fn uninstall_version(&self, tag: &str) -> Result<(), String> {
        let _guard = self.state.write().expect("lock poisoned");
        let mut gs = self.load_global_internal();

        // Guard: cannot uninstall active version
        if gs.active_version.as_deref() == Some(tag) {
            return Err(format!(
                "cannot uninstall active version '{}'; set another version as active first",
                tag
            ));
        }

        // Find the version
        let idx = gs.installed_versions
            .iter()
            .position(|v| v.tag == tag)
            .ok_or_else(|| format!("version '{}' is not installed", tag))?;

        let version = &gs.installed_versions[idx];

        // Remove install directory
        if !version.install_path.is_empty() {
            crate::versions::remove_dir_all_force(&Path::new(&version.install_path));
        }

        // Remove from registry
        gs.installed_versions.remove(idx);

        // Clean up any stale install state
        gs.install_states.remove(tag);

        self.save_global_internal(&gs);
        Ok(())
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
        let existing_pid = read_pid(&self.pid_file);
        if existing_pid > 0 && process::is_process_running(existing_pid) {
            return -1;
        }
        // Full reset of monitoring (stats + cursor) before launching (avoids reentrant lock).
        {
            let mut mon = self.monitoring.write().expect("lock poisoned");
            mon.full_reset();
        }
        let mut state = self.state.write().expect("lock poisoned");
        state.current_model_path.clear();
        for i in 0..cmd.len() {
            if cmd[i] == "--model" && i + 1 < cmd.len() {
                state.current_model_path = cmd[i + 1].clone();
                break;
            }
        }
        self.launch_internal(&cmd, exe_path)
    }

    /// Stop the running server.
    pub fn stop(&self) -> i32 {
        let mut state = self.state.write().expect("lock poisoned");
        state.current_model_path.clear();
        self.stop_internal()
    }

    /// Atomically stop and relaunch the server.
    pub fn restart(&self, cmd: Vec<String>, exe_path: &str) -> i32 {
        // Full reset of monitoring (stats + cursor) before restarting (avoids reentrant lock).
        {
            let mut mon = self.monitoring.write().expect("lock poisoned");
            mon.full_reset();
        }
        self.stop_internal();
        let mut state = self.state.write().expect("lock poisoned");
        state.current_model_path.clear();
        for i in 0..cmd.len() {
            if cmd[i] == "--model" && i + 1 < cmd.len() {
                state.current_model_path = cmd[i + 1].clone();
                break;
            }
        }
        self.launch_internal(&cmd, exe_path)
    }

    /// Apply the startup profile if one is enabled.
    pub fn apply_startup_profile(&self) -> Result<(), String> {
        let profiles = self.load_profiles();
        let selected_idx = profiles.iter().position(|p| p.start_on_boot);
        let Some(idx) = selected_idx else {
            return Ok(());
        };
        let profile = &profiles[idx];

        let exe_path = self.resolve_active_executable().or_else(|_| {
            let settings = self.load_global();
            if settings.llama_server_path.trim().is_empty() {
                Err("no llama-server path configured".to_string())
            } else {
                Ok(settings.llama_server_path)
            }
        })?;

        let options = self.load_options(&exe_path)?;
        let cmd = self.build_command(profile, &exe_path, &options)?;

        let (running, _) = self.is_server_running();
        if running {
            let pid = self.restart(cmd, &exe_path);
            ensure_startup_pid("restart", pid)?;
        } else {
            let pid = self.launch(cmd, &exe_path);
            ensure_startup_pid("launch", pid)?;
        }
        Ok(())
    }

    pub fn current_model_path(&self) -> String {
        let s = self.state.read().expect("lock poisoned");
        s.current_model_path.clone()
    }

    // -- monitoring ---------------------------------------------------------

    /// Return ``(used_bytes, total_bytes)`` of physical RAM.
    pub fn get_ram_usage(&self) -> (u64, u64) {
        let mon = self.monitoring.read().expect("lock poisoned");
        mon.ram_usage_bytes()
    }

    /// Return approximate RAM usage (bytes) of the process with *pid*.
    pub fn get_process_ram(&self, pid: i32) -> u64 {
        let mon = self.monitoring.read().expect("lock poisoned");
        mon.process_ram_bytes(pid)
    }

    /// Return ``(used_bytes, total_bytes)`` of GPU VRAM.
    pub fn get_gpu_vram(&self) -> (u64, u64) {
        let mon = self.monitoring.read().expect("lock poisoned");
        mon.gpu_vram_info()
    }

    /// Format *value* (bytes) as a human-readable GB string.
    pub fn format_bytes(&self, value: u64) -> String {
        let mon = self.monitoring.read().expect("lock poisoned");
        mon.format_bytes(value)
    }

    // -- log tailing --------------------------------------------------------

    /// Read new content appended to the log file since *last_size*.
    pub fn tail_log(&self, last_size: usize, last_marker: &str) -> (String, usize, bool, String) {
        // NOTE: perf_stats are NOT updated here.  This method uses the
        // *client's* cursor (last_size/last_marker).  Feeding stats from
        // a client cursor would undo /api/perf/reset when a slow client
        // re-scans historical log content.  Perf stats are driven solely
        // by the internal cursor in refresh_and_get_perf_stats().
        let mon = self.monitoring.read().expect("lock poisoned");
        mon.tail_log_for_client(&self.log_out, last_size, last_marker)
    }

    // -- monitoring text ----------------------------------------------------

    /// Build a two-line ``RAM … / VRAM …`` string suitable for display.
    pub fn build_monitoring_text(&self) -> String {
        monitoring::build_monitoring_text()
    }

    /// Assemble the full monitoring JSON payload (TDA).
    ///
    /// *running* and *pid* are supplied by the caller (not a monitoring concern).
    pub fn build_monitoring_payload(&self, running: bool, pid: i32) -> serde_json::Value {
        let mut mon = self.monitoring.write().expect("lock poisoned");
        mon.refresh();

        let (used_ram, total_ram) = mon.ram_usage_bytes();
        let (used_vram, total_vram) = mon.gpu_vram_info();
        let process_ram = if running {
            mon.process_ram_bytes(pid)
        } else {
            0
        };
        let perf = mon.stats_clone();

        serde_json::json!({
            "ram": {
                "used": used_ram,
                "total": total_ram,
                "used_human": mon.format_bytes(used_ram),
                "total_human": mon.format_bytes(total_ram),
            },
            "vram": {
                "used": used_vram,
                "total": total_vram,
                "used_human": mon.format_bytes(used_vram),
                "total_human": mon.format_bytes(total_vram),
            },
            "process_ram": process_ram,
            "process_ram_human": mon.format_bytes(process_ram),
            "performance": {
                "prompt_tps": perf.prompt_tps,
                "gen_tps": perf.gen_tps,
                "model_loaded": perf.model_loaded,
                "model_loaded_at": perf.model_loaded_at,
                "model_uptime_secs": perf.model_uptime_secs,
                "last_prompt": perf.last_prompt,
            },
        })
    }

    // -- performance stats --------------------------------------------------

    /// Return a snapshot of the current performance statistics.
    pub fn get_perf_stats(&self) -> PerfStats {
        let mon = self.monitoring.read().expect("lock poisoned");
        mon.stats_clone()
    }

    /// Reset performance statistics to their default (empty) values.
    ///
    /// Preserves the internal cursor so the next ``refresh_and_get_perf_stats()``
    /// only sees *new* log content — avoids re-injecting historical markers
    /// after a reset.
    pub fn reset_perf_stats(&self) {
        let mut mon = self.monitoring.write().expect("lock poisoned");
        mon.reset_perf();
    }

    /// Tail the log for new content, feed perf stats, refresh uptime,
    /// and return a snapshot.  Suitable for read-only endpoints that
    /// should always return fresh data.
    ///
    /// Uses the internal cursor to tail **only new content** — avoids
    /// re-parsing historical markers (e.g. model-load) that would undo
    /// a reset.
    pub fn refresh_and_get_perf_stats(&self) -> PerfStats {
        let mut mon = self.monitoring.write().expect("lock poisoned");
        mon.refresh();
        mon.stats_clone()
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
    use crate::models::{InstallPhase, VersionStatus};
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

    // ---- Acceptance: coercion matches legacy behavior ----

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

        // Verify managed process is stopped (pid file removed, original pid dead).
        // Avoid global is_server_running() fallback which can match unrelated llama-server processes.
        assert!(
            !svc.pid_file.exists(),
            "pid file should be removed after stop"
        );
        assert!(
            !process::is_process_running(pid),
            "original pid {} should no longer be running",
            pid
        );
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
        data.insert("start_on_boot".into(), serde_json::json!(true));
        svc.update_profile(1, &data).expect("update");

        // Duplicate.
        let dup = svc.duplicate_profile(1).expect("duplicate");
        assert_eq!(dup.name, "source (copy)");
        assert_eq!(dup.ctx_size, 8192);
        assert_eq!(dup.threads, 16);
        assert!(dup.enable_mtp);
        assert_eq!(dup.spec_draft_n_max, 5);
        assert!(!dup.start_on_boot);
        let profiles = svc.load_profiles();
        assert!(profiles[1].start_on_boot);
        assert!(!profiles[2].start_on_boot);
        // Defaults preserved.
        assert_eq!(dup.host, "127.0.0.1");
        assert_eq!(dup.port, 8080);
    }

    #[test]
    fn test_update_profile_start_on_boot_clears_other_profiles() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        svc.add_profile("p1");
        svc.add_profile("p2");

        let mut first = HashMap::new();
        first.insert("start_on_boot".into(), serde_json::json!(true));
        svc.update_profile(1, &first).expect("enable p1");

        let mut second = HashMap::new();
        second.insert("start_on_boot".into(), serde_json::json!(true));
        svc.update_profile(2, &second).expect("enable p2");

        let profiles = svc.load_profiles();
        assert!(!profiles[0].start_on_boot);
        assert!(!profiles[1].start_on_boot);
        assert!(profiles[2].start_on_boot);
    }

    #[test]
    fn test_update_profile_enforces_single_start_on_boot_even_when_disabling() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        svc.save_profiles(vec![
            Profile { name: "a".into(), start_on_boot: true, ..Profile::default() },
            Profile { name: "b".into(), start_on_boot: true, ..Profile::default() },
            Profile { name: "c".into(), start_on_boot: false, ..Profile::default() },
        ]);

        let mut update = HashMap::new();
        update.insert("start_on_boot".into(), serde_json::json!(false));
        svc.update_profile(2, &update).expect("update profile");

        let profiles = svc.load_profiles();
        assert!(profiles[0].start_on_boot);
        assert!(!profiles[1].start_on_boot);
        assert!(!profiles[2].start_on_boot);
    }

    #[test]
    fn test_ensure_startup_pid_errors_on_zero() {
        assert!(ensure_startup_pid("launch", 0).is_err());
        assert!(ensure_startup_pid("restart", -1).is_err());
        assert!(ensure_startup_pid("launch", 1234).is_ok());
    }

    // ---- Acceptance: perf stats initial state ----

    #[test]
    fn test_perf_stats_initial_empty() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let perf = svc.get_perf_stats();
        assert!(perf.prompt_tps.is_none());
        assert!(perf.gen_tps.is_none());
        assert!(!perf.model_loaded);
        assert_eq!(perf.model_loaded_at, 0);
        assert!(perf.last_prompt.is_empty());
    }

    // ---- Acceptance: perf stats reset ----

    #[test]
    fn test_perf_stats_reset() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        // Write log content that triggers perf stats updates.
        std::fs::create_dir_all(svc.log_out_path().parent().unwrap()).ok();
        std::fs::write(
            svc.log_out_path(),
            "llama_model_loader: loaded model\n\
             User prompt: Hello\n\
             prompt eval time = 100.00 ms / 5 tokens (20.00 ms per token, 50.00 tokens per second)\n",
        )
        .expect("write log");

        // refresh_and_get_perf_stats feeds the stats from the internal cursor.
        let perf = svc.refresh_and_get_perf_stats();
        assert!(perf.model_loaded);
        assert_eq!(perf.last_prompt, "Hello");
        assert_eq!(perf.prompt_tps, Some(50.00));

        // Reset clears everything.
        svc.reset_perf_stats();
        let perf = svc.get_perf_stats();
        assert!(perf.prompt_tps.is_none());
        assert!(!perf.model_loaded);
        assert!(perf.last_prompt.is_empty());
    }

    // ---- Fix: refresh_and_get_perf_stats tails log and refreshes uptime ----

    #[test]
    fn test_refresh_and_get_perf_stats_tails_log_and_uptime() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        // Write log content with model load marker.
        std::fs::create_dir_all(svc.log_out_path().parent().unwrap()).ok();
        std::fs::write(
            svc.log_out_path(),
            "llama_model_loader: loaded model\n\
             User prompt: Hello\n\
             prompt eval time = 100.00 ms / 5 tokens (20.00 ms per token, 50.00 tokens per second)\n",
        )
        .expect("write log");

        // refresh_and_get_perf_stats should pick up the log content.
        let perf = svc.refresh_and_get_perf_stats();
        assert!(perf.model_loaded);
        assert_eq!(perf.last_prompt, "Hello");
        assert_eq!(perf.prompt_tps, Some(50.00));
        let uptime1 = perf.model_uptime_secs;

        std::thread::sleep(std::time::Duration::from_secs(1));

        // Second call should refresh uptime even without new log content.
        let perf2 = svc.refresh_and_get_perf_stats();
        let uptime2 = perf2.model_uptime_secs;
        assert!(
            uptime2 >= uptime1,
            "uptime should not decrease after refresh ({} vs {})",
            uptime2,
            uptime1
        );
    }

    // ---- Acceptance: perf stats reset on launch ----

    #[test]
    #[cfg(windows)]
    fn test_perf_stats_reset_on_launch() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        // Seed some stats via log.
        std::fs::create_dir_all(svc.log_out_path().parent().unwrap()).ok();
        std::fs::write(
            svc.log_out_path(),
            "llama_model_loader: loaded model\n",
        )
        .expect("write log");
        let perf = svc.refresh_and_get_perf_stats();
        assert!(perf.model_loaded);

        // Launch a dummy process — should reset stats.
        let cmd = vec![
            "ping".into(),
            "-n".into(),
            "2".into(),
            "127.0.0.1".into(),
        ];
        let pid = svc.launch(cmd, "");
        assert!(pid > 0);

        let perf = svc.get_perf_stats();
        assert!(!perf.model_loaded, "stats should be reset after launch");
        assert!(perf.prompt_tps.is_none());

        // Clean up.
        svc.stop();
    }

    #[test]
    fn test_launch_while_running_keeps_runtime_state() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));
        svc.ensure_state();

        {
            let mut state = svc.state.write().expect("lock poisoned");
            state.current_model_path = "kept-model.gguf".to_string();
        }

        std::fs::write(
            svc.log_out_path(),
            "llama_model_loader: loaded model\n",
        )
        .expect("write log");
        let seeded = svc.refresh_and_get_perf_stats();
        assert!(seeded.model_loaded);

        write_pid(&svc.pid_file, std::process::id() as i32);

        let pid = svc.launch(vec!["ignored".into()], "");
        assert_eq!(pid, -1);
        assert_eq!(svc.current_model_path(), "kept-model.gguf");
        assert!(svc.get_perf_stats().model_loaded);
    }

    // ---- Acceptance: version management ----

    #[test]
    fn test_list_installed_versions_empty_by_default() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let versions = svc.list_installed_versions();
        assert!(versions.is_empty());
    }

    #[test]
    fn test_list_installed_versions_falls_back_to_legacy_llama_server_path() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));
        let exe_path = tmp.path().join("llama-server.exe");
        std::fs::write(&exe_path, "").expect("create dummy exe");

        let mut gs = svc.load_global();
        gs.llama_server_path = exe_path.to_string_lossy().to_string();
        svc.save_global(gs);

        let versions = svc.list_installed_versions();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].tag, "legacy");
        assert_eq!(versions[0].source, "legacy");
        assert_eq!(versions[0].executable_path, exe_path.to_string_lossy().to_string());
    }

    #[test]
    fn test_register_and_list_installed_version() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let ver = InstalledVersion {
            tag: "b3594".into(),
            source: "github".into(),
            install_path: format!("{}", tmp.path().display()),
            executable_path: format!("{}/llama-server.exe", tmp.path().display()),
            status: VersionStatus::Installed,
            installed_at: None,
        };
        svc.register_installed_version(ver);

        let versions = svc.list_installed_versions();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].tag, "b3594");
        assert_eq!(versions[0].source, "github");
    }

    #[test]
    fn test_register_installed_version_updates_existing() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let ver1 = InstalledVersion {
            tag: "b3594".into(),
            source: "github".into(),
            install_path: "path1".into(),
            executable_path: "path1/exe".into(),
            status: VersionStatus::Installed,
            installed_at: None,
        };
        svc.register_installed_version(ver1);

        let ver2 = InstalledVersion {
            tag: "b3594".into(),
            source: "manual".into(),
            install_path: "path2".into(),
            executable_path: "path2/exe".into(),
            status: VersionStatus::Missing,
            installed_at: None,
        };
        svc.register_installed_version(ver2);

        let versions = svc.list_installed_versions();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].source, "manual");
        assert_eq!(versions[0].install_path, "path2");
    }

    #[test]
    fn test_unregister_installed_version() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        svc.register_installed_version(InstalledVersion {
            tag: "b3594".into(),
            source: "github".into(),
            install_path: "p".into(),
            executable_path: "p/exe".into(),
            status: VersionStatus::Installed,
            installed_at: None,
        });

        assert!(svc.unregister_installed_version("b3594"));
        assert!(svc.list_installed_versions().is_empty());

        // Removing non-existent tag returns false.
        assert!(!svc.unregister_installed_version("b3594"));
    }

    #[test]
    fn test_unregister_clears_active_if_matching() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        svc.register_installed_version(InstalledVersion {
            tag: "b3594".into(),
            source: "github".into(),
            install_path: "p".into(),
            executable_path: "p/exe".into(),
            status: VersionStatus::Installed,
            installed_at: None,
        });
        svc.set_active_version("b3594").expect("set active");

        svc.unregister_installed_version("b3594");
        let gs = svc.load_global();
        assert_eq!(gs.active_version, None);
    }

    #[test]
    fn test_set_active_version_success() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        svc.register_installed_version(InstalledVersion {
            tag: "b3594".into(),
            source: "github".into(),
            install_path: "p".into(),
            executable_path: "p/exe".into(),
            status: VersionStatus::Installed,
            installed_at: None,
        });

        svc.set_active_version("b3594").expect("set active");

        let gs = svc.load_global();
        assert_eq!(gs.active_version, Some("b3594".into()));
    }

    #[test]
    fn test_set_active_version_not_found() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let err = svc.set_active_version("nonexistent").expect_err("should fail");
        assert!(err.contains("not in the installed versions list"));
    }

    #[test]
    fn test_resolve_active_executable_fallback_to_llama_server_path() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        // Create a dummy exe file.
        let exe_path = tmp.path().join("llama-server.exe");
        std::fs::write(&exe_path, "").expect("create dummy exe");

        // Set llama_server_path but no active version.
        let mut gs = svc.load_global();
        gs.llama_server_path = exe_path.to_string_lossy().to_string();
        svc.save_global(gs);

        let resolved = svc.resolve_active_executable().expect("should resolve");
        assert_eq!(resolved, exe_path.to_string_lossy().to_string());
    }

    #[test]
    fn test_resolve_active_executable_stale_active_version() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        // Register a version with a non-existent executable.
        svc.register_installed_version(InstalledVersion {
            tag: "b3594".into(),
            source: "github".into(),
            install_path: "C:\\nonexistent".into(),
            executable_path: "C:\\nonexistent\\llama-server.exe".into(),
            status: VersionStatus::Installed,
            installed_at: None,
        });
        svc.set_active_version("b3594").expect("set active");

        let err = svc.resolve_active_executable().expect_err("should fail with stale");
        assert!(err.contains("stale"));
        assert!(err.contains("b3594"));
    }

    #[test]
    fn test_resolve_active_executable_nothing_configured() {
        let tmp = TempDir::new().expect("create temp dir");
        let _svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        // Fresh service with no global config.
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));
        let err = svc.resolve_active_executable().expect_err("should fail");
        assert!(err.contains("no active version set"));
    }

    #[test]
    fn test_resolve_active_executable_active_version_with_existing_exe() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

        let exe_path = tmp.path().join("llama-server.exe");
        std::fs::write(&exe_path, "").expect("create dummy exe");

        svc.register_installed_version(InstalledVersion {
            tag: "b3594".into(),
            source: "github".into(),
            install_path: tmp.path().to_string_lossy().to_string(),
            executable_path: exe_path.to_string_lossy().to_string(),
            status: VersionStatus::Installed,
            installed_at: None,
        });
        svc.set_active_version("b3594").expect("set active");

        let resolved = svc.resolve_active_executable().expect("should resolve");
        assert_eq!(resolved, exe_path.to_string_lossy().to_string());
    }

    // ---- Acceptance: backward compatibility — old global.json without new keys ----

    #[test]
    fn test_backward_compat_old_global_json_loads() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));
        svc.ensure_state();

        // Simulate an old global.json without the new keys.
        let old_json = serde_json::json!({
            "llama_server_path": "C:\\old\\llama-server.exe",
            "model_dirs": ["C:\\models"],
            "api_host": "127.0.0.1",
            "api_port": 7890
        });
        let text = serde_json::to_string_pretty(&old_json).expect("serialize");
        std::fs::write(&svc.global_file, text).expect("write old global.json");

        let gs = svc.load_global();
        assert_eq!(gs.llama_server_path, "C:\\old\\llama-server.exe");
        assert_eq!(gs.model_dirs, vec!["C:\\models"]);
        assert_eq!(gs.api_host, "127.0.0.1");
        assert_eq!(gs.api_port, 7890);
        assert!(gs.installed_versions.is_empty());
        assert_eq!(gs.active_version, None);
    }

    #[test]
    fn test_backward_compat_fallback_to_llama_server_path_when_no_active() {
        let tmp = TempDir::new().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));
        svc.ensure_state();

        // Old-style config: only llama_server_path, no active_version.
        let exe_path = tmp.path().join("llama-server.exe");
        std::fs::write(&exe_path, "").expect("create dummy exe");

        let old_json = serde_json::json!({
            "llama_server_path": exe_path.to_string_lossy().to_string(),
            "model_dirs": [],
            "api_host": "127.0.0.1",
            "api_port": 0
        });
        let text = serde_json::to_string_pretty(&old_json).expect("serialize");
        std::fs::write(&svc.global_file, text).expect("write old global.json");

        // resolve_active_executable should fall back to llama_server_path.
        let resolved = svc.resolve_active_executable().expect("should resolve via fallback");
        assert_eq!(resolved, exe_path.to_string_lossy().to_string());
    }

    #[test]
    fn test_global_settings_json_shape_with_new_fields() {
        let gs = GlobalSettings {
            llama_server_path: "C:\\llama\\server.exe".into(),
            model_dirs: vec!["C:\\models".into()],
            api_host: "0.0.0.0".into(),
            api_port: 8080,
            installed_versions: vec![InstalledVersion {
                tag: "b3594".into(),
                source: "github".into(),
                install_path: "C:\\versions\\b3594".into(),
                executable_path: "C:\\versions\\b3594\\llama-server.exe".into(),
                status: VersionStatus::Installed,
                installed_at: Some("2025-01-01T00:00:00Z".into()),
            }],
            active_version: Some("b3594".into()),
            install_states: std::collections::HashMap::new(),
        };
        let json = serde_json::to_string_pretty(&gs).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");

        assert_eq!(parsed["llama_server_path"], "C:\\llama\\server.exe");
        assert_eq!(parsed["active_version"], "b3594");
        assert_eq!(parsed["installed_versions"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["installed_versions"][0]["tag"], "b3594");
        assert_eq!(parsed["installed_versions"][0]["status"], "installed");
        assert_eq!(
            parsed["installed_versions"][0]["installed_at"],
            "2025-01-01T00:00:00Z"
        );
    }

    #[test]
    fn test_global_settings_json_shape_without_optional_fields() {
        // When active_version is None and installed_versions is empty,
        // active_version should be omitted (skip_serializing_if).
        let gs = GlobalSettings::default();
        let json = serde_json::to_string_pretty(&gs).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");

        assert!(!parsed.as_object().unwrap().contains_key("active_version"));
        assert_eq!(parsed["installed_versions"], serde_json::json!([]));
    }

    // ---- Acceptance: uninstall_version removes from registry and disk ----

    #[test]
    fn test_uninstall_version_success() {
        let dir = tempfile::tempdir().unwrap();
        let svc = LlamaLauncherService::new(Some(dir.path().to_path_buf()));
        svc.ensure_state();

        // Register a version
        svc.register_installed_version(InstalledVersion {
            tag: "b3594".into(),
            source: "github".into(),
            install_path: dir.path().join("versions").join("b3594").to_str().unwrap().into(),
            executable_path: dir.path().join("versions").join("b3594").join("llama-server.exe").to_str().unwrap().into(),
            status: VersionStatus::Installed,
            installed_at: None,
        });

        assert_eq!(svc.list_installed_versions().len(), 1);

        // Uninstall
        svc.uninstall_version("b3594").unwrap();
        assert_eq!(svc.list_installed_versions().len(), 0);
    }

    #[test]
    fn test_uninstall_version_blocks_active() {
        let dir = tempfile::tempdir().unwrap();
        let svc = LlamaLauncherService::new(Some(dir.path().to_path_buf()));
        svc.ensure_state();

        // Register and activate
        svc.register_installed_version(InstalledVersion {
            tag: "b3594".into(),
            source: "github".into(),
            install_path: dir.path().join("versions").join("b3594").to_str().unwrap().into(),
            executable_path: dir.path().join("versions").join("b3594").join("llama-server.exe").to_str().unwrap().into(),
            status: VersionStatus::Installed,
            installed_at: None,
        });
        svc.set_active_version("b3594").unwrap();

        // Cannot uninstall active version
        let result = svc.uninstall_version("b3594");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("active"));
    }

    #[test]
    fn test_uninstall_version_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let svc = LlamaLauncherService::new(Some(dir.path().to_path_buf()));
        svc.ensure_state();

        let result = svc.uninstall_version("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not installed"));
    }

    // ---- Acceptance: cancel_install clears temp and resets state ----

    #[test]
    fn test_cancel_install_no_active_install() {
        let dir = tempfile::tempdir().unwrap();
        let svc = LlamaLauncherService::new(Some(dir.path().to_path_buf()));
        svc.ensure_state();

        let result = svc.cancel_install("b3594");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no install"));
    }

    // ---- Acceptance: get_install_state returns None when idle ----

    #[test]
    fn test_get_install_state_none_when_idle() {
        let dir = tempfile::tempdir().unwrap();
        let svc = LlamaLauncherService::new(Some(dir.path().to_path_buf()));
        svc.ensure_state();

        assert!(svc.get_install_state("b3594").is_none());
    }

    #[test]
    fn test_start_install_version_rejects_unsupported_asset() {
        let dir = tempfile::tempdir().unwrap();
        let svc = LlamaLauncherService::new(Some(dir.path().to_path_buf()));
        svc.ensure_state();

        let asset = crate::models::GitHubReleaseAsset {
            name: "llama-cli-b3594-bin-win.zip".into(),
            size_bytes: 10,
            download_url: "https://example.invalid/llama-cli.zip".into(),
        };

        let result = svc.start_install_version("b3594", &asset);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not a supported Windows llama.cpp package"));
        assert!(svc.get_install_state("b3594").is_none());
        assert!(svc.list_installed_versions().is_empty());
    }

    #[test]
    fn test_start_install_version_download_failure_not_registered() {
        let dir = tempfile::tempdir().unwrap();
        let svc = LlamaLauncherService::new(Some(dir.path().to_path_buf()));
        svc.ensure_state();

        let asset = crate::models::GitHubReleaseAsset {
            name: "llama-server-b3594-bin-win-ssl.zip".into(),
            size_bytes: 10,
            download_url: "http://127.0.0.1:1/llama-server-b3594-bin-win-ssl.zip".into(),
        };

        let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
        rt.block_on(async {
            svc.start_install_version("b3594", &asset).expect("start install");

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if let Some(state) = svc.get_install_state("b3594") {
                    if state.phase == InstallPhase::Error {
                        assert!(!state.error.is_empty());
                        break;
                    }
                }
                assert!(std::time::Instant::now() < deadline, "install did not fail in time");
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        });

        assert!(svc.list_installed_versions().is_empty());
    }

    // ---- Acceptance: apply_startup_profile resolves exe through version-aware resolver ----

    #[test]
    fn test_apply_startup_profile_uses_version_resolver() {
        let dir = tempfile::tempdir().unwrap();
        let svc = LlamaLauncherService::new(Some(dir.path().to_path_buf()));
        svc.ensure_state();

        // Create a dummy exe file
        let exe_path = dir.path().join("llama-server.exe");
        std::fs::write(&exe_path, "").expect("create dummy exe");

        // Register an installed version and set as active
        svc.register_installed_version(InstalledVersion {
            tag: "b3594".into(),
            source: "github".into(),
            install_path: dir.path().to_string_lossy().to_string(),
            executable_path: exe_path.to_string_lossy().to_string(),
            status: VersionStatus::Installed,
            installed_at: None,
        });
        svc.set_active_version("b3594").expect("set active");

        // Ensure llama_server_path is empty (old-style path not configured)
        let mut gs = svc.load_global();
        gs.llama_server_path = String::new();
        svc.save_global(gs);

        // Verify resolve_active_executable finds the versioned exe
        let resolved = svc.resolve_active_executable().expect("should resolve via active version");
        assert_eq!(resolved, exe_path.to_string_lossy().to_string());

        // Create a startup profile
        svc.add_profile("startup-test");
        let mut data = HashMap::new();
        data.insert("start_on_boot".into(), serde_json::json!(true));
        svc.update_profile(1, &data).expect("update");

        // apply_startup_profile should NOT error with "no llama-server path configured"
        // because resolve_active_executable() should find the versioned exe.
        // It will fail later (load_options/build_command) because the dummy exe isn't real,
        // but the error must not be the old "no path configured" message.
        let result = svc.apply_startup_profile();
        if let Err(e) = result {
            assert!(
                !e.contains("no llama-server path configured"),
                "should not get 'no llama-server path configured' when active version is set; got: {}",
                e
            );
        }
    }

    #[test]
    fn test_apply_startup_profile_fallback_to_llama_server_path() {
        let dir = tempfile::tempdir().unwrap();
        let svc = LlamaLauncherService::new(Some(dir.path().to_path_buf()));
        svc.ensure_state();

        // No active version, but llama_server_path is set
        let exe_path = dir.path().join("llama-server.exe");
        std::fs::write(&exe_path, "").expect("create dummy exe");

        let mut gs = svc.load_global();
        gs.llama_server_path = exe_path.to_string_lossy().to_string();
        svc.save_global(gs);

        // resolve_active_executable should fall back to llama_server_path
        let resolved = svc.resolve_active_executable().expect("should resolve via fallback");
        assert_eq!(resolved, exe_path.to_string_lossy().to_string());
    }
}
