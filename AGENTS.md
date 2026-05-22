# LLama Launcher — Agent Notes

## Quick start

```powershell
pip install textual
python main.py              # TUI mode
python main.py --headless   # API-only (port 7890 default)
```

## Running tests

No `pyproject.toml` or `setup.py` — the package is **not pip-installable**. Tests require `PYTHONPATH` pointing to the repo root.

No test framework required — every test file has its own `if __name__ == "__main__"` runner.

```powershell
$env:PYTHONPATH = "C:\Users\Shadow\Documents\LLama Launcher"

# All tests
python tests\test_startup_regression.py
python tests\test_headless_startup.py
python tests\test_api_endpoints.py
python tests\test_concurrency.py

# Or via pytest (if installed)
pytest tests/
```

Test files also import `llama_launcher.ui.app` indirectly through `main()`. The regression and headless tests mock this away — do not install `textual` just to run those tests; they work without it. The API and concurrency tests use real `ThreadingHTTPServer` on ephemeral ports (port 0) with isolated temp dirs.

## Architecture

- **`llama_launcher/`** — the only real package. All logic lives here.
- **`main.py`** and **`launcher.py`** (root) — thin wrappers delegating to `llama_launcher.main.main()`. Keep them in sync; tests verify identity.
- **`llama_launcher/api.py`** — `LlamaLauncherService` is the central facade. API server, TUI, and tests all call through it.
- **`llama_launcher/server.py`** — stdlib `http.server` only. Zero external deps. Routes are plain string matches on `self.path`.
- **`llama_launcher/config.py`** — legacy module-level persistence helpers. `LlamaLauncherService` delegates to them when `app_dir is APP_DIR`.
- **`llama_launcher/process.py`** — Windows-only: `tasklist` / `taskkill` for process lifecycle.

## State directory

All runtime state lives in `.launcher/` (gitignored):
- `global.json` — global settings (exe path, model dirs, API host/port)
- `profiles.json` — profiles with `advanced_favorites` / `advanced_values`
- `llama-server.pid` / `llama-server.log` — runtime artifacts

## Windows-specific

- `process.py` uses `tasklist` / `taskkill` — not portable.
- `monitoring.py` uses WinAPI (`ctypes`) for RAM and `nvidia-smi` for VRAM.
- `start_server()` uses `CREATE_BREAKAWAY_FROM_JOB | CREATE_NEW_PROCESS_GROUP` (0x08000000 | 0x00000200) to detach the child.

## Conventions

- Tests use plain `def test_*()` functions with an inline `if __name__ == "__main__"` loop — no pytest fixtures, no `assert` rewrites.
- `LlamaLauncherService` uses `threading.RLock` for all read-modify-write paths.
- API server uses `ThreadingHTTPServer` with a 1 MB body limit (`_MAX_BODY`).
