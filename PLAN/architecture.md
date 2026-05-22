# Target Architecture — Module Dependency Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│  Entry Points                                                    │
│                                                                  │
│  launcher.py  ──────────────────────────────────────┐            │
│  (deprecated wrapper)                                │            │
│                                                     ▼            │
│  main.py  ──────────────────────────────────────────┼──► main()  │
│  (top-level convenience)                            │            │
│                                                     │            │
│  python -m llama_launcher ──────────────────────────┘            │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│  llama_launcher/                                                 │
│                                                                  │
│  __init__.py ────────────────────────────────────────────────┐   │
│  (version, package metadata)                                  │   │
│                                                               │   │
│  main.py ────────────────────────────────────────────────────┼──► ui.app.LlamaLauncherApp().run()
│  (package entry point)                                        │   │
│                                                               │   │
│  api.py  ──────── LlamaLauncherService ──────────────────────┤   │
│  (facade — the ONLY core dependency for the UI)               │   │
│    │                                                           │   │
│    ├──► models.py  (LlamaOption, GlobalSettings, Profile)     │   │
│    ├──► config.py  (load/save global, load/save profiles)     │   │
│    ├──► options.py (resolve path, parse help, load options)   │   │
│    ├──► command.py (build_command, canonical key helpers)     │   │
│    ├──► process.py (pid, is_running, launch, stop)            │   │
│    ├──► monitoring.py (RAM, VRAM, log tail)                   │   │
│    └──► discovery.py (GGUF model discovery)                   │   │
│                                                               │   │
│  ┌─────────────────────────────────────────────────────────┐  │   │
│  │  ui/                                                     │  │   │
│  │                                                          │  │   │
│  │  __init__.py                                             │  │   │
│  │                                                          │  │   │
│  │  app.py ──── LlamaLauncherApp(App) ─────────────────────┼──┤  │
│  │  (Textual TUI — depends ONLY on api.LlamaLauncherService)│  │  │
│  │    │                                                     │  │   │
│  │    └──► api.py  (service facade)                         │  │   │
│  └─────────────────────────────────────────────────────────┘  │   │
│                                                               │   │
│  models.py ───────────────────────────────────────────────────┘   │
│  (no external deps — pure dataclasses)                            │
│  config.py ───────────────────────────────────────────────────────┤
│  (deps: models, json, pathlib)                                    │
│  options.py ──────────────────────────────────────────────────────┤
│  (deps: models, subprocess, re)                                   │
│  command.py ──────────────────────────────────────────────────────┤
│  (deps: models, options, shlex)                                   │
│  process.py ──────────────────────────────────────────────────────┤
│  (deps: config, subprocess, pathlib)                              │
│  monitoring.py ───────────────────────────────────────────────────┤
│  (deps: ctypes, subprocess, pathlib)                              │
│  discovery.py ────────────────────────────────────────────────────┤
│  (deps: pathlib)                                                  │
└─────────────────────────────────────────────────────────────────┘

Dependency flow (simplified):

  UI (ui/app.py)
    │
    ▼
  LlamaLauncherService (api.py)
    │
    ├── models.py
    ├── config.py ──► models.py
    ├── options.py ──► models.py
    ├── command.py ──► models.py, options.py
    ├── process.py ──► config.py
    ├── monitoring.py
    └── discovery.py

  No textual imports outside ui/
  No circular dependencies
```

## Key Design Decisions

1. **Service facade over direct module imports**: The TUI imports only `LlamaLauncherService`. This makes it trivial to swap the UI later (e.g., a REST API would also just instantiate the service).

2. **`command.py` depends on `options.py`**: Because `build_command` calls `canonical_adv_key` and `favorite_string_value` which need the parsed options dict.

3. **`process.py` depends on `config.py`**: Because launch/stop need `PID_FILE`, `LOG_OUT`, `LOG_ERR`, `APP_DIR`.

4. **`monitoring.py` is self-contained**: No dependencies on other core modules — pure system queries (ctypes, subprocess).

5. **`discovery.py` is self-contained**: Takes `model_dirs` as parameter, returns paths.

6. **Windows-specific code stays in core**: `ctypes.windll.kernel32`, `tasklist`, `taskkill`, `nvidia-smi` are business logic (resource monitoring, process management), not UI concerns. They belong in `process.py` and `monitoring.py`.
