# Step 3 — API, dashboard, active switch, tests

**Goal:** Surface version management in the HTTP API and dashboard, then cover the lifecycle with focused tests.

**Scope:**
- `rust-version/src/server.rs`
- `rust-version/static/dashboard.html`
- `rust-version/tests/api_endpoints.rs`
- `rust-version/tests/integration.rs`

**Steps:**
1. Add API endpoints for installed versions, GitHub catalog, install, delete, and set-active.
2. Update launch/options flows to resolve the executable from the active version when present, while preserving manual-path fallback.
3. Add a dashboard section showing installed versions, available GitHub versions, current active version, and operation feedback.
4. Add tests for persistence, catalog parsing, install/delete guards, active-version selection, and launch path resolution.

**Acceptance:**
- UI can list installed and available versions separately.
- User can activate a version without editing raw paths.
- Launch/restart/options use the selected active executable.
- API failure modes are explicit for unsupported asset, missing active version, delete-active, and network errors.

**Risk / decision:**
- Confirm whether changing the active version should auto-restart a running server or only affect the next launch.
