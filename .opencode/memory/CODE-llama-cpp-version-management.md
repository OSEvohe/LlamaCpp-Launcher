# CODE: llama.cpp Version Management — State and Registry (Step 1)

## Goal
- Implement version state management and registry for llama.cpp installs per `PLAN/llama-cpp-version-management/01-state-and-version-registry.md`.

## Constraints & Preferences
- Edit only scoped files (`models.rs`, `config.rs`, `service.rs`) plus minimal test updates.
- No API redesign; maintain backward compatibility for existing `.launcher/global.json`.
- Provide clear recoverable errors for stale/missing active versions.
- Keep fallback to `llama_server_path` when no active version is set.

## Progress
### Done
- Added `InstalledVersion` struct and `VersionStatus` enum to `models.rs`.
- Extended `GlobalSettings` with `installed_versions` (Vec) and `active_version` (Option) with backward-compatible `Deserialize` logic.
- Updated `config.rs` `load_global` to parse new fields safely.
- Implemented service methods in `service.rs`: `list_installed_versions`, `register_installed_version`, `unregister_installed_version`, `set_active_version`, `resolve_active_executable`.
- Updated `update_global` to merge new fields.
- Made `ensure_state` public for test use.
- Added comprehensive unit tests for version management and backward compatibility in `service.rs` and `config.rs`.
- Fixed all compilation errors and test failures. **All 200 unit tests pass.**

### In Progress
- (none)

### Blocked
- (none)

## Key Decisions
- Used `#[serde(default)]` and `#[serde(skip_serializing_if = "...")]` to ensure old `global.json` files load without errors and new JSON omits empty optional fields.
- `resolve_active_executable` resolution order: 1) Active version executable (check existence), 2) `llama_server_path` fallback, 3) Error.
- Unregistering a version automatically clears `active_version` if it matches the removed tag.
- `ensure_state` made public so tests can create `.launcher/` dir before writing files.

## Next Steps
- Commit changes.
- Proceed to Step 2: Download/extract logic.

## Verification
- `cargo test` — **200 passed; 0 failed** (unit tests). One pre-existing flaky integration test (`launch_status_stop_restart_dummy_lifecycle_with_pid_cleanup`) unrelated to these changes.

## Relevant Files
- `rust-version/src/models.rs`: Added `InstalledVersion`, `VersionStatus`, extended `GlobalSettings` and its `Deserialize` impl.
- `rust-version/src/config.rs`: Updated `load_global` parsing logic, added backward-compat tests, updated `test_default_global_settings_json_shape` expected output.
- `rust-version/src/service.rs`: Added version management methods, updated `update_global`, made `ensure_state` public, added extensive tests.
- `rust-version/src/server.rs`: Fixed test initializer with `..GlobalSettings::default()`.
- `PLAN/llama-cpp-version-management/01-state-and-version-registry.md`: Step specification.
