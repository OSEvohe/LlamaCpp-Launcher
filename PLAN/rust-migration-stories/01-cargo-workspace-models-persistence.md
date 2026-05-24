# Story 1 — Cargo workspace + model structs + persistence

**Depends on:** nothing

Create the `rust-version/` Cargo workspace, define all data model structs with serde derives, and implement JSON persistence with identical schema to Python.

### Scope
- `Cargo.toml` with dependencies: `serde`, `serde_json`, `tokio`, `axum`, `tower-http`, `winapi`, `clap`, `regex`, `reqwest` (tests).
- `src/models.rs` — `GlobalSettings`, `Profile`, `LlamaOption` structs with `#[derive(Serialize, Deserialize, Debug, Clone)]` and default values matching Python dataclasses exactly.
- `src/config.rs` — `load_global()`, `save_global()`, `load_profiles()`, `save_profiles()` reading/writing `.launcher/global.json` and `.launcher/profiles.json`. Include `_normalize_mtp` legacy migration logic.
- `rust-version/.gitignore`.

### Files
| Action | Path |
|--------|------|
| Create | `rust-version/Cargo.toml` |
| Create | `rust-version/.gitignore` |
| Create | `rust-version/src/main.rs` |
| Create | `rust-version/src/models.rs` |
| Create | `rust-version/src/config.rs` |

### Acceptance criteria
- ✅ `cargo build` succeeds.
- ✅ `cargo test` succeeds (empty or trivial tests).
- ✅ Model round-trip: serialize a `Profile` → JSON string → deserialize → struct equals original.
- ✅ JSON output for a default `GlobalSettings` is byte-identical to Python `asdict(GlobalSettings())` (key names, types, defaults).
- ✅ `load_profiles()` on an empty directory returns a default profile list.
- ✅ Legacy MTP migration: raw JSON with `--spec-type` in advanced settings produces `enable_mtp: true` after load.

### Verification
```powershell
cd rust-version
cargo build
cargo test
```

### Non-goals
- No HTTP server. No process management. No CLI flags yet.
