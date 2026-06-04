use std::collections::HashMap;

use crate::models::{GlobalSettings, InstalledVersion};

use super::LlamaLauncherService;

pub(super) fn update_global(
    service: &LlamaLauncherService,
    settings_data: &HashMap<String, serde_json::Value>,
) -> GlobalSettings {
    let _guard = service.state.write().expect("lock poisoned");
    let current = service.load_global_internal();
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
        installed_versions: settings_data
            .get("installed_versions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value::<InstalledVersion>(v.clone()).ok())
                    .collect()
            })
            .unwrap_or(current.installed_versions),
        active_version: settings_data
            .get("active_version")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or(current.active_version),
        install_states: current.install_states,
    };
    service.save_global_internal(&settings);
    settings
}
