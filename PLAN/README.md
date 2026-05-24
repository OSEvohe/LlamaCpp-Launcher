# Planning — LLama Launcher

## Active Plans

### 🦀 Rust Migration (in progress)
Rewrite the Python application as a fully native Rust binary under `rust-version/`.
7 phases: scaffolding → core logic → process/monitoring → HTTP API → CLI/auto-start → dashboard → testing & cutover.
Python remains source of truth until Rust achieves full behavioral parity.

**Details:** [`rust-migration.md`](rust-migration.md)
- **Implementation stories (13 stories):** [`rust-migration-stories.md`](rust-migration-stories.md)

---

## Completed Plans

### TUI → Service Facade Refactor (✅ done)
Extracted business logic into a UI-agnostic `LlamaLauncherService` facade so REST API, CLI, and web UI can share the same core.

- **Architecture & dependency diagram:** [`architecture.md`](architecture.md)
- **Implementation stories (9 stories):** [`stories.md`](stories.md)
