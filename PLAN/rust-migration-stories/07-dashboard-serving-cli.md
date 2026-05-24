# Story 7 — Dashboard serving + CLI entry point

**Depends on:** Story 6

Serve the dashboard HTML and implement the full CLI with `clap`.

### Scope
- Copy `llama_launcher/static/dashboard.html` → `rust-version/static/dashboard.html`.
- `GET /` → serve dashboard (via `tower_http::services::ServeFile` or `rust-embed`).
- `src/main.rs` — `clap` CLI: `--api-host`, `--api-port`, `--install-task`, `--uninstall-task`, `--install-service`, `--uninstall-service`.
- Resolve host/port from persisted settings (same fallback logic as Python: settings → defaults → 7890).

### Files
| Action | Path |
|--------|------|
| Create | `rust-version/static/dashboard.html` |
| Modify | `rust-version/src/main.rs` |
| Modify | `rust-version/src/server.rs` |

### Acceptance criteria
- ✅ `GET /` returns the dashboard HTML with `Content-Type: text/html`.
- ✅ `--api-host` and `--api-port` CLI flags override persisted settings.
- ✅ Default port is 7890 when no settings are configured.
- ✅ `--api-port 0` (ephemeral) is preserved as-is.
- ✅ Dashboard loads in browser and all buttons work against the Rust API.

### Verification
```powershell
cd rust-version
cargo run -- --api-port 0
# Open http://127.0.0.1:<port>/ in browser
```
