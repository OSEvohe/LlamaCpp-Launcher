# Planning — LLama Launcher

## Active Plans

### 🗂️ llama.cpp Version Management
Manage launcher-owned llama.cpp versions: installed list, GitHub catalog, download/delete lifecycle, and active-version selection.

- **Step 1:** [`llama-cpp-version-management/01-state-and-version-registry.md`](llama-cpp-version-management/01-state-and-version-registry.md)
- **Step 2:** [`llama-cpp-version-management/02-github-catalog-and-install-lifecycle.md`](llama-cpp-version-management/02-github-catalog-and-install-lifecycle.md)
- **Step 3:** [`llama-cpp-version-management/03-api-dashboard-and-tests.md`](llama-cpp-version-management/03-api-dashboard-and-tests.md)

### 🦀 Rust Migration (in progress)
Consolidate LLama Launcher as a native Rust binary under `rust-version/`.
7 phases: scaffolding → core logic → process/monitoring → HTTP API → CLI/auto-start → dashboard → testing & cutover.

**Details:** [`rust-migration.md`](rust-migration.md)
- **Implementation stories (13 stories):** [`rust-migration-stories.md`](rust-migration-stories.md)

---

## Completed Plans

### TUI → Service Facade Refactor (✅ done)
Extracted business logic into a UI-agnostic `LlamaLauncherService` facade so REST API, CLI, and web UI can share the same core.

- **Architecture & dependency diagram:** [`architecture.md`](architecture.md)
- **Implementation stories (9 stories):** [`stories.md`](stories.md)
