# Web API + Headless Mode — LLama Launcher

## Summary

Add a lightweight HTTP API server to LLama Launcher that exposes every launcher capability (launch, stop, restart, profile management, settings, model discovery, logs, monitoring) over REST. When the API server is active, the launcher can run in **headless mode** — no TUI, just the API. A minimal HTML/JS web dashboard is served alongside the API for browser-based control.

**Why it matters:** Enables remote control of llama-server from any HTTP client, scripting automation, and headless deployment (servers, CI, containers). The existing TUI remains completely unaffected.

**High-level approach:** Build a `ThreadingHTTPServer` (stdlib) with JSON REST routes that delegate directly into the existing `LlamaLauncherService`. A new `--headless` / `--api-port` CLI flag bypasses the TUI and runs the API server on its own event loop. A small embedded HTML dashboard provides a visual control surface.

---

## Implementation Stories

### Story 1 — `models.py`: API configuration dataclass

**Depends on:** nothing

Add `ApiConfig` dataclass to hold host, port, and enabled state. Extend `GlobalSettings` with optional API fields.

| Action | Path |
|--------|------|
| Modify | `llama_launcher/models.py` |

**Changes:**
```python
@dataclass
class ApiConfig:
    host: str = "127.0.0.1"
    port: int = 0  # 0 = disabled
```

Extend `GlobalSettings`:
```python
@dataclass
class GlobalSettings:
    llama_server_path: str = ""
    model_dirs: List[str] = None
    api_host: str = "127.0.0.1"
    api_port: int = 0
```

**Acceptance criteria:**
- `GlobalSettings` round-trips through JSON with new fields (defaults to `api_port=0` meaning disabled)
- Existing `.launcher/global.json` files load without error (missing fields get defaults)

---

### Story 2 — `server.py`: HTTP API server module

**Depends on:** Story 1

Create the HTTP server module using `http.server.ThreadingHTTPServer` (stdlib). This is the core of the feature.

| Action | Path |
|--------|------|
| Create | `llama_launcher/server.py` |

**Design decisions:**
- **Framework:** `http.server.ThreadingHTTPServer` — zero new dependencies, matches the project's stdlib-first constraint. `ThreadingHTTPServer` handles concurrent requests (important for log polling + control operations).
- **Protocol:** JSON REST, `Content-Type: application/json`.
- **Service lifecycle:** The server holds a single `LlamaLauncherService` instance, created at startup. All routes delegate to it. No per-request service creation.
- **Static files:** Embedded HTML/JS served from `__init__.py` string constants (no separate file to manage), or from a `llama_launcher/static/` directory if the dashboard grows.

**REST Endpoints:**

| Method | Path | Description | Response |
|--------|------|-------------|----------|
| `GET` | `/api/status` | Server health + llama-server running state | `{running, pid, api_host, api_port}` |
| `GET` | `/api/profiles` | List all profiles | `{profiles: [{…}]}` |
| `GET` | `/api/profiles/:index` | Get single profile | profile dict |
| `POST` | `/api/profiles` | Create profile | new profile dict |
| `PUT` | `/api/profiles/:index` | Update profile | updated profile dict |
| `DELETE` | `/api/profiles/:index` | Delete profile | `{ok: true}` |
| `GET` | `/api/settings` | Global settings | settings dict |
| `PUT` | `/api/settings` | Update global settings | settings dict |
| `GET` | `/api/options` | Load llama-server options | `{options: {key: {…}}}` |
| `GET` | `/api/models` | Discover .gguf models | `{models: [str, …]}` |
| `POST` | `/api/launch` | Launch llama-server | `{pid: int}` |
| `POST` | `/api/stop` | Stop llama-server | `{ok: true, pid: int}` |
| `POST` | `/api/restart` | Stop then launch | `{pid: int}` |
| `GET` | `/api/logs` | Tail log output | `{chunk: str, size: int}` |
| `GET` | `/api/monitoring` | RAM/VRAM stats | `{ram_used, ram_total, vram_used, vram_total}` |
| `GET` | `/` | Web dashboard | HTML page |

**Module structure:**
```python
class ApiHandler(http.server.BaseHTTPRequestHandler):
    """Route handler — dispatches to service methods."""
    service: LlamaLauncherService  # set by server factory

    def do_GET(self) -> None: ...
    def do_POST(self) -> None: ...
    def do_PUT(self) -> None: ...
    def do_DELETE(self) -> None: ...

def create_api_server(
    service: LlamaLauncherService,
    host: str,
    port: int,
) -> http.server.ThreadingHTTPServer: ...

def run_api_server(service: LlamaLauncherService, host: str, port: int) -> None:
    """Block until interrupted. The headless main loop."""
    ...
```

**Acceptance criteria:**
- `create_api_server` returns a configured `ThreadingHTTPServer`
- All endpoints return valid JSON with correct HTTP status codes
- `/api/status` returns 200 with running/pid info
- Error responses use `{error: "message"}` with 4xx/5xx status
- No `textual` imports in `server.py`

---

### Story 3 — `main.py`: CLI flags and headless mode

**Depends on:** Story 2

Add `--headless` and `--api-port` / `--api-host` CLI arguments. When headless mode is active, skip the TUI and run the API server directly.

| Action | Path |
|--------|------|
| Modify | `llama_launcher/main.py` |

**CLI design:**
```
python main.py                          # TUI mode (unchanged)
python main.py --headless               # headless, uses global.json api_host/api_port
python main.py --headless --api-port 7890  # headless with explicit port
python main.py --api-port 7890          # TUI + API sidecar (both active)
```

**Implementation:**
```python
def main() -> None:
    import argparse
    parser = argparse.ArgumentParser(description="LLama Launcher")
    parser.add_argument("--headless", action="store_true", help="Run without TUI")
    parser.add_argument("--api-port", type=int, default=None)
    parser.add_argument("--api-host", type=str, default=None)
    args = parser.parse_args()

    service = LlamaLauncherService()
    settings = service.load_global()

    api_host = args.api_host or settings.api_host or "127.0.0.1"
    api_port = args.api_port or settings.api_port or 0

    if args.headless:
        if api_port <= 0:
            api_port = 7890  # sensible default for headless
        print(f"LLama Launcher API: {api_host}:{api_port}")
        run_api_server(service, api_host, api_port)
    else:
        # TUI mode
        if api_port > 0:
            # Start API in background thread, then launch TUI
            ...
        from llama_launcher.ui.app import LlamaLauncherApp
        LlamaLauncherApp().run()
```

**Acceptance criteria:**
- `python main.py` launches the TUI exactly as before (zero behavior change)
- `python main.py --headless` starts the API server, prints the URL, blocks
- `python main.py --headless --api-port 9999` binds to port 9999
- TUI + API sidecar mode works: both TUI and API are active simultaneously

---

### Story 4 — Web dashboard

**Depends on:** Story 2

Minimal HTML/JS frontend served at `/`. Single-page application that communicates with the REST API.

| Action | Path |
|--------|------|
| Create | `llama_launcher/static/dashboard.html` |

**Dashboard views (tabs):**
1. **Status** — server running state, PID, RAM/VRAM, launch/stop/restart buttons
2. **Profiles** — list, create, edit, delete profiles (form matching TUI fields)
3. **Settings** — global settings editor (server path, model dirs, API config)
4. **Logs** — live-tailing log viewer (polls `/api/logs` every 500ms)
5. **Models** — discovered model list

**Technical choices:**
- Single HTML file, no build step, no JS framework
- Vanilla JS with `fetch()` for API calls
- CSS: minimal inline styles, dark theme matching the TUI aesthetic
- Auto-refresh for status and logs via `setInterval`
- Served as static file from `llama_launcher/static/`

**Acceptance criteria:**
- `GET /` returns the HTML dashboard
- Dashboard can launch/stop/restart llama-server
- Dashboard can view and edit profiles
- Log viewer auto-refreshes
- Responsive enough for desktop browser use

---

### Story 5 — Concurrency and state safety

**Depends on:** Stories 2-4

Address the shared mutable state between API handlers and TUI when both are active.

**Problem:** `LlamaLauncherService` reads/writes JSON files and manages process state. When the TUI and API server share the same service instance, concurrent profile edits or launch/stop races are possible.

**Solution — file-level locking:**
- Add a `threading.Lock` inside `LlamaLauncherService` for write operations (`save_profiles`, `save_global`, `launch`, `stop`).
- Read operations remain lock-free (JSON reads are safe; stale reads are acceptable — the caller can retry).
- The lock is a `threading.RLock` (reentrant) to handle nested calls like `launch()` → `save_global()`.

| Action | Path |
|--------|------|
| Modify | `llama_launcher/api.py` |

**Changes to `LlamaLauncherService`:**
```python
def __init__(self, ...):
    ...
    self._lock = threading.RLock()

def save_profiles(self, profiles: List[Profile]) -> None:
    with self._lock:
        ...

def launch(self, cmd: list, exe_path: str = "") -> int:
    with self._lock:
        ...
```

**Acceptance criteria:**
- Concurrent API requests to `/api/profiles` and `/api/launch` do not corrupt JSON
- TUI + API sidecar mode: profile edits from the dashboard don't crash the TUI
- No deadlocks under normal usage

---

### Story 6 — Security hardening

**Depends on:** Story 2

Default security posture for the API server.

| Action | Path |
|--------|------|
| Modify | `llama_launcher/server.py` |
| Modify | `llama_launcher/main.py` |

**Measures:**
1. **Localhost-only by default:** `api_host` defaults to `127.0.0.1`. Binding to `0.0.0.0` requires explicit `--api-host 0.0.0.0`.
2. **No authentication (documented risk):** No auth in MVP. API is localhost-only by default. Add a `TODO` comment noting that auth (token, basic, or mTLS) should be added before exposing to a network.
3. **CORS:** No CORS headers by default (same-origin only). A `--api-cors` flag can be added later if cross-origin access is needed.
4. **Request size limit:** Reject requests with body > 1MB (prevents abuse via large payloads).
5. **No file traversal:** All API paths are under `/api/` or `/`. No path-based file access.

**Acceptance criteria:**
- Default binding is `127.0.0.1` only
- No CORS headers in responses
- Large request bodies are rejected with 413
- Security considerations are documented in code comments

---

### Story 7 — Configuration persistence

**Depends on:** Story 1

Make API host/port configurable through both CLI and global settings.

| Action | Path |
|--------|------|
| Modify | `llama_launcher/config.py` |
| Modify | `llama_launcher/api.py` (GlobalSettings load/save) |

**Changes:**
- `load_global()` in `config.py` already handles missing fields via `GlobalSettings()` defaults — no change needed if Story 1's defaults are correct.
- The web dashboard Settings tab can edit `api_host` and `api_port` via `PUT /api/settings`.
- Changing `api_port` at runtime does **not** restart the server (would require server recreation). Document this limitation.

**Acceptance criteria:**
- `api_host` and `api_port` persist across restarts in `.launcher/global.json`
- CLI `--api-port` overrides the stored value for the current session
- TUI can eventually edit API settings (deferred — API Settings tab in dashboard is sufficient for MVP)

---

### Story 8 — Testing strategy

**Depends on:** all previous stories

Verification plan for the feature.

**Test categories:**

| Category | Method | Scope |
|----------|--------|-------|
| **API endpoints** | `http.server` test with `urllib.request` | Each endpoint returns correct JSON, correct status codes |
| **Headless mode** | `subprocess` + `--headless` | Process starts, binds to port, responds to requests, exits on SIGINT |
| **TUI regression** | Manual launch | `python main.py` works identically to pre-feature state |
| **Concurrency** | Threaded requests | Concurrent profile edits don't corrupt JSON |
| **Security** | Manual verification | Default binding is localhost, no CORS headers |

**Test module:**
| Action | Path |
|--------|------|
| Create | `llama_launcher/test_api.py` |

**Test approach:**
- Use `unittest` (stdlib) with `http.server.ThreadingHTTPServer` started in a thread
- Each test creates a fresh temp directory for `.launcher/` state
- No external test framework needed (stdlib only)
- Tests can be run with `python -m llama_launcher.test_api`

**Acceptance criteria:**
- All API endpoints tested with at least one positive and one negative case
- Headless mode starts and responds within 5 seconds
- `python main.py` (TUI only) launches without errors
- JSON corruption test passes under concurrent load

---

## Open Questions / Risks

| # | Question / Risk | Impact | Status |
|---|-----------------|--------|--------|
| Q1 | Should the API server run in a thread alongside the TUI, or only in headless mode? | Architecture | **Decision:** Support both. Sidecar mode (TUI + API thread) for flexibility; headless mode as the primary use case. |
| Q2 | How to handle API server shutdown when TUI exits in sidecar mode? | UX | **Plan:** TUI `action_quit` stops the API thread before exiting. |
| Q3 | Is `ThreadingHTTPServer` sufficient or do we need async (aiohttp)? | Performance | **Assessment:** `ThreadingHTTPServer` is fine. llama-server operations are fast (file I/O, subprocess). No long-running blocking calls in handlers. |
| Q4 | Should we add authentication before exposing to non-localhost? | Security | **Plan:** Defer to post-MVP. Add `TODO` and document the risk. |
| R1 | JSON file corruption under concurrent writes | Data integrity | **Mitigation:** `threading.RLock` in Story 5. |
| R2 | Port already in use | UX | **Mitigation:** Catch `OSError` on bind, print error, exit with code 1. |
| R3 | Dashboard HTML grows too large for inline embedding | Maintainability | **Mitigation:** Use `llama_launcher/static/` directory from the start. |

---

## Phased Rollout

### Phase 1 — MVP (Stories 1-3, 6)
- API server with all REST endpoints
- Headless mode via `--headless`
- Security defaults (localhost, no CORS)
- **Deliverable:** Full programmatic control via curl/HTTP clients

### Phase 2 — Dashboard + Polish (Stories 4, 5, 7)
- Web dashboard at `/`
- Thread safety for TUI + API sidecar
- Configuration persistence for API settings
- **Deliverable:** Browser-based control surface

### Phase 3 — Testing + Hardening (Story 8)
- Test suite for API endpoints
- Concurrency tests
- TUI regression verification
- **Deliverable:** Confidence in production use

---

## File Inventory

| Action | Path | Purpose |
|--------|------|---------|
| Modify | `llama_launcher/models.py` | Add `ApiConfig` fields to `GlobalSettings` |
| Create | `llama_launcher/server.py` | HTTP API server + route handlers |
| Modify | `llama_launcher/main.py` | CLI flags, headless mode, sidecar mode |
| Create | `llama_launcher/static/dashboard.html` | Web dashboard |
| Modify | `llama_launcher/api.py` | Thread lock for write operations |
| Create | `llama_launcher/test_api.py` | API test suite |
| Modify | `requirements.txt` | No changes (stdlib only) |
