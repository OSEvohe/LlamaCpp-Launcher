use crate::config;
use crate::models::{GlobalSettings, Profile};

use super::LlamaLauncherService;

pub(super) fn load_profiles_internal(service: &LlamaLauncherService) -> Vec<Profile> {
    if service.is_default_app_dir {
        config::load_profiles()
    } else {
        service.ensure_state();
        if !service.profiles_file.exists() {
            return vec![Profile::default()];
        }
        match std::fs::read_to_string(&service.profiles_file) {
            Ok(text) => {
                let data: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(d) => d,
                    Err(_) => return vec![Profile::default()],
                };
                match data.get("profiles").and_then(|v| v.as_array()) {
                    Some(arr) => {
                        let mut profiles = Vec::new();
                        for item in arr {
                            let mut obj = match item.as_object().cloned() {
                                Some(o) => o,
                                None => continue,
                            };
                            config::normalize_mtp(&mut obj);
                            if let Ok(p) =
                                serde_json::from_value(serde_json::Value::Object(obj))
                            {
                                profiles.push(p);
                            }
                        }
                        if profiles.is_empty() {
                            vec![Profile::default()]
                        } else {
                            if config::normalize_profile_uids(&mut profiles) {
                                save_profiles_internal(service, &profiles);
                            }
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

pub(super) fn save_profiles_internal(service: &LlamaLauncherService, profiles: &[Profile]) {
    if service.is_default_app_dir {
        config::save_profiles(profiles);
    } else {
        service.ensure_state();
        let payload = serde_json::json!({ "profiles": profiles });
        let json = serde_json::to_string_pretty(&payload).expect("serialize profiles");
        std::fs::write(&service.profiles_file, json).expect("write profiles.json");
    }
}

pub(super) fn load_global_internal(service: &LlamaLauncherService) -> GlobalSettings {
    if service.is_default_app_dir {
        config::load_global()
    } else {
        service.ensure_state();
        if !service.global_file.exists() {
            return GlobalSettings::default();
        }
        match std::fs::read_to_string(&service.global_file) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(gs) => gs,
                Err(_) => GlobalSettings::default(),
            },
            Err(_) => GlobalSettings::default(),
        }
    }
}

pub(super) fn save_global_internal(service: &LlamaLauncherService, settings: &GlobalSettings) {
    if service.is_default_app_dir {
        config::save_global(settings);
    } else {
        service.ensure_state();
        let json = serde_json::to_string_pretty(settings).expect("serialize GlobalSettings");
        std::fs::write(&service.global_file, json).expect("write global.json");
    }
}
