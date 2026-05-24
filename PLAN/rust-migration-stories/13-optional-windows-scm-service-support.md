# Story 13 *(optional)* — Windows SCM service support

**Depends on:** Story 8

Implement native Windows service installation (`--install-service` / `--uninstall-service`).

### Scope
- `src/service_install.rs` — add `install_service()`, `uninstall_service()` using `windows-service` crate or `winapi` + `RegisterServiceCtrlHandlerExW`.
- May require a separate binary or entry point for the SCM callback.

### Files
| Action | Path |
|--------|------|
| Modify | `rust-version/src/service_install.rs` |
| Modify | `rust-version/src/main.rs` |

### Acceptance criteria
- ✅ `--install-service` registers "LlamaLauncher" in Windows SCM.
- ✅ `sc start LlamaLauncher` starts the API server.
- ✅ `sc stop LlamaLauncher` stops cleanly.
- ✅ `--uninstall-service` removes the service.

### Verification
```powershell
cd rust-version
.\target\release\llama-launcher.exe --install-service
sc start LlamaLauncher
sc stop LlamaLauncher
.\target\release\llama-launcher.exe --uninstall-service
```

### Non-goals
- This story is optional and deferred until the main migration is verified. Scheduled-task-only is sufficient for initial cutover.
