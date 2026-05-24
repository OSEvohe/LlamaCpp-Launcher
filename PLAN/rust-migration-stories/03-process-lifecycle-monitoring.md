# Story 3 — Process lifecycle + monitoring

**Depends on:** Story 2

Port Windows process management and system monitoring.

### Scope
- `src/process.rs` — `read_pid()`, `is_process_running()`, `start_server()`, `find_llama_server_pid()`, `stop_server()`. Use `std::process::Command` for `tasklist`/`taskkill` calls. Spawn with `CREATE_BREAKAWAY_FROM_JOB | CREATE_NEW_PROCESS_GROUP` creation flags.
- `src/monitoring.rs` — `ram_usage_bytes()` via `GlobalMemoryStatusEx` WinAPI, `gpu_vram_info()` via `nvidia-smi` subprocess, `process_ram_bytes()` via `tasklist`, `tail_log_chunk()` with marker-based rewrite detection, `bytes_to_gb()`, `build_monitoring_text()`.

### Files
| Action | Path |
|--------|------|
| Create | `rust-version/src/process.rs` |
| Create | `rust-version/src/monitoring.rs` |
| Modify | `rust-version/src/main.rs` |

### Acceptance criteria
- ✅ `start_server()` spawns a detached process and returns its PID.
- ✅ `is_process_running(pid)` correctly reports a live process.
- ✅ `stop_server(pid)` kills the process and its children.
- ✅ `read_pid()` / PID file round-trip works.
- ✅ `ram_usage_bytes()` returns non-zero `(used, total)` on a machine with RAM.
- ✅ `gpu_vram_info()` returns `(0, 0)` when `nvidia-smi` is absent; returns plausible values when present.
- ✅ `tail_log_chunk()` detects append, truncation, and equal-size rewrite correctly (marker logic).

### Verification
```powershell
cd rust-version
cargo test
```

### Non-goals
- No HTTP server yet. Process tests may use a no-op executable (e.g., `ping -n 30 127.0.0.1`) instead of real `llama-server.exe`.
