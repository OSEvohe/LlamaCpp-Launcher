# Refactoring Plan — LLama Launcher

## Goal
Separate business logic from the Textual TUI view layer so that other interfaces (REST API, CLI, web UI) can be built on top of the same core logic. The existing TUI must continue to work identically after refactoring.

## Principles
1. **UI-agnostic core** — no `textual` imports outside `llama_launcher/ui/`.
2. **Facade pattern** — a single `LlamaLauncherService` class exposes all business operations; the TUI only talks to this service.
3. **Minimal diffs** — extract, don't rewrite. Each module is a direct lift of existing code with only the necessary changes to become independent.
4. **Windows-only preserved** — `ctypes.windll`, `tasklist`/`taskkill`, `nvidia-smi` stay in core (they are platform-specific business logic, not UI concerns).
5. **Backward-compatible paths** — `.launcher/global.json`, `.launcher/profiles.json`, PID file, log files all keep their current locations relative to the app directory.

## Target Architecture
```
llama_launcher/
├── __init__.py           # package init, version
├── models.py             # LlamaOption, GlobalSettings, Profile dataclasses
├── config.py             # load_global/save_global, load_profiles/save_profiles, ensure_state
├── options.py            # resolve_llama_server_path, parse_help_options, load_options_from_exe
├── command.py            # build_command(profile, exe_path, options)
├── process.py            # pid_value, is_process_running, launch_server, stop_server
├── monitoring.py         # ram_usage_bytes, process_ram_bytes, gpu_vram_info, bytes_to_gb, tail_log
├── discovery.py          # discover_models(model_dirs)
├── api.py                # LlamaLauncherService — high-level facade
└── ui/
    ├── __init__.py
    ├── app.py            # LlamaLauncherApp (Textual App)
    └── widgets.py        # helper functions for building UI rows (optional, if needed)
main.py                   # entry point: llama_launcher.ui.app:LlamaLauncherApp().run()
requirements.txt          # textual
launcher.py               # DEPRECATED — kept for backward compat, delegates to main.py
llama-launcher.ps1        # unchanged
```

See `architecture.md` for the full dependency diagram.

## Progress — ✅ Plan Complete
- ✅ **Story 1** — Package skeleton + dependency manifest
- ✅ **Story 2** — Extract data models
- ✅ **Story 3** — Extract persistence layer
- ✅ **Story 4** — Extract options discovery
- ✅ **Story 5** — Extract process/command/monitoring/discovery
- ✅ **Story 6** — Create the service facade (`api.py`)
- ✅ **Story 7** — Refactor TUI to use the service facade
- ✅ **Story 8** — Cleanup + top-level main.py
- ✅ **Story 9** — Verification pass

> **Note:** Temporary package compatibility shims were introduced early
> (`ui/app.py` re-export of `launcher.LlamaLauncherApp`, `__main__.py`,
> deferred import in `main.py`) to keep `python -m llama_launcher` usable
> until the full UI extraction in Story 7.
>
> **Story 5 update:** The real `LlamaLauncherApp` was moved into
> `llama_launcher/ui/app.py` during Story 5 (package-compatibility
> correction), and `launcher.py` became the backward-compat wrapper
> earlier than planned. This reduces the scope of Story 7 to wiring the
> service facade into the already-relocated app.
>
> **Non-blocking follow-ups:**
> - Clearing a favorite only removes the raw-key entry, not a
>   canonical-key fallback in some mixed/migrated profiles.
> - `err.log` contract is slightly stale: the file is defined/cleaned
>   while stderr is actually redirected into stdout.

## Stories
See `stories.md` for the sequential implementation plan (9 stories).
