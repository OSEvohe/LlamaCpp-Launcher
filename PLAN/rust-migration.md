# Rust Migration Plan — LLama Launcher

## Goal
Rewrite the LLama Launcher as a fully native Rust application under `rust-version/`, achieving 100% behavioral parity with the current Python implementation while preserving the `.launcher/` state contract, API surface, and dashboard.

## Constraints
- **Windows-only** — no cross-platform portability required.
- **Coexistence** — Python remains source of truth until Rust is fully verified. Both apps share the same `.launcher/` state directory.
- **API parity** — every REST endpoint, status code, and JSON schema must match.
- **State compatibility** — `global.json`, `profiles.json`, PID/log files use identical paths and JSON schema.
- **Dashboard** — `dashboard.html` is served verbatim; no frontend changes.
- **No opportunistic refactors** — the Rust version mirrors current behavior, including quirks.

## Non-Goals
- Cross-platform support (Linux/macOS).
- API redesign or versioning.
- Dashboard rewrite.
- Performance benchmarking (parity, not optimization, is the target).

**Implementation stories:** [`rust-migration-stories.md`](rust-migration-stories.md) (13 ordered stories)

---

## Phases

### Phase 0 — Scaffolding & Foundation
**Deliverable:** `rust-version/` Cargo workspace, build succeeds, `cargo test` passes (empty).

- Create `rust-version/` with `Cargo.toml` workspace.
- Pick dependency set:
  - `serde` + `serde_json` — data models + persistence
  - `tokio` — async runtime (process spawning, HTTP)
  - `axum` — HTTP server (closest match to Python `ThreadingHTTPServer` routing)
  - `winapi` — Windows process/monitoring APIs
  - `clap` — CLI argument parsing
  - `tower-http` — static file serving (dashboard)
- Define `src/models.rs` structs mirroring `GlobalSettings`, `Profile`, `LlamaOption` with `#[derive(Serialize, Deserialize)]` and identical default values.
- Define `src/config.rs` persistence helpers reading/writing `.launcher/global.json` and `.launcher/profiles.json` with the same JSON schema.
- Add `rust-version/.gitignore` for `.launcher/` and build artifacts.

**Verification:** `cargo build` and `cargo test` succeed. Model round-trip tests (serialize → deserialize) produce identical JSON to Python `asdict()`.

---

### Phase 1 — Core Business Logic
**Deliverable:** All pure-logic modules ported and tested in isolation.

| Python module | Rust module | Notes |
|---|---|---|
| `models.py` | `src/models.rs` | Done in Phase 0 |
| `config.py` | `src/config.rs` | Include MTP legacy migration logic |
| `discovery.py` | `src/discovery.rs` | `.gguf` file scanning |
| `options.py` | `src/options.rs` | `--help` parsing, `subprocess` → `std::process::Command` |
| `command.py` | `src/command.rs` | Command-line assembly, MTP dedup |

- Port the `_normalize_mtp` legacy migration from `config.py`.
- Port `parse_help_options` regex logic (use `regex` crate).
- Port `build_command` including `shlex`-style splitting (use `shlex` crate or manual parser for Windows).
- Port `canonical_adv_key` and `favorite_string_value`.

**Verification:** Unit tests for every function. JSON persistence round-trips match Python output byte-for-byte for known fixtures.

---

### Phase 2 — Process Lifecycle & Monitoring
**Deliverable:** Process management and system monitoring working on Windows.

| Python module | Rust module | Notes |
|---|---|---|
| `process.py` | `src/process.rs` | `tasklist`/`taskkill` → `std::process::Command` or direct WinAPI |
| `monitoring.py` | `src/monitoring.rs` | `GlobalMemoryStatusEx` via `winapi`, `nvidia-smi` via subprocess |

- `start_server()`: Use `std::process::Command` with `CREATE_BREAKAWAY_FROM_JOB \| CREATE_NEW_PROCESS_GROUP` creation flags (same as Python: `0x08000000 \| 0x00000200`).
- `is_process_running()` / `find_llama_server_pid()`: Either keep `tasklist` subprocess calls or use `winapi::psapi::EnumProcesses` + `winapi::psapi::GetModuleFileNameExW` for name matching.
- `stop_server()`: `taskkill /F /T` via subprocess or `winapi::processthreadsapi::TerminateProcess` + job object for tree kill.
- `ram_usage_bytes()`: `GlobalMemoryStatusEx` via `winapi`.
- `gpu_vram_info()`: `nvidia-smi` subprocess parsing (keep as-is; no WMI/WinAPI alternative needed).
- `tail_log_chunk()`: File-reading with marker-based rewrite detection.
- `bytes_to_gb()` / `build_monitoring_text()`: Trivial port.

**Verification:** Integration tests that spawn a dummy process, verify PID file, check running state, and kill it. RAM/VRAM monitoring returns plausible values (or `(0,0)` when hardware is absent).

---

### Phase 3 — Service Facade & HTTP API
**Deliverable:** Full REST API server matching Python endpoint-for-endpoint.

| Python module | Rust module | Notes |
|---|---|---|
| `api.py` | `src/service.rs` | `LlamaLauncherService` struct with `RwLock` |
| `server.py` | `src/server.rs` | axum router with identical routes |

- Port `LlamaLauncherService` as an `Arc<RwLock<...>>` or `Arc<Mutex<...>>` shared state.
- Implement every route from `server.py`:
  - `GET /` → serve `dashboard.html`
  - `GET /api/status` → running + pid
  - `GET/POST/PUT/DELETE /api/profiles[/:index]` → CRUD
  - `POST /api/profiles/:index/duplicate`
  - `GET/PUT /api/settings`
  - `GET /api/options` → parse `--help`
  - `GET /api/models` → scan `.gguf`
  - `POST /api/launch`, `/api/stop`, `/api/restart`
  - `GET /api/logs` → tail with markers
  - `GET /api/monitoring` → RAM/VRAM/process RAM
- 1 MB body limit enforcement.
- Identical JSON response schemas and HTTP status codes.

**Verification:** Run the Rust server alongside the Python server on different ports. Send the same requests to both and diff JSON responses. Port the Python API endpoint tests (`test_api_endpoints.py`) to Rust using `reqwest`.

---

### Phase 4 — CLI & Auto-Start
**Deliverable:** CLI flags, scheduled task, and Windows service installation.

| Python module | Rust module | Notes |
|---|---|---|
| `main.py` | `src/main.rs` | `clap` for `--api-host`, `--api-port`, task/service flags |
| `service.py` | `src/service_install.rs` | `schtasks` for scheduled task; Windows SCM for native service |

- CLI: `--api-host`, `--api-port`, `--install-task`, `--uninstall-task`, `--install-service`, `--uninstall-service`.
- Scheduled task: `schtasks /create` via subprocess (same as Python).
- Windows service: Use `windows-service` crate or `winapi` + `RegisterServiceCtrlHandlerExW` for SCM integration. This is the hardest part — may require a separate binary or installer helper.

**Verification:** `--install-task` creates a task visible in `taskschd.msc`. `--uninstall-task` removes it. Native service install tested with admin rights.

---

### Phase 5 — Dashboard & Static Assets
**Deliverable:** Dashboard served identically from the Rust binary.

- Copy `llama_launcher/static/dashboard.html` into `rust-version/static/dashboard.html`.
- Serve via `axum::serve` with `tower_http::services::ServeDir` or embed with `include_str!` / `rust-embed`.
- The dashboard is purely client-side (fetches JSON from `/api/*`), so no changes needed.

**Verification:** Open `http://127.0.0.1:<port>/` against Rust server — dashboard loads and all buttons work.

---

### Phase 6 — Testing & Parity Verification
**Deliverable:** Comprehensive test suite proving behavioral parity.

- Port all tests from `tests/test_api_endpoints.py` (~35 tests) to Rust.
- Add integration tests covering:
  - Launch → status → stop → restart cycle.
  - Profile CRUD round-trips.
  - Settings persistence.
  - Log tailing with markers.
  - Concurrency (simultaneous requests).
  - MTP field normalization and legacy migration.
- Run a **comparison harness**: start both Python and Rust servers, send identical requests, assert responses are byte-identical.

**Verification:** 100% of ported tests pass. Comparison harness shows zero diffs.

---

### Phase 7 — Cutover Preparation
**Deliverable:** Build artifacts, installer, and rollback plan.

- Build release binary: `cargo build --release` → single `llama-launcher.exe`.
- Create a side-by-side comparison script that validates Rust output against Python.
- Document the cutover procedure (see below).

**Story 12 status (current workspace):**
- Cutover prep is documented, but release verification is currently blocked by missing Rust tooling in this environment.
- `cargo build --release` failed: `cargo` command not found.
- `cargo test --all-targets` failed: `cargo` command not found.
- `./target/release/llama-launcher.exe --api-port 0` failed: binary not found at `rust-version/target/release/llama-launcher.exe`.

---

## Verification Strategy

| Gate | Method |
|---|---|
| Model round-trip | Serialize/deserialize JSON, diff against Python `asdict()` output |
| API endpoint parity | Parallel Python + Rust servers, same requests, diff responses |
| Process lifecycle | Spawn `llama-server.exe` (or dummy), verify PID file, stop, verify cleanup |
| State compatibility | Write state with Python, read with Rust (and vice versa) |
| Dashboard | Visual inspection + automated HTML fetch |
| Concurrency | Simultaneous requests to Rust server (port `test_concurrency.py`) |

## Rollout / Cutover

1. **Side-by-side** — Install Rust binary alongside Python. Both read the same `.launcher/` directory.
2. **Shadow mode** — Run Rust server on a different port. User accesses Rust dashboard; Python server is idle.
3. **Task swap** — Update the Windows scheduled task to point to `llama-launcher.exe` instead of `pythonw.exe main.py`.
4. **Rollback** — Revert the scheduled task to the Python path. Python can still read any state written by Rust (identical JSON schema).

### Rollback Checklist (Rust -> Python)

1. Stop the Rust process (`llama-launcher.exe`) and confirm `/api/status` reports not running.
2. Re-point the Windows scheduled task action from Rust binary back to `pythonw.exe main.py`.
3. Start the scheduled task and verify Python dashboard/API responds on the configured host/port.
4. Validate `.launcher/global.json` and `.launcher/profiles.json` remain readable with expected profile/settings values.
5. Run one lifecycle smoke test via API (`/api/launch` -> `/api/status` -> `/api/stop`) to confirm operational recovery.

## Risks & Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Windows service (SCM) integration is complex | High — may delay Phase 4 | Use `windows-service` crate; fall back to scheduled-task-only if SCM proves too difficult |
| `taskkill /F /T` tree-kill semantics differ from WinAPI | Medium — orphaned child processes | Keep `taskkill` subprocess call (proven behavior) rather than rewriting with raw WinAPI |
| Dashboard HTML has Python-specific assumptions | Low — unlikely, but possible | Audit dashboard JS for hardcoded paths or Python-specific responses |
| State file corruption during coexistence | Medium — concurrent writes | Enforce single-writer discipline: only one app writes to `.launcher/` at a time during transition |
| `nvidia-smi` not available on all machines | Low — graceful degradation already exists | Preserve `(0, 0)` fallback |
| Build size / distribution | Low — single binary | `cargo build --release` produces ~10-20 MB binary; acceptable for this use case |

## Open Questions

1. **Native Windows service (SCM):** Should the Rust version support the native SCM service path (`--install-service`), or is scheduled-task-only sufficient? The Python version supports both, but SCM requires admin rights and is less commonly used.
2. **Binary distribution:** Should the release be a single portable `.exe`, or should we produce an installer (NSIS/Inno Setup)?
3. **Python retirement:** After cutover, should the Python code be archived in the repo (e.g., `python-legacy/`) or removed entirely?
4. **Logging:** The Python app writes `service.log` and `service-crash.log` when running as a service. Should the Rust version replicate this, or use Windows Event Log?
