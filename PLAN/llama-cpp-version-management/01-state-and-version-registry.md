# Step 1 — State model + version registry

**Goal:** Add first-class launcher state for installed llama.cpp versions and the selected active version.

**Scope:**
- `rust-version/src/models.rs`
- `rust-version/src/config.rs`
- `rust-version/src/service.rs`

**Steps:**
1. Define persisted metadata for an installed version (`tag`, source, install path, executable path, status, timestamps optional).
2. Extend global settings or a dedicated state file to store the installed-version list and `active_version` without breaking existing installs.
3. Add service methods for list installed versions, resolve active executable, set active version, and reject stale/missing installs.
4. Keep the existing `llama_server_path` flow compatible during migration/fallback.

**Acceptance:**
- Existing `.launcher` state still loads.
- Installed versions can be listed without scanning the whole disk.
- Active version survives restart.
- If the active version is missing, API returns a clear recoverable state.

**Risk / decision:**
- Prefer a dedicated launcher-managed install root (recommended: `.launcher/llama-cpp/versions/<tag>/`) instead of arbitrary external paths.
