# Story 6 — HTTP API server (launch, logs, monitoring, options)

**Depends on:** Story 5

Add the remaining API endpoints: process lifecycle, log tailing, monitoring, and options discovery.

### Scope
Extend `src/server.rs` with:
- `GET /api/status` → running + pid
- `POST /api/launch` → assemble command and launch server
- `POST /api/stop` → stop server
- `POST /api/restart` → stop + launch
- `GET /api/logs?last_size=N&last_marker=...` → tail with markers
- `GET /api/monitoring` → RAM, VRAM, process RAM
- `GET /api/options` → parse `--help` from configured exe
- `GET /api/models` → scan `.gguf` files

### Files
| Action | Path |
|--------|------|
| Modify | `rust-version/src/server.rs` |

### Acceptance criteria
- ✅ `POST /api/launch` returns `{"pid": N, "command": [...]}` with correct command list.
- ✅ `GET /api/status` reflects actual running state.
- ✅ `POST /api/stop` kills the process and cleans up PID file.
- ✅ `GET /api/logs` returns chunks with correct `last_size`, `reset`, and `last_marker`.
- ✅ `GET /api/monitoring` returns RAM/VRAM/process RAM with human-readable strings.
- ✅ `GET /api/options` returns parsed option map from `llama-server --help`.
- ✅ `GET /api/models` returns list of `.gguf` paths.

### Verification
```powershell
cd rust-version
cargo test
```
