# TUI Removal — Web-UI-Only Conversion

## Summary

Remove the Textual-based TUI and make the API server + web dashboard the sole interface. After this change, `python main.py` starts the API server in the foreground (blocking), serving the web dashboard at `http://localhost:7890`. The `--headless` flag is removed since it's the only mode.

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| `--headless` flag | **Removed** | Single mode = no flag needed. No backward-compat reason to keep a no-op. |
| Sidecar API concept | **Removed** | No TUI means no sidecar. `_start_api_sidecar()` is dead code. |
| Default port | **7890** | Was already the headless default; becomes the universal default. |
| `requirements.txt` | **Deleted** | `textual` was the only external dep. Project is stdlib-only. |
| `llama_launcher/ui/` | **Deleted** | Entire directory removed (2 files). |

## Stories / Tasks

### 1. Rewrite `llama_launcher/main.py`
- Remove `--headless` arg from argparse.
- Remove `_start_api_sidecar()` and `_run_api_headless()` — collapse into a single path: resolve settings, then call `run_api_server()` blocking.
- Remove the TUI import (`from llama_launcher.ui.app import ...`).
- Simplify `_resolve_api_settings()`: drop the `headless` parameter; always default port to 7890 when unset.
- Update argparse description to "LLama Launcher — API server and web dashboard for llama.cpp".
- **Files:** `llama_launcher/main.py`

### 2. Delete TUI package
- Remove `llama_launcher/ui/` directory entirely (`__init__.py`, `app.py`).
- **Files:** `llama_launcher/ui/__init__.py`, `llama_launcher/ui/app.py`

### 3. Delete `requirements.txt`
- File contains only `textual`; project is now stdlib-only.
- **Files:** `requirements.txt`

### 4. Update root wrappers
- Update docstrings in `main.py` and `launcher.py` to reflect "API server" instead of "TUI".
- **Files:** `main.py`, `launcher.py`

### 5. Rewrite `tests/test_startup_regression.py`
- Remove all TUI-sidecar tests (5 tests: `test_default_non_headless_launches_tui`, `test_non_headless_with_api_port_still_launches_tui`, `test_sidecar_starts_when_port_positive_non_headless`, `test_sidecar_skipped_when_port_zero_non_headless`, `test_headless_does_not_launch_tui`).
- Remove `--headless` CLI parsing tests (`test_cli_headless_flag_parsed`, `test_cli_headless_with_api_port_combined`).
- Keep entrypoint delegation tests (3 tests): `test_main_py_delegates_to_canonical_main`, `test_launcher_py_delegates_to_canonical_main`, `test_launcher_py_all_exports_main`.
- Rewrite remaining CLI tests against the simplified single-mode flow: verify `--api-host` and `--api-port` still work, and that empty args default to 7890.
- New test scope: entrypoint identity + CLI flag parsing + default port resolution.
- **Files:** `tests/test_startup_regression.py`

### 6. Rewrite `tests/test_headless_startup.py`
- Remove `headless` parameter from all `_resolve_api_settings` calls (now just `(cli_host, cli_port)`).
- Remove the "headless fallback 7890" section — 7890 is now always the default.
- Remove sidecar-specific tests (`test_sidecar_bind_failure_non_headless_continues`, `test_sidecar_zero_port_non_headless_skips_sidecar`).
- Keep port sanitization boundary tests (10 tests) — adapt to new signature.
- Keep bind-failure test — rename to remove "headless" from name, keep the same error-path logic.
- New test: verify default port is 7890 without any "headless" context.
- **Files:** `tests/test_headless_startup.py`

### 7. Verify unaffected tests still pass
- `tests/test_api_endpoints.py` — no changes expected.
- `tests/test_concurrency.py` — no changes expected.
- **Files:** `tests/test_api_endpoints.py`, `tests/test_concurrency.py` (read-only verification)

### 8. Update `AGENTS.md` quick-start
- Replace the two-mode quick-start with a single `python main.py` command.
- Remove the `--headless` reference.
- **Files:** `AGENTS.md`

## Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| Users with scripts invoking `--headless` break | Acceptable breaking change; document in commit message. Flag removal is intentional. |
| Port 0 edge case: old non-headless default was port 0 (no API); new default is 7890 (always API) | By design — the web UI is the interface. If a user wants port 0 (ephemeral), they can use `--api-port 0`. |
| `llama_launcher/ui/` imported elsewhere | Grep confirmed: only imported in `llama_launcher/main.py:102` and mocked in tests. No other references. |
| Test file ordering / import side effects | Tests use `if __name__ == "__main__"` runners; run each independently to verify. |

## Definition of Done

- [ ] `python main.py` starts the API server on `127.0.0.1:7890` and blocks.
- [ ] `http://localhost:7890` serves the web dashboard.
- [ ] `llama_launcher/ui/` directory does not exist.
- [ ] `requirements.txt` does not exist (or is empty).
- [ ] `--headless` flag is not recognized by argparse.
- [ ] `--api-host` and `--api-port` still work.
- [ ] All 4 test files pass (`python tests/test_*.py` with `PYTHONPATH` set).
- [ ] `AGENTS.md` quick-start reflects single-mode operation.
