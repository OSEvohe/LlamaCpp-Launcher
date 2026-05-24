# Story 5 — HTTP API server (profile + settings routes)

**Depends on:** Story 4

Wire up the axum HTTP server with all profile and settings CRUD endpoints.

### Scope
- `src/server.rs` — axum router with:
  - `GET /api/profiles` → list profiles
  - `GET /api/profiles/:index` → single profile
  - `POST /api/profiles` → create profile
  - `PUT /api/profiles/:index` → update profile
  - `DELETE /api/profiles/:index` → delete profile
  - `POST /api/profiles/:index/duplicate` → duplicate profile
  - `GET /api/settings` → global settings
  - `PUT /api/settings` → update global settings (with validation)
- Shared state: `Arc<RwLock<LlamaLauncherService>>` via axum `State`.
- 1 MB body limit enforcement.
- Identical JSON schemas and HTTP status codes (200, 201, 400, 404, 413).

### Files
| Action | Path |
|--------|------|
| Create | `rust-version/src/server.rs` |
| Modify | `rust-version/src/main.rs` |

### Acceptance criteria
- ✅ Server starts and binds to `127.0.0.1:0` (ephemeral port) in tests.
- ✅ All profile CRUD endpoints return correct status codes and JSON.
- ✅ `PUT /api/settings` validates `api_host` (string) and `api_port` (0–65535 integer).
- ✅ Oversized body returns 413.
- ✅ Invalid JSON body returns 400.
- ✅ Unknown routes return 404.

### Verification
```powershell
cd rust-version
cargo test
```

### Non-goals
- No launch/stop/restart, logs, monitoring, or options endpoints yet.
