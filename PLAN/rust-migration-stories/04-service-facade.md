# Story 4 — Service facade

**Depends on:** Story 3

Create the `LlamaLauncherService` struct that ties all modules together with thread-safe access.

### Scope
- `src/service.rs` — `LlamaLauncherService` struct wrapping all state (paths, lock). Implement all methods from Python `api.py`: `load_profiles()`, `save_profiles()`, `add_profile()`, `delete_profile()`, `duplicate_profile()`, `update_profile()`, `load_global()`, `save_global()`, `update_global()`, `load_options()`, `discover_models()`, `build_command()`, `is_server_running()`, `launch()`, `stop()`, `restart()`, `get_ram_usage()`, `get_process_ram()`, `get_gpu_vram()`, `format_bytes()`, `tail_log()`, `build_monitoring_text()`, `canonical_adv_key()`, `favorite_string_value()`.
- Use `std::sync::RwLock` or `tokio::sync::RwLock` for read-modify-write safety.

### Files
| Action | Path |
|--------|------|
| Create | `rust-version/src/service.rs` |
| Modify | `rust-version/src/main.rs` |

### Acceptance criteria
- ✅ `LlamaLauncherService` is constructible and all methods compile.
- ✅ Profile CRUD operations are correct and thread-safe (no data races under `tokio::test`).
- ✅ `update_profile()` with partial data preserves unchanged fields.
- ✅ `launch()` → `is_server_running()` → `stop()` cycle works end-to-end with a dummy process.
- ✅ Coercion of `top_k`, `min_p`, `presence_penalty`, `np`, `enable_mtp`, `spec_draft_n_max` matches Python behavior.

### Verification
```powershell
cd rust-version
cargo test
```
