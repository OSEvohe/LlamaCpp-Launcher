use std::path::Path;

use crate::models::{InstalledVersion, VersionStatus};
use crate::options::resolve_llama_server_path;

use super::LlamaLauncherService;

pub(super) fn list_installed_versions(service: &LlamaLauncherService) -> Vec<InstalledVersion> {
    let _guard = service.state.read().expect("lock poisoned");
    let gs = service.load_global_internal();
    if !gs.installed_versions.is_empty() {
        return gs.installed_versions;
    }

    if gs.llama_server_path.trim().is_empty() {
        return Vec::new();
    }

    let exe_path = resolve_llama_server_path(&gs.llama_server_path);
    vec![InstalledVersion {
        tag: "legacy".to_string(),
        source: "legacy".to_string(),
        install_path: exe_path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        executable_path: exe_path.to_string_lossy().to_string(),
        status: if exe_path.exists() {
            VersionStatus::Installed
        } else {
            VersionStatus::Missing
        },
        installed_at: None,
    }]
}

pub(super) fn resolve_active_executable(service: &LlamaLauncherService) -> Result<String, String> {
    let _guard = service.state.read().expect("lock poisoned");
    let gs = service.load_global_internal();

    // 1. Try active_version first.
    if let Some(ref tag) = gs.active_version {
        if let Some(ver) = gs.installed_versions.iter().find(|v| v.tag == *tag) {
            let exe_path = &ver.executable_path;
            if !exe_path.is_empty() && Path::new(exe_path).exists() {
                return Ok(exe_path.clone());
            }
            // Executable missing — stale.
            return Err(format!(
                "active version '{}' is stale: executable not found at '{}'",
                tag, exe_path
            ));
        }
    }

    // 2. Fallback to llama_server_path.
    if !gs.llama_server_path.is_empty() && Path::new(&gs.llama_server_path).exists() {
        return Ok(gs.llama_server_path.clone());
    }
    if !gs.llama_server_path.is_empty() {
        return Err(format!(
            "fallback llama_server_path not found: '{}'",
            gs.llama_server_path
        ));
    }

    // 3. Nothing available.
    Err("no active version set and llama_server_path is empty".to_string())
}

pub(super) fn get_install_state(
    service: &LlamaLauncherService,
    tag: &str,
) -> Option<crate::models::InstallState> {
    let _guard = service.state.read().expect("lock poisoned");
    let gs = service.load_global_internal();
    gs.install_states.get(tag).cloned()
}
