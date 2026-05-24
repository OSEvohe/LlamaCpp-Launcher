use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use axum::http::StatusCode;
use llama_launcher::command;
use llama_launcher::models::{LlamaOption, Profile};
use llama_launcher::server;
use llama_launcher::service::LlamaLauncherService;
use reqwest::Client;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const MAX_BODY: usize = 1024 * 1024;

struct TestServer {
    base: String,
    client: Client,
    app_dir: PathBuf,
    _tmp: TempDir,
    handle: JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn boot_server(seed_profiles: Option<Vec<Profile>>) -> TestServer {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let app_dir = tmp.path().to_path_buf();
    let service = LlamaLauncherService::new(Some(app_dir.clone()));
    service.save_profiles(seed_profiles.unwrap_or_else(|| vec![Profile::default()]));

    let state = Arc::new(RwLock::new(service));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");
    let server_state = Arc::clone(&state);
    let handle = tokio::spawn(async move {
        server::serve(listener, server_state)
            .await
            .expect("run server");
    });

    TestServer {
        base: format!("http://{}", addr),
        client: Client::new(),
        app_dir,
        _tmp: tmp,
        handle,
    }
}

fn write_raw_profiles_json(app_dir: &Path, profiles_data: Value) {
    let state_dir = app_dir.join(".launcher");
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    let payload = json!({ "profiles": profiles_data });
    std::fs::write(
        state_dir.join("profiles.json"),
        serde_json::to_string_pretty(&payload).expect("serialize profiles payload"),
    )
    .expect("write profiles.json");
}

#[tokio::test]
async fn get_status_200_schema() {
    let ts = boot_server(None).await;
    let resp = ts.client.get(format!("{}/api/status", ts.base)).send().await.expect("status");
    assert_eq!(resp.status(), StatusCode::OK);
    let data: Value = resp.json().await.expect("status json");
    assert!(data.get("running").is_some());
    assert!(data.get("pid").is_some());
}

#[tokio::test]
async fn get_profiles_200() {
    let ts = boot_server(Some(vec![
        Profile { name: "alpha".into(), ..Profile::default() },
        Profile { name: "beta".into(), ..Profile::default() },
    ]))
    .await;
    let resp = ts.client.get(format!("{}/api/profiles", ts.base)).send().await.expect("profiles");
    assert_eq!(resp.status(), StatusCode::OK);
    let data: Value = resp.json().await.expect("profiles json");
    let arr = data.as_array().expect("profiles array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["name"], "alpha");
    assert_eq!(arr[1]["name"], "beta");
}

#[tokio::test]
async fn get_profile_by_index_200_and_404() {
    let ts = boot_server(Some(vec![
        Profile { name: "first".into(), ..Profile::default() },
        Profile { name: "second".into(), ..Profile::default() },
    ]))
    .await;

    let ok = ts.client.get(format!("{}/api/profiles/0", ts.base)).send().await.expect("profile 0");
    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(ok.json::<Value>().await.expect("profile json")["name"], "first");

    let missing = ts.client.get(format!("{}/api/profiles/99", ts.base)).send().await.expect("profile 99");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(missing.json::<Value>().await.expect("404 json").get("error").is_some());
}

#[tokio::test]
async fn post_profile_valid_invalid_and_oversized() {
    let ts = boot_server(None).await;

    let created = ts
        .client
        .post(format!("{}/api/profiles", ts.base))
        .json(&json!({ "name": "new-profile" }))
        .send()
        .await
        .expect("create profile");
    assert_eq!(created.status(), StatusCode::CREATED);
    assert_eq!(created.json::<Value>().await.expect("created json")["name"], "new-profile");

    let invalid = ts
        .client
        .post(format!("{}/api/profiles", ts.base))
        .json(&json!({ "name": 12345 }))
        .send()
        .await
        .expect("invalid profile");
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert!(invalid.json::<Value>().await.expect("invalid json").get("error").is_some());

    let body = "x".repeat(MAX_BODY + 64);
    let oversized = ts
        .client
        .post(format!("{}/api/profiles", ts.base))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .expect("oversized");
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(oversized.json::<Value>().await.expect("oversized json").get("error").is_some());
}

#[tokio::test]
async fn put_profile_update_missing_body_and_out_of_range() {
    let ts = boot_server(Some(vec![Profile { name: "original".into(), ..Profile::default() }])).await;

    let updated = ts
        .client
        .put(format!("{}/api/profiles/0", ts.base))
        .json(&json!({ "name": "updated" }))
        .send()
        .await
        .expect("update");
    assert_eq!(updated.status(), StatusCode::OK);
    assert_eq!(updated.json::<Value>().await.expect("updated json")["name"], "updated");

    let missing = ts.client.put(format!("{}/api/profiles/0", ts.base)).send().await.expect("missing body");
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);
    assert!(missing.json::<Value>().await.expect("missing json").get("error").is_some());

    let out_of_range = ts
        .client
        .put(format!("{}/api/profiles/99", ts.base))
        .json(&json!({ "name": "x" }))
        .send()
        .await
        .expect("out of range");
    assert_eq!(out_of_range.status(), StatusCode::NOT_FOUND);
    assert!(out_of_range.json::<Value>().await.expect("oor json").get("error").is_some());
}

#[tokio::test]
async fn put_profile_advanced_fields_and_partial_preserve() {
    let ts = boot_server(Some(vec![Profile {
        name: "keep-me".into(),
        model_path: "/models/m.gguf".into(),
        ctx_size: 8192,
        ..Profile::default()
    }]))
    .await;

    let update = ts
        .client
        .put(format!("{}/api/profiles/0", ts.base))
        .json(&json!({
            "advanced_favorites": ["--temp"],
            "advanced_values": { "--temp": "0.5" }
        }))
        .send()
        .await
        .expect("advanced update");
    assert_eq!(update.status(), StatusCode::OK);
    let body: Value = update.json().await.expect("advanced body");
    assert_eq!(body["name"], "keep-me");
    assert_eq!(body["model_path"], "/models/m.gguf");
    assert_eq!(body["ctx_size"], 8192);
    assert_eq!(body["advanced_favorites"], json!(["--temp"]));
    assert_eq!(body["advanced_values"], json!({ "--temp": "0.5" }));

    let get_back = ts.client.get(format!("{}/api/profiles/0", ts.base)).send().await.expect("get back");
    assert_eq!(get_back.status(), StatusCode::OK);
    let get_body: Value = get_back.json().await.expect("get body");
    assert_eq!(get_body["advanced_favorites"], json!(["--temp"]));
    assert_eq!(get_body["advanced_values"], json!({ "--temp": "0.5" }));
}

#[tokio::test]
async fn delete_profile_200_and_404() {
    let ts = boot_server(Some(vec![
        Profile { name: "a".into(), ..Profile::default() },
        Profile { name: "b".into(), ..Profile::default() },
    ]))
    .await;

    let ok = ts.client.delete(format!("{}/api/profiles/0", ts.base)).send().await.expect("delete 0");
    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(ok.json::<Value>().await.expect("delete json")["deleted"], 0);

    let missing = ts.client.delete(format!("{}/api/profiles/99", ts.base)).send().await.expect("delete 99");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(missing.json::<Value>().await.expect("delete 404 json").get("error").is_some());
}

#[tokio::test]
async fn duplicate_profile_crud_and_preserve_fields() {
    let ts = boot_server(Some(vec![
        Profile {
            name: "source".into(),
            model_path: "/models/m.gguf".into(),
            ctx_size: 8192,
            advanced_favorites: vec!["--verbose".into()],
            advanced_values: HashMap::from([("--verbose".into(), "1".into())]),
            advanced_modes: HashMap::from([("--verbose".into(), "flag".into())]),
            ..Profile::default()
        },
        Profile { name: "b".into(), ..Profile::default() },
    ]))
    .await;

    let created = ts
        .client
        .post(format!("{}/api/profiles/0/duplicate", ts.base))
        .json(&json!({}))
        .send()
        .await
        .expect("duplicate");
    assert_eq!(created.status(), StatusCode::CREATED);
    let data: Value = created.json().await.expect("duplicate json");
    assert_eq!(data["name"], "source (copy)");
    assert_eq!(data["model_path"], "/models/m.gguf");
    assert_eq!(data["ctx_size"], 8192);
    assert_eq!(data["advanced_favorites"], json!(["--verbose"]));
    assert_eq!(data["advanced_values"], json!({"--verbose": "1"}));
    assert_eq!(data["advanced_modes"], json!({"--verbose": "flag"}));

    let missing = ts
        .client
        .post(format!("{}/api/profiles/99/duplicate", ts.base))
        .json(&json!({}))
        .send()
        .await
        .expect("duplicate missing");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(missing.json::<Value>().await.expect("dup 404 json").get("error").is_some());

    let ts2 = boot_server(Some(vec![
        Profile { name: "a".into(), ..Profile::default() },
        Profile { name: "b".into(), ..Profile::default() },
    ]))
    .await;
    ts2.client
        .post(format!("{}/api/profiles/1/duplicate", ts2.base))
        .json(&json!({}))
        .send()
        .await
        .expect("duplicate append");
    let list = ts2.client.get(format!("{}/api/profiles", ts2.base)).send().await.expect("list after dup");
    let arr = list.json::<Value>().await.expect("list json").as_array().expect("array").clone();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[2]["name"], "b (copy)");
}

#[tokio::test]
async fn settings_models_logs_unknown_route() {
    let ts = boot_server(None).await;

    let settings = ts.client.get(format!("{}/api/settings", ts.base)).send().await.expect("get settings");
    assert_eq!(settings.status(), StatusCode::OK);
    let settings_json: Value = settings.json().await.expect("settings json");
    assert!(settings_json.get("llama_server_path").is_some());
    assert!(settings_json.get("model_dirs").is_some());
    assert!(settings_json.get("api_host").is_some());
    assert!(settings_json.get("api_port").is_some());

    let put_settings = ts
        .client
        .put(format!("{}/api/settings", ts.base))
        .json(&json!({ "api_host": "0.0.0.0", "api_port": 9090 }))
        .send()
        .await
        .expect("put settings");
    assert_eq!(put_settings.status(), StatusCode::OK);
    let put_json: Value = put_settings.json().await.expect("put settings json");
    assert_eq!(put_json["api_host"], "0.0.0.0");
    assert_eq!(put_json["api_port"], 9090);

    for body in [json!({ "api_port": "not-a-number" }), json!({ "api_port": 70000 }), json!({ "api_host": 12345 })] {
        let bad = ts.client.put(format!("{}/api/settings", ts.base)).json(&body).send().await.expect("bad settings");
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        assert!(bad.json::<Value>().await.expect("bad settings json").get("error").is_some());
    }

    let models = ts.client.get(format!("{}/api/models", ts.base)).send().await.expect("get models");
    assert_eq!(models.status(), StatusCode::OK);
    let models_json: Value = models.json().await.expect("models json");
    assert!(models_json.get("models").and_then(Value::as_array).is_some());

    let logs = ts.client.get(format!("{}/api/logs", ts.base)).send().await.expect("get logs");
    assert_eq!(logs.status(), StatusCode::OK);
    let logs_json: Value = logs.json().await.expect("logs json");
    assert!(logs_json.get("chunk").is_some());
    assert!(logs_json.get("last_size").is_some());
    assert!(logs_json.get("reset").is_some());
    assert!(logs_json.get("last_marker").is_some());

    let missing = ts.client.get(format!("{}/api/nonexistent", ts.base)).send().await.expect("unknown route");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(missing.json::<Value>().await.expect("404 json").get("error").is_some());
}

#[test]
fn mtp_normalization_and_legacy_migration_on_load() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));

    write_raw_profiles_json(
        tmp.path(),
        json!([
            { "name": "mtp-string", "enable_mtp": "true", "spec_draft_n_max": 2 },
            { "name": "mtp-string-false", "enable_mtp": "false", "spec_draft_n_max": 2 },
            { "name": "mtp-str-int", "enable_mtp": true, "spec_draft_n_max": "4" },
            { "name": "mtp-invalid-int", "enable_mtp": true, "spec_draft_n_max": "not-a-number" },
            {
                "name": "legacy-spec-type",
                "enable_mtp": false,
                "spec_draft_n_max": 2,
                "advanced_favorites": ["--spec-type"],
                "advanced_values": { "--spec-type": "draft-mtp" }
            },
            {
                "name": "legacy-draft-n",
                "enable_mtp": true,
                "spec_draft_n_max": 2,
                "advanced_favorites": ["--spec-draft-n-max"],
                "advanced_values": { "--spec-draft-n-max": "5" }
            },
            {
                "name": "legacy-both",
                "enable_mtp": false,
                "spec_draft_n_max": 2,
                "advanced_favorites": ["--spec-type", "--spec-draft-n-max"],
                "advanced_values": { "--spec-type": "draft-mtp", "--spec-draft-n-max": "8" }
            }
        ]),
    );

    let profiles = svc.load_profiles();
    assert_eq!(profiles.len(), 7);

    assert_eq!(profiles[0].enable_mtp, true);
    assert_eq!(profiles[1].enable_mtp, false);
    assert_eq!(profiles[2].spec_draft_n_max, 4);
    assert_eq!(profiles[3].spec_draft_n_max, 2);

    assert_eq!(profiles[4].enable_mtp, true);
    assert!(!profiles[4].advanced_favorites.iter().any(|x| x == "--spec-type"));
    assert!(!profiles[4].advanced_values.contains_key("--spec-type"));

    assert_eq!(profiles[5].spec_draft_n_max, 5);
    assert!(!profiles[5].advanced_favorites.iter().any(|x| x == "--spec-draft-n-max"));
    assert!(!profiles[5].advanced_values.contains_key("--spec-draft-n-max"));

    assert_eq!(profiles[6].enable_mtp, true);
    assert_eq!(profiles[6].spec_draft_n_max, 8);
    assert!(!profiles[6].advanced_values.contains_key("--spec-type"));
    assert!(!profiles[6].advanced_values.contains_key("--spec-draft-n-max"));
}

#[test]
fn build_command_mtp_on_off_and_dedup() {
    let mtp_profile = Profile {
        name: "mtp-test".into(),
        model_path: "/fake/model.gguf".into(),
        enable_mtp: true,
        spec_draft_n_max: 4,
        ..Profile::default()
    };
    let cmd = command::build_command(Path::new("/fake/server.exe"), &mtp_profile, &HashMap::new())
        .expect("build command mtp");
    assert!(cmd.contains(&"--spec-type".to_string()));
    assert!(cmd.contains(&"--spec-draft-n-max".to_string()));
    let st_idx = cmd.iter().position(|x| x == "--spec-type").expect("spec type idx");
    let dn_idx = cmd.iter().position(|x| x == "--spec-draft-n-max").expect("draft n idx");
    assert_eq!(cmd[st_idx + 1], "draft-mtp");
    assert_eq!(cmd[dn_idx + 1], "4");

    let no_mtp_profile = Profile {
        name: "no-mtp".into(),
        model_path: "/fake/model.gguf".into(),
        enable_mtp: false,
        spec_draft_n_max: 2,
        ..Profile::default()
    };
    let no_cmd = command::build_command(Path::new("/fake/server.exe"), &no_mtp_profile, &HashMap::new())
        .expect("build command no mtp");
    assert!(!no_cmd.contains(&"--spec-type".to_string()));
    assert!(!no_cmd.contains(&"--spec-draft-n-max".to_string()));

    let dup_profile = Profile {
        name: "mtp-dup".into(),
        model_path: "/fake/model.gguf".into(),
        enable_mtp: true,
        spec_draft_n_max: 4,
        advanced_favorites: vec!["--spec-type".into(), "--spec-draft-n-max".into(), "--verbose".into()],
        advanced_values: HashMap::from([
            ("--spec-type".into(), "draft-mtp".into()),
            ("--spec-draft-n-max".into(), "9".into()),
            ("--verbose".into(), "1".into()),
        ]),
        ..Profile::default()
    };
    let dup_cmd = command::build_command(Path::new("/fake/server.exe"), &dup_profile, &HashMap::new())
        .expect("build command dedup");
    assert_eq!(dup_cmd.iter().filter(|x| *x == "--spec-type").count(), 1);
    assert_eq!(dup_cmd.iter().filter(|x| *x == "--spec-draft-n-max").count(), 1);
    assert!(dup_cmd.contains(&"--verbose".to_string()));

    let alias_profile = Profile {
        name: "mtp-alias".into(),
        model_path: "/fake/model.gguf".into(),
        enable_mtp: true,
        spec_draft_n_max: 4,
        advanced_favorites: vec!["-st".into(), "-sdn".into(), "--verbose".into()],
        advanced_values: HashMap::from([
            ("-st".into(), "draft-mtp".into()),
            ("-sdn".into(), "9".into()),
            ("--verbose".into(), "1".into()),
        ]),
        ..Profile::default()
    };
    let options = HashMap::from([
        (
            "--spec-type".to_string(),
            LlamaOption {
                key: "--spec-type".into(),
                aliases: vec!["-st".into()],
                arity: 1,
                default_value: String::new(),
                description: String::new(),
                positive_flag: String::new(),
                negative_flag: String::new(),
            },
        ),
        (
            "--spec-draft-n-max".to_string(),
            LlamaOption {
                key: "--spec-draft-n-max".into(),
                aliases: vec!["-sdn".into()],
                arity: 1,
                default_value: String::new(),
                description: String::new(),
                positive_flag: String::new(),
                negative_flag: String::new(),
            },
        ),
    ]);
    let alias_cmd = command::build_command(Path::new("/fake/server.exe"), &alias_profile, &options)
        .expect("build command alias dedup");
    assert_eq!(alias_cmd.iter().filter(|x| *x == "--spec-type").count(), 1);
    assert_eq!(alias_cmd.iter().filter(|x| *x == "--spec-draft-n-max").count(), 1);
    assert!(alias_cmd.contains(&"--verbose".to_string()));
}

#[tokio::test]
async fn put_profile_mtp_fields_roundtrip() {
    let ts = boot_server(Some(vec![Profile { name: "original".into(), ..Profile::default() }])).await;
    let put = ts
        .client
        .put(format!("{}/api/profiles/0", ts.base))
        .json(&json!({ "enable_mtp": true, "spec_draft_n_max": 6 }))
        .send()
        .await
        .expect("put mtp");
    assert_eq!(put.status(), StatusCode::OK);
    let put_body: Value = put.json().await.expect("put mtp json");
    assert_eq!(put_body["enable_mtp"], true);
    assert_eq!(put_body["spec_draft_n_max"], 6);

    let get = ts.client.get(format!("{}/api/profiles/0", ts.base)).send().await.expect("get mtp");
    assert_eq!(get.status(), StatusCode::OK);
    let get_body: Value = get.json().await.expect("get mtp json");
    assert_eq!(get_body["enable_mtp"], true);
    assert_eq!(get_body["spec_draft_n_max"], 6);
}

#[test]
fn malformed_advanced_fields_resilience() {
    let tmp = tempfile::tempdir().expect("temp dir");
    write_raw_profiles_json(
        tmp.path(),
        json!([
            {
                "name": "malformed-favs",
                "enable_mtp": true,
                "spec_draft_n_max": 2,
                "advanced_favorites": "--spec-type",
                "advanced_values": {}
            },
            {
                "name": "malformed-vals",
                "enable_mtp": true,
                "spec_draft_n_max": 2,
                "advanced_favorites": [],
                "advanced_values": "not-a-dict"
            }
        ]),
    );
    let svc = LlamaLauncherService::new(Some(tmp.path().to_path_buf()));
    let profiles = svc.load_profiles();
    assert_eq!(profiles.len(), 2);
    assert!(profiles[0].enable_mtp);
    assert!(profiles[0].advanced_favorites.is_empty());
    assert!(profiles[1].enable_mtp);
    assert!(profiles[1].advanced_values.is_empty());
}
