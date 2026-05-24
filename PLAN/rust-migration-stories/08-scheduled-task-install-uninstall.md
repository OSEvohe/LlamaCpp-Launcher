# Story 8 — Scheduled task install/uninstall

**Depends on:** Story 7

Implement Windows scheduled task management (`schtasks` subprocess calls).

### Scope
- `src/service_install.rs` — `install_task()`, `uninstall_task()`, `task_exists()`.
- Wire `--install-task` and `--uninstall-task` into `main.rs`.
- Use `std::process::Command` for `schtasks /create` and `schtasks /delete`.

### Files
| Action | Path |
|--------|------|
| Create | `rust-version/src/service_install.rs` |
| Modify | `rust-version/src/main.rs` |

### Acceptance criteria
- ✅ `--install-task` creates a task named "LLama Launcher" visible in `taskschd.msc`.
- ✅ `--uninstall-task` removes it.
- ✅ `--install-task` with `--force` overwrites existing task.
- ✅ Task triggers `onlogon` with the Rust binary as target.

### Verification
```powershell
cd rust-version
cargo run -- --install-task
Get-ScheduledTask -TaskName "LLama Launcher"
cargo run -- --uninstall-task
```

### Non-goals
- Native Windows SCM service (`--install-service`) is deferred to a follow-up story if needed.
