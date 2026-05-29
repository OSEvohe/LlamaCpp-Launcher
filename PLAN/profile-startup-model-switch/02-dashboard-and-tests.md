# Step 2 — Dashboard wiring + tests

**Goal:** Expose the startup-switch flag in the dashboard and cover the new startup path with focused tests.

**Scope:**
- `rust-version/static/dashboard.html`
- `rust-version/tests/api_endpoints.rs`
- `rust-version/tests/integration.rs`
- nearby tests in `rust-version/src/config.rs` and `rust-version/src/service.rs`

**Steps:**
1. Add a checkbox in the profile editor, load it in `openProfileEditor`, and send it in `doSaveProfile`.
2. Optionally show a small badge in the profiles list so the startup profile is visible without opening the editor.
3. Add tests for JSON round-trip/defaults, API persistence, duplicate/update behavior, and startup auto-apply behavior.
4. Verify failure behavior stays safe: bad model path / missing exe should not prevent the dashboard API from starting unless product decides otherwise.

**Acceptance:**
- Checkbox state survives `GET /api/profiles` and `PUT /api/profiles/:index`.
- Existing profile edit flows still preserve advanced fields.
- Startup automation is covered for both cold start and already-running server cases.
- API startup behavior on auto-switch failure is explicitly tested or documented.

**Risk:**
- The feature sounds like a “model switch”, but the current architecture only changes model through `launch` / `restart`; confirm whether startup should also auto-launch when no server is running.
