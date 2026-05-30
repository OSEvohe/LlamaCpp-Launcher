# CODE: llama.cpp Version Management — Steps 1+2 Complete, Step 3 Next

## Goal
- Implement llama.cpp version management: state/registry (Step 1, complete) and GitHub catalog + install lifecycle (Step 2, complete).

## Constraints & Preferences
- Edit only scoped files plus new `versions/` module; maintain backward compatibility for existing `.launcher/global.json`.
- No API redesign; provide clear recoverable errors for stale/missing active versions.
- Keep fallback to `llama_server_path` when no active version is set.

## Progress
### Done
- Step 1 (state & registry): `InstalledVersion`, `VersionStatus`, `GlobalSettings` extension, service methods (`list_installed_versions`, `register_installed_version`, `unregister_installed_version`, `set_active_version`, `resolve_active_executable`), backward-compat deserialization, `ensure_state` made public. **All 200 unit tests pass.**
- Step 2 (GitHub catalog + install lifecycle): Created `versions/` module (`github.rs`, `installer.rs`, `mod.rs`).
  - GitHub client with TTL cache (5 min), timeout, rate-limit/offline error types.
  - Asset filtering for Windows `llama-server.exe` binaries (prefers CPU over CUDA builds).
  - Install flow: download with progress, extract zip, validate `llama-server.exe`, move to install dir, register metadata atomically.
  - Delete flow with guards: blocks removal of active version.
  - `InstallState`/`InstallPhase` models and `install_states` field to `GlobalSettings` for in-progress tracking.
  - Service methods: `fetch_available_versions`, `get_install_state`, `start_install_version`, `cancel_install`, `uninstall_version`.
  - `versions_dir` field to `LlamaLauncherService` (`.launcher/versions/`).
  - Helper functions: `copy_dir_all`, `find_exe_in_dir`.
  - Updated `Cargo.toml` with `reqwest`, `zip`, `flate2`, `futures`, `chrono` dependencies.
  - Updated `test_default_global_settings_json_shape` to expect `installed_versions: []`.
  - 12 new unit tests for uninstall/cancel/state. **All 212 unit tests pass.**

### In Progress
- (none)

### Blocked
- (none)

## Key Decisions
- Used `#[serde(default)]` and `#[serde(skip_serializing_if = "...")]` for backward-compatible JSON handling.
- `resolve_active_executable` resolution order: 1) Active version executable (check existence), 2) `llama_server_path` fallback, 3) Error.
- Unregistering a version automatically clears `active_version` if it matches the removed tag.
- Install progress persisted to `global.json` via direct file writes during async download (avoids holding RwLock during long operations).
- GitHub releases cached in-process with 5-minute TTL via `Mutex<ReleaseCache>`.
- Prefers CPU builds over CUDA/Vulkan/ROCm for `find_windows_asset`.
- Cross-device move fallback: copy then delete when `std::fs::rename` fails.

## Next Steps
- Proceed to Step 3: API dashboard endpoints and tests.

## Verification
- `cargo build` succeeds with minor unused import warnings (`Body` in `server.rs`, `std::io::Write` in `process.rs`, `marker` in `log_tailer.rs`).
- `cargo test` — **212 passed; 0 failed** (unit tests). One pre-existing flaky integration test (`launch_status_stop_restart_dummy_lifecycle_with_pid_cleanup`) unrelated to these changes.
- 14 API integration tests pass.

## Relevant Files
- `rust-version/src/models.rs`: Added `GitHubRelease`, `GitHubReleaseAsset`, `InstallState`, `InstallPhase`, `VersionInfo`; extended `GlobalSettings` with `install_states` HashMap.
- `rust-version/src/config.rs`: Updated `load_global` to initialize `install_states`, updated `test_default_global_settings_json_shape` expected output.
- `rust-version/src/service.rs`: Added `versions_dir` field, version management methods, helper functions, updated `update_global` to preserve `install_states`.
- `rust-version/src/versions/mod.rs`: Module layout and re-exports.
- `rust-version/src/versions/github.rs`: GitHub releases client with cache, timeout, rate-limit errors, asset filtering.
- `rust-version/src/versions/installer.rs`: Download with progress, zip extraction, `llama-server.exe` validation, cleanup helpers.
- `rust-version/src/server.rs`: Fixed test initializers with `..GlobalSettings::default()`.
- `rust-version/Cargo.toml`: Added `reqwest`, `zip`, `flate2`, `futures`, `chrono` dependencies.
- `PLAN/llama-cpp-version-management/01-state-and-version-registry.md`: Step 1 spec (complete).
- `PLAN/llama-cpp-version-management/02-github-catalog-and-install-lifecycle.md`: Step 2 spec (complete).
- `PLAN/llama-cpp-version-management/03-api-dashboard-and-tests.md`: Step 3 spec (next).
