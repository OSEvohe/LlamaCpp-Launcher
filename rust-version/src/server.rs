use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, RwLock};

use axum::body::to_bytes;
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::net::TcpListener;

use crate::models::{GlobalSettings, Profile};
use crate::service::LlamaLauncherService;

const MAX_BODY: usize = 1 * 1024 * 1024;

pub type SharedState = Arc<RwLock<LlamaLauncherService>>;

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.to_string(),
        }
    }

    fn not_found(message: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.to_string(),
        }
    }

    fn payload_too_large() -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            message: "request body too large".to_string(),
        }
    }

    fn internal(message: &str) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}

pub fn app(state: SharedState) -> Router {
    Router::new()
        .route("/", get(get_dashboard))
        .route("/api/profiles", get(get_profiles).post(post_profile))
        .route("/api/status", get(get_status))
        .route("/api/launch", post(post_launch))
        .route("/api/stop", post(post_stop))
        .route("/api/restart", post(post_restart))
        .route("/api/logs", get(get_logs))
        .route("/api/monitoring", get(get_monitoring))
        .route("/api/perf", get(get_perf))
        .route("/api/perf/reset", post(post_perf_reset))
        .route("/api/options", get(get_options))
        .route("/api/models", get(get_models))
        .route("/api/versions/installed", get(get_versions_installed))
        .route("/api/versions/available", get(get_versions_available))
        .route("/api/versions/install", post(post_versions_install))
        .route("/api/versions/:tag/active", post(post_versions_set_active))
        .route("/api/versions/:tag", delete(delete_versions_uninstall))
        .route(
            "/api/profiles/:index",
            get(get_profile).put(put_profile).delete(delete_profile),
        )
        .route("/api/profiles/:index/duplicate", post(duplicate_profile))
        .route("/api/settings", get(get_settings).put(put_settings))
        .fallback(not_found)
        .with_state(state)
}

pub async fn serve(listener: TcpListener, state: SharedState) -> std::io::Result<()> {
    axum::serve(listener, app(state)).await
}

pub async fn serve_with_shutdown<F>(
    listener: TcpListener,
    state: SharedState,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown)
        .await
}

async fn not_found() -> ApiError {
    ApiError::not_found("not found")
}

const DASHBOARD_TEMPLATE: &str = include_str!("../static/dashboard.html");
const DASHBOARD_VERSION_PLACEHOLDER: &str = "__APP_VERSION__";

fn app_version() -> &'static str {
    match option_env!("LLAMA_LAUNCHER_GIT_COMMIT") {
        Some(v) => v,
        None => "unknown",
    }
}

fn render_dashboard_html() -> String {
    DASHBOARD_TEMPLATE.replace(DASHBOARD_VERSION_PLACEHOLDER, app_version())
}

async fn get_dashboard() -> Html<String> {
    Html(render_dashboard_html())
}

async fn get_profiles(State(state): State<SharedState>) -> Json<Vec<Profile>> {
    let service = state.read().expect("service lock poisoned");
    Json(service.load_profiles())
}

async fn get_status(State(state): State<SharedState>) -> Json<Value> {
    let service = state.read().expect("service lock poisoned");
    let (running, pid) = service.is_server_running();
    Json(serde_json::json!({
        "running": running,
        "pid": if running { Value::from(pid) } else { Value::Null },
    }))
}

#[derive(Deserialize)]
struct LogsQuery {
    last_size: Option<String>,
    last_marker: Option<String>,
}

async fn get_logs(
    State(state): State<SharedState>,
    Query(query): Query<LogsQuery>,
) -> Json<Value> {
    let last_size = query
        .last_size
        .as_deref()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let last_marker = query.last_marker.unwrap_or_default();

    let service = state.read().expect("service lock poisoned");
    let (chunk, new_size, reset, new_marker) = service.tail_log(last_size, &last_marker);
    Json(serde_json::json!({
        "chunk": chunk,
        "last_size": new_size,
        "reset": reset,
        "last_marker": new_marker,
    }))
}

 async fn get_monitoring(State(state): State<SharedState>) -> Json<Value> {
    let service = state.read().expect("service lock poisoned");
    let (running, pid) = service.is_server_running();
    // TDA: the service assembles the full monitoring payload in one call.
    let monitoring_payload = service.build_monitoring_payload(running, pid);

    let mut response = monitoring_payload.as_object().unwrap().clone();
    response.insert("running".into(), Value::Bool(running));
    response.insert("pid".into(), if running { Value::from(pid) } else { Value::Null });
    Json(Value::Object(response))
}

async fn get_perf(State(state): State<SharedState>) -> Json<Value> {
    let service = state.read().expect("service lock poisoned");
    let perf = service.refresh_and_get_perf_stats();
    Json(serde_json::json!({
        "prompt_tps": perf.prompt_tps,
        "gen_tps": perf.gen_tps,
        "model_loaded": perf.model_loaded,
        "loaded_model_path": service.current_model_path(),
        "model_loaded_at": perf.model_loaded_at,
        "model_uptime_secs": perf.model_uptime_secs,
        "last_prompt": perf.last_prompt,
    }))
}

async fn post_perf_reset(State(state): State<SharedState>) -> Json<Value> {
    let service = state.read().expect("service lock poisoned");
    service.reset_perf_stats();
    Json(serde_json::json!({ "reset": true }))
}

async fn get_options(State(state): State<SharedState>) -> Result<Json<Value>, ApiError> {
    let service = state.read().expect("service lock poisoned");
    let exe_path = match service.resolve_active_executable() {
        Ok(path) => path,
        Err(_) => {
            let settings = service.load_global();
            if settings.llama_server_path.trim().is_empty() {
                service.default_server_path().to_string_lossy().to_string()
            } else {
                settings.llama_server_path
            }
        }
    };
    let options = service
        .load_options(&exe_path)
        .map_err(|err| ApiError::internal(&err))?;
    Ok(Json(serde_json::to_value(options).expect("serialize options")))
}

async fn get_models(State(state): State<SharedState>) -> Json<Value> {
    let service = state.read().expect("service lock poisoned");
    let settings = service.load_global();
    let models = service.discover_models(&settings.model_dirs);
    Json(serde_json::json!({ "models": models }))
}

#[derive(Serialize)]
struct InstalledVersionsResponse {
    installed_versions: Vec<crate::models::InstalledVersion>,
    active_version: Option<String>,
}

async fn get_versions_installed(
    State(state): State<SharedState>,
) -> Result<Json<InstalledVersionsResponse>, ApiError> {
    let service = state.read().expect("service lock poisoned");
    let settings = service.load_global();
    Ok(Json(InstalledVersionsResponse {
        installed_versions: service.list_installed_versions(),
        active_version: settings.active_version,
    }))
}

async fn get_versions_available(State(state): State<SharedState>) -> Result<Json<Value>, ApiError> {
    let _ = state;
    let releases = crate::versions::fetch_releases()
        .await
        .map_err(|err| ApiError::internal(&err.to_string()))?;

    let available = map_available_releases(releases);

    Ok(Json(serde_json::json!({ "releases": available })))
}

#[derive(Serialize)]
struct AvailableWindowsVariant {
    asset_name: String,
    variant: String,
    size_bytes: u64,
}

#[derive(Serialize)]
struct AvailableRelease {
    tag_name: String,
    name: String,
    published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    windows_asset: Option<crate::models::GitHubReleaseAsset>,
    windows_variants: Vec<AvailableWindowsVariant>,
}

fn map_available_releases(releases: Vec<crate::models::GitHubRelease>) -> Vec<AvailableRelease> {
    releases
        .into_iter()
        .map(|r| {
            let windows_variants = r
                .assets
                .iter()
                .filter(|a| crate::versions::is_supported_windows_asset_name(&a.name))
                .map(|a| AvailableWindowsVariant {
                    asset_name: a.name.clone(),
                    variant: crate::versions::classify_windows_variant(&a.name).to_string(),
                    size_bytes: a.size_bytes,
                })
                .collect();
            AvailableRelease {
                tag_name: r.tag_name,
                name: r.name,
                published_at: r.published_at,
                windows_asset: crate::versions::find_windows_asset(&r.assets),
                windows_variants,
            }
        })
        .collect()
}

#[derive(Deserialize)]
struct InstallVersionRequest {
    tag: String,
    asset_name: Option<String>,
}

async fn post_versions_install(
    State(state): State<SharedState>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let body = parse_json_object(request, false)
        .await?
        .ok_or_else(|| ApiError::bad_request("missing request body"))?;
    let req: InstallVersionRequest = serde_json::from_value(Value::Object(body))
        .map_err(|_| ApiError::bad_request("invalid install request"))?;

    if req.tag.trim().is_empty() {
        return Err(ApiError::bad_request("tag must be a non-empty string"));
    }
    if let Some(asset_name) = req.asset_name.as_deref() {
        if !crate::versions::is_supported_windows_asset_name(asset_name) {
            return Err(ApiError::bad_request(&format!(
                "unsupported asset '{}': expected supported Windows llama.cpp zip",
                asset_name
            )));
        }
    }

    let releases = crate::versions::fetch_releases()
        .await
        .map_err(|err| ApiError::internal(&err.to_string()))?;
    let release = releases
        .iter()
        .find(|r| r.tag_name == req.tag)
        .ok_or_else(|| ApiError::not_found(&format!("release '{}' not found", req.tag)))?;

    let asset = if let Some(asset_name) = req.asset_name.as_deref() {
        release
            .assets
            .iter()
            .find(|a| a.name == asset_name)
            .cloned()
            .ok_or_else(|| ApiError::not_found(&format!("asset '{}' not found", asset_name)))?
    } else {
        crate::versions::find_windows_asset(&release.assets)
            .ok_or_else(|| ApiError::bad_request("no supported Windows llama.cpp asset found"))?
    };

    let service = state.read().expect("service lock poisoned");
    service
        .start_install_version(&req.tag, &asset)
        .map_err(|err| ApiError::bad_request(&err))?;

    Ok(Json(serde_json::json!({ "started": true, "tag": req.tag, "asset": asset.name })))
}

async fn post_versions_set_active(
    State(state): State<SharedState>,
    Path(tag): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let service = state.read().expect("service lock poisoned");
    service
        .set_active_version(&tag)
        .map_err(|err| ApiError::bad_request(&err))?;
    Ok(Json(serde_json::json!({ "active_version": tag })))
}

async fn delete_versions_uninstall(
    State(state): State<SharedState>,
    Path(tag): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let service = state.read().expect("service lock poisoned");
    service
        .uninstall_version(&tag)
        .map_err(|err| ApiError::bad_request(&err))?;
    Ok(Json(serde_json::json!({ "uninstalled": tag })))
}

fn prepare_launch_data(
    body: Option<Map<String, Value>>,
    service: &LlamaLauncherService,
) -> Result<(String, Vec<String>), ApiError> {
    let body = body.ok_or_else(|| ApiError::bad_request("missing request body"))?;
    let exe_path = match body.get("exe_path") {
        None => String::new(),
        Some(Value::String(v)) => v.clone(),
        Some(_) => return Err(ApiError::bad_request("exe_path must be a string")),
    };

    let profiles = service.load_profiles();
    let idx = if let Some(profile_uid) = body.get("profile_uid") {
        let uid = profile_uid
            .as_str()
            .ok_or_else(|| ApiError::bad_request("profile_uid must be a string"))?;
        profiles
            .iter()
            .position(|p| p.uid == uid)
            .ok_or_else(|| ApiError::bad_request(&format!("profile uid {} not found", uid)))?
    } else {
        let profile_index = match body.get("profile_index") {
            None => 0_i64,
            Some(Value::Bool(_)) => return Err(ApiError::bad_request("profile_index must be an integer")),
            Some(v) => v
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("profile_index must be an integer"))?,
        };
        if profile_index < 0 {
            return Err(ApiError::bad_request(&format!(
                "profile index {} out of range",
                profile_index
            )));
        }
        let idx = profile_index as usize;
        if idx >= profiles.len() {
            return Err(ApiError::bad_request(&format!(
                "profile index {} out of range",
                profile_index
            )));
        }
        idx
    };
    let profile = &profiles[idx];

    let resolved_exe = if exe_path.trim().is_empty() {
        service
            .resolve_active_executable()
            .map_err(|err| ApiError::bad_request(&err))?
    } else {
        exe_path
    };
    if resolved_exe.trim().is_empty() {
        return Err(ApiError::bad_request(
            "no exe_path provided and none saved in settings",
        ));
    }

    let options = service
        .load_options(&resolved_exe)
        .map_err(|err| ApiError::internal(&err))?;
    let cmd = service
        .build_command(profile, &resolved_exe, &options)
        .map_err(|err| ApiError::bad_request(&err))?;
    Ok((resolved_exe, cmd))
}

async fn post_launch(
    State(state): State<SharedState>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let body = parse_json_object(request, false).await?;
    let service = state.write().expect("service lock poisoned");
    let (resolved_exe, cmd) = prepare_launch_data(body, &service)?;
    let pid = service.launch(cmd.clone(), &resolved_exe);
    Ok(Json(serde_json::json!({ "pid": pid, "command": cmd })))
}

async fn post_stop(State(state): State<SharedState>) -> Json<Value> {
    let service = state.read().expect("service lock poisoned");
    let pid = service.stop();
    Json(serde_json::json!({ "stopped": pid > 0, "pid": pid }))
}

async fn post_restart(
    State(state): State<SharedState>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let body = parse_json_object(request, false).await?;
    let service = state.write().expect("service lock poisoned");
    let (resolved_exe, cmd) = prepare_launch_data(body, &service)?;
    let pid = service.restart(cmd.clone(), &resolved_exe);
    Ok(Json(serde_json::json!({ "pid": pid, "command": cmd })))
}

async fn get_profile(
    State(state): State<SharedState>,
    Path(profile_ref): Path<String>,
) -> Result<Json<Profile>, ApiError> {
    let service = state.read().expect("service lock poisoned");
    let profiles = service.load_profiles();
    let idx = resolve_profile_index(&profiles, &profile_ref)
        .ok_or_else(|| ApiError::not_found(&format!("profile {} not found", profile_ref)))?;
    Ok(Json(profiles[idx].clone()))
}

async fn post_profile(
    State(state): State<SharedState>,
    request: Request,
) -> Result<(StatusCode, Json<Profile>), ApiError> {
    let body = parse_json_object(request, true).await?;
    let name = match body.and_then(|v| v.get("name").cloned()) {
        Some(Value::String(name)) => name,
        Some(_) => return Err(ApiError::bad_request("name must be a string")),
        None => "default".to_string(),
    };

    let service = state.read().expect("service lock poisoned");
    let profile = service.add_profile(&name);
    Ok((StatusCode::CREATED, Json(profile)))
}

async fn put_profile(
    State(state): State<SharedState>,
    Path(profile_ref): Path<String>,
    request: Request,
) -> Result<Json<Profile>, ApiError> {
    let body = parse_json_object(request, false)
        .await?
        .ok_or_else(|| ApiError::bad_request("missing request body"))?;
    let data: HashMap<String, Value> = body.into_iter().collect();

    let service = state.read().expect("service lock poisoned");
    let profiles = service.load_profiles();
    let idx = resolve_profile_index(&profiles, &profile_ref)
        .ok_or_else(|| ApiError::not_found(&format!("profile {} not found", profile_ref)))?;

    let updated = service.update_profile(idx as i64, &data).map_err(|err| {
        if err.contains("out of range") {
            ApiError::not_found(&err)
        } else {
            ApiError::bad_request(&err)
        }
    })?;
    Ok(Json(updated))
}

async fn delete_profile(
    State(state): State<SharedState>,
    Path(profile_ref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let service = state.read().expect("service lock poisoned");
    let profiles = service.load_profiles();
    let idx = resolve_profile_index(&profiles, &profile_ref)
        .ok_or_else(|| ApiError::not_found(&format!("profile {} not found", profile_ref)))?;
    if !service.delete_profile(idx as i64) {
        return Err(ApiError::not_found(&format!("profile {} not found", profile_ref)));
    }
    Ok(Json(serde_json::json!({ "deleted": idx })))
}

async fn duplicate_profile(
    State(state): State<SharedState>,
    Path(profile_ref): Path<String>,
) -> Result<(StatusCode, Json<Profile>), ApiError> {
    let service = state.read().expect("service lock poisoned");
    let profiles = service.load_profiles();
    let idx = resolve_profile_index(&profiles, &profile_ref)
        .ok_or_else(|| ApiError::not_found(&format!("profile {} not found", profile_ref)))?;
    let profile = service
        .duplicate_profile(idx as i64)
        .map_err(|err| ApiError::not_found(&err))?;
    Ok((StatusCode::CREATED, Json(profile)))
}

fn resolve_profile_index(profiles: &[Profile], profile_ref: &str) -> Option<usize> {
    if let Some(uid_index) = profiles.iter().position(|p| p.uid == profile_ref) {
        return Some(uid_index);
    }
    if let Ok(index) = profile_ref.parse::<usize>() {
        return (index < profiles.len()).then_some(index);
    }
    let mut match_index = None;
    let mut match_count = 0usize;

    for (idx, profile) in profiles.iter().enumerate() {
        if profile.name.eq_ignore_ascii_case(profile_ref) {
            match_count += 1;
            if match_count == 1 {
                match_index = Some(idx);
            } else {
                return None;
            }
        }
    }

    match_index
}

async fn get_settings(State(state): State<SharedState>) -> Json<GlobalSettings> {
    let service = state.read().expect("service lock poisoned");
    Json(service.load_global())
}

async fn put_settings(
    State(state): State<SharedState>,
    request: Request,
) -> Result<Json<GlobalSettings>, ApiError> {
    let mut body = parse_json_object(request, false)
        .await?
        .ok_or_else(|| ApiError::bad_request("missing request body"))?;

    if let Some(api_host) = body.get("api_host") {
        if !api_host.is_string() {
            return Err(ApiError::bad_request("api_host must be a string"));
        }
    }

    if let Some(api_port) = body.get("api_port") {
        if api_port.is_boolean() {
            return Err(ApiError::bad_request("api_port must be an integer"));
        }
        let Some(port) = api_port.as_i64() else {
            return Err(ApiError::bad_request("api_port must be an integer"));
        };
        if !(0..=65535).contains(&port) {
            return Err(ApiError::bad_request("api_port must be between 0 and 65535"));
        }
        body.insert("api_port".to_string(), Value::Number(port.into()));
    }

    let data: HashMap<String, Value> = body.into_iter().collect();
    let service = state.read().expect("service lock poisoned");
    Ok(Json(service.update_global(&data)))
}

async fn parse_json_object(
    request: Request,
    allow_empty: bool,
) -> Result<Option<Map<String, Value>>, ApiError> {
    let bytes = to_bytes(request.into_body(), MAX_BODY)
        .await
        .map_err(|_| ApiError::payload_too_large())?;

    if bytes.is_empty() {
        return if allow_empty {
            Ok(None)
        } else {
            Err(ApiError::bad_request("missing request body"))
        };
    }

    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::bad_request("invalid JSON"))?;
    let object = value
        .as_object()
        .cloned()
        .ok_or_else(|| ApiError::bad_request("invalid JSON"))?;
    Ok(Some(object))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use reqwest::StatusCode;
    use tempfile::tempdir;

    async fn spawn_server() -> (String, SharedState, tokio::task::JoinHandle<()>) {
        let temp = tempdir().expect("create temp dir");
        let app_dir = temp.path().to_path_buf();
        std::mem::forget(temp);

        let service = LlamaLauncherService::new(Some(app_dir));
        let state = Arc::new(RwLock::new(service));

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind listener");
        let addr = listener.local_addr().expect("listener address");

        let state_for_server = Arc::clone(&state);
        let handle = tokio::spawn(async move {
            serve(listener, state_for_server).await.expect("run server");
        });

        (format!("http://{}", addr), state, handle)
    }

    #[tokio::test]
    async fn profiles_settings_crud_and_ephemeral_bind() {
        let (base, _state, handle) = spawn_server().await;
        let client = Client::new();

        let response = client
            .get(format!("{}/api/profiles", base))
            .send()
            .await
            .expect("get profiles");
        assert_eq!(response.status(), StatusCode::OK);

        let response = client
            .post(format!("{}/api/profiles", base))
            .json(&serde_json::json!({ "name": "story05" }))
            .send()
            .await
            .expect("create profile");
        assert_eq!(response.status(), StatusCode::CREATED);
        let created: Profile = response.json().await.expect("parse created profile");
        let created_uid = created.uid.clone();

        let response = client
            .get(format!("{}/api/profiles/{}", base, created_uid))
            .send()
            .await
            .expect("get profile by uid");
        assert_eq!(response.status(), StatusCode::OK);

        let response = client
            .put(format!("{}/api/profiles/1", base))
            .json(&serde_json::json!({ "name": "updated" }))
            .send()
            .await
            .expect("update profile");
        assert_eq!(response.status(), StatusCode::OK);

        let response = client
            .post(format!("{}/api/profiles/1/duplicate", base))
            .send()
            .await
            .expect("duplicate profile");
        assert_eq!(response.status(), StatusCode::CREATED);

        let response = client
            .delete(format!("{}/api/profiles/1", base))
            .send()
            .await
            .expect("delete profile");
        assert_eq!(response.status(), StatusCode::OK);

        let response = client
            .get(format!("{}/api/settings", base))
            .send()
            .await
            .expect("get settings");
        assert_eq!(response.status(), StatusCode::OK);

        let response = client
            .put(format!("{}/api/settings", base))
            .json(&serde_json::json!({ "api_host": "0.0.0.0", "api_port": 8082 }))
            .send()
            .await
            .expect("update settings");
        assert_eq!(response.status(), StatusCode::OK);

        handle.abort();
    }

    #[tokio::test]
    async fn invalid_json_validation_and_unknown_route() {
        let (base, _state, handle) = spawn_server().await;
        let client = Client::new();

        let response = client
            .put(format!("{}/api/settings", base))
            .header("Content-Type", "application/json")
            .body("{")
            .send()
            .await
            .expect("invalid json");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = client
            .put(format!("{}/api/settings", base))
            .json(&serde_json::json!({ "api_host": 123 }))
            .send()
            .await
            .expect("invalid api_host");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = client
            .put(format!("{}/api/settings", base))
            .json(&serde_json::json!({ "api_port": 65536 }))
            .send()
            .await
            .expect("api_port out of range");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = client
            .get(format!("{}/api/unknown", base))
            .send()
            .await
            .expect("unknown route");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        handle.abort();
    }

    #[tokio::test]
    async fn oversized_body_returns_413() {
        let (base, _state, handle) = spawn_server().await;
        let client = Client::new();

        let body = "x".repeat(MAX_BODY + 50);
        let response = client
            .put(format!("{}/api/settings", base))
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .expect("oversized body");

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        handle.abort();
    }

    #[tokio::test]
    async fn runtime_endpoints_logs_monitoring_options_models_and_control_routes() {
        let (base, state, handle) = spawn_server().await;
        let client = Client::new();

        let status = client
            .get(format!("{}/api/status", base))
            .send()
            .await
            .expect("get status");
        assert_eq!(status.status(), StatusCode::OK);
        let status_body: Value = status.json().await.expect("parse status body");
        assert!(status_body.get("running").and_then(|v| v.as_bool()).is_some());
        assert!(status_body.get("pid").is_some());

        let model_dir = {
            let service = state.read().expect("service lock poisoned");
            service
                .log_out_path()
                .parent()
                .expect("state dir")
                .join("models")
        };
        std::fs::create_dir_all(&model_dir).expect("create model dir");
        let model_path = model_dir.join("dummy.gguf");
        std::fs::write(&model_path, "dummy").expect("create model file");

        let exe_output = std::process::Command::new("where")
            .arg("where.exe")
            .output()
            .expect("find where.exe");
        let exe_path = String::from_utf8_lossy(&exe_output.stdout)
            .lines()
            .next()
            .expect("where.exe path")
            .trim()
            .to_string();

        let response = client
            .put(format!("{}/api/settings", base))
            .json(&serde_json::json!({
                "llama_server_path": exe_path,
                "model_dirs": [model_dir.to_string_lossy().to_string()]
            }))
            .send()
            .await
            .expect("update settings for options/models");
        assert_eq!(response.status(), StatusCode::OK);

        let options = client
            .get(format!("{}/api/options", base))
            .send()
            .await
            .expect("get options");
        assert_eq!(options.status(), StatusCode::OK);
        let options_body: Value = options.json().await.expect("parse options body");
        assert!(options_body.is_object());

        let models = client
            .get(format!("{}/api/models", base))
            .send()
            .await
            .expect("get models");
        assert_eq!(models.status(), StatusCode::OK);
        let models_body: Value = models.json().await.expect("parse models body");
        let models_list = models_body
            .get("models")
            .and_then(|v| v.as_array())
            .expect("models array");
        assert!(models_list.iter().any(|v| {
            v.as_str()
                .map(|s| s.ends_with("dummy.gguf"))
                .unwrap_or(false)
        }));

        {
            let service = state.read().expect("service lock poisoned");
            std::fs::write(service.log_out_path(), "line-1\nline-2\n").expect("write log");
        }

        let logs = client
            .get(format!("{}/api/logs?last_size=0&last_marker=", base))
            .send()
            .await
            .expect("get logs initial");
        assert_eq!(logs.status(), StatusCode::OK);
        let logs_body: Value = logs.json().await.expect("parse logs body");
        assert_eq!(
            logs_body.get("chunk").and_then(|v| v.as_str()),
            Some("line-1\nline-2\n")
        );
        let next_size = logs_body
            .get("last_size")
            .and_then(|v| v.as_u64())
            .expect("last_size present");
        let marker = logs_body
            .get("last_marker")
            .and_then(|v| v.as_str())
            .expect("last_marker present")
            .to_string();

        let logs_next = client
            .get(format!("{}/api/logs", base))
            .query(&[("last_size", &next_size.to_string()), ("last_marker", &marker)])
            .send()
            .await
            .expect("get logs next");
        assert_eq!(logs_next.status(), StatusCode::OK);
        let logs_next_body: Value = logs_next.json().await.expect("parse logs next body");
        assert_eq!(logs_next_body.get("chunk").and_then(|v| v.as_str()), Some(""));
        assert_eq!(
            logs_next_body.get("reset").and_then(|v| v.as_bool()),
            Some(false)
        );

        let monitoring = client
            .get(format!("{}/api/monitoring", base))
            .send()
            .await
            .expect("get monitoring");
        assert_eq!(monitoring.status(), StatusCode::OK);
        let monitoring_body: Value = monitoring.json().await.expect("parse monitoring body");
        assert!(monitoring_body.get("ram").is_some());
        assert!(monitoring_body.get("vram").is_some());
        assert!(monitoring_body.get("process_ram_human").is_some());
        // Performance section present in monitoring.
        assert!(monitoring_body.get("performance").is_some());
        let perf_section = monitoring_body.get("performance").expect("performance section");
        assert!(perf_section.get("prompt_tps").is_some());
        assert!(perf_section.get("gen_tps").is_some());
        assert!(perf_section.get("model_loaded").is_some());
        assert!(perf_section.get("last_prompt").is_some());

        let launch_invalid = client
            .post(format!("{}/api/launch", base))
            .json(&serde_json::json!({ "profile_index": 9999 }))
            .send()
            .await
            .expect("launch out of range profile");
        assert_eq!(launch_invalid.status(), StatusCode::BAD_REQUEST);

        let stop = client
            .post(format!("{}/api/stop", base))
            .send()
            .await
            .expect("stop");
        assert_eq!(stop.status(), StatusCode::OK);

        let restart_missing = client
            .post(format!("{}/api/restart", base))
            .send()
            .await
            .expect("restart missing body");
        assert_eq!(restart_missing.status(), StatusCode::BAD_REQUEST);

        handle.abort();
    }

    #[tokio::test]
    async fn perf_endpoint_and_reset() {
        let (base, _state, handle) = spawn_server().await;
        let client = Client::new();

        // GET /api/perf returns OK with expected fields.
        let perf = client
            .get(format!("{}/api/perf", base))
            .send()
            .await
            .expect("get perf");
        assert_eq!(perf.status(), StatusCode::OK);
        let perf_body: Value = perf.json().await.expect("parse perf body");
        assert!(perf_body.get("prompt_tps").is_some());
        assert!(perf_body.get("gen_tps").is_some());
        assert!(perf_body.get("model_loaded").is_some());
        assert!(perf_body.get("model_loaded_at").is_some());
        assert!(perf_body.get("model_uptime_secs").is_some());
        assert!(perf_body.get("last_prompt").is_some());
        // Initially empty.
        assert_eq!(perf_body.get("model_loaded").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(perf_body.get("last_prompt").and_then(|v| v.as_str()), Some(""));

        // POST /api/perf/reset returns OK.
        let reset = client
            .post(format!("{}/api/perf/reset", base))
            .send()
            .await
            .expect("reset perf");
        assert_eq!(reset.status(), StatusCode::OK);
        let reset_body: Value = reset.json().await.expect("parse reset body");
        assert_eq!(reset_body.get("reset").and_then(|v| v.as_bool()), Some(true));

        handle.abort();
    }

    #[test]
    fn test_supported_windows_asset_alternate_naming() {
        // Alternate naming without "bin-win" must be accepted
        assert!(crate::versions::is_supported_windows_asset_name("llama-server-b3594-bin-windows-ssl.zip"));
        assert!(crate::versions::is_supported_windows_asset_name("llama-b9442-bin-win-cpu-x64.zip"));
        assert!(crate::versions::is_supported_windows_asset_name("llama-b9442-bin-win-cuda-x64.zip"));
    }

    #[test]
    fn test_supported_windows_asset_rejects_non_server_and_patches() {
        // Patch asset — rejected
        assert!(!crate::versions::is_supported_windows_asset_name("llama-server-b3594-bin-win-patches.zip"));
        // Linux — rejected
        assert!(!crate::versions::is_supported_windows_asset_name("llama-server-b3594-linux-avx.zip"));
        // macOS — rejected
        assert!(!crate::versions::is_supported_windows_asset_name("llama-server-b3594-macos-arm64.zip"));
        // Non-server — rejected
        assert!(!crate::versions::is_supported_windows_asset_name("llama-cli-b3594-bin-win.zip"));
        // Not a zip — rejected
        assert!(!crate::versions::is_supported_windows_asset_name("llama-server-b3594-bin-win.tar.gz"));
        // Missing Windows token — rejected
        assert!(!crate::versions::is_supported_windows_asset_name("llama-server-b3594-bin-avx2.zip"));
        // darwin must not match via "win" substring
        assert!(!crate::versions::is_supported_windows_asset_name("llama-server-b3594-darwin-arm64.zip"));
    }

    #[test]
    fn versions_available_shape_keeps_legacy_windows_asset_and_new_variants() {
        let releases = vec![crate::models::GitHubRelease {
            tag_name: "b1000".into(),
            name: "b1000".into(),
            published_at: Some("2026-01-01T00:00:00Z".into()),
            assets: vec![
                crate::models::GitHubReleaseAsset {
                    name: "llama-b1000-bin-win-cpu-x64.zip".into(),
                    size_bytes: 10,
                    download_url: "https://example.com/cpu.zip".into(),
                },
                crate::models::GitHubReleaseAsset {
                    name: "llama-b1000-bin-win-cuda-x64.zip".into(),
                    size_bytes: 20,
                    download_url: "https://example.com/cuda.zip".into(),
                },
            ],
        }];

        let body = serde_json::to_value(serde_json::json!({ "releases": map_available_releases(releases) }))
            .expect("serialize response body");
        let release = &body["releases"][0];

        assert_eq!(release["windows_asset"]["name"], "llama-b1000-bin-win-cpu-x64.zip");
        assert_eq!(release["windows_asset"]["size"], 10);
        assert_eq!(release["windows_variants"].as_array().map(|v| v.len()), Some(2));
    }

    #[tokio::test]
    async fn dashboard_shows_version_and_not_placeholder() {
        let (base, _state, handle) = spawn_server().await;
        let client = Client::new();

        let response = client
            .get(format!("{}/", base))
            .send()
            .await
            .expect("get dashboard");
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.text().await.expect("read dashboard body");
        assert!(body.contains("Version: "));
        assert!(!body.contains(DASHBOARD_VERSION_PLACEHOLDER));

        handle.abort();
    }

    #[test]
    fn resolve_profile_index_prefers_exact_uid_over_numeric_index() {
        let profiles = vec![
            Profile {
                uid: "42".into(),
                name: "uid-42".into(),
                ..Profile::default()
            },
            Profile {
                uid: "other".into(),
                name: "p1".into(),
                ..Profile::default()
            },
        ];

        let idx = resolve_profile_index(&profiles, "42").expect("resolve uid");
        assert_eq!(idx, 0);
    }

    #[test]
    fn prepare_launch_data_prefers_profile_uid_over_profile_index() {
        let temp = tempdir().expect("create temp dir");
        let svc = LlamaLauncherService::new(Some(temp.path().to_path_buf()));

        let exe_output = std::process::Command::new("where")
            .arg("where.exe")
            .output()
            .expect("find where.exe");
        let exe_path = String::from_utf8_lossy(&exe_output.stdout)
            .lines()
            .next()
            .expect("where.exe path")
            .trim()
            .to_string();

      svc.save_global(GlobalSettings {
            llama_server_path: exe_path.clone(),
            model_dirs: Vec::new(),
            api_host: "127.0.0.1".into(),
            api_port: 0,
            ..GlobalSettings::default()
        });

        let p0 = Profile {
            uid: "uid-a".into(),
            model_path: temp.path().join("a.gguf").to_string_lossy().to_string(),
            ..Profile::default()
        };
        let p1 = Profile {
            uid: "uid-b".into(),
            model_path: temp.path().join("b.gguf").to_string_lossy().to_string(),
            ..Profile::default()
        };
        std::fs::write(&p0.model_path, "a").expect("write model a");
        std::fs::write(&p1.model_path, "b").expect("write model b");
        svc.save_profiles(vec![p0.clone(), p1.clone()]);

        let mut body = Map::new();
        body.insert("profile_uid".into(), Value::String("uid-b".into()));
        body.insert("profile_index".into(), Value::Number(0.into()));
        body.insert("exe_path".into(), Value::String(exe_path));

        let (_resolved_exe, cmd) = prepare_launch_data(Some(body), &svc).expect("prepare launch");

        assert!(
            cmd.iter().any(|arg| arg == &p1.model_path),
            "command should contain selected profile model path"
        );
    }
}
