# Step 2 — GitHub catalog + install lifecycle

**Goal:** Fetch available llama.cpp releases from GitHub and manage download/install/remove operations locally.

**Scope:**
- `rust-version/Cargo.toml`
- new service/helper module(s) under `rust-version/src/`
- runtime download/install area under `.launcher/`

**Steps:**
1. Add a GitHub client layer for releases/tags with timeout, basic caching, and explicit rate-limit / offline errors.
2. Define filtering rules for supported assets (Windows first, matching `llama-server.exe` payloads only).
3. Implement install flow: fetch asset, unpack to version folder, validate `llama-server.exe`, register metadata atomically.
4. Implement delete flow with guards: block removal of active/running version unless product wants forced replacement.
5. Expose progress/state transitions so UI can poll install status safely.

**Acceptance:**
- Available GitHub versions can be listed even when nothing is installed locally.
- Installing a supported release produces a validated executable in the managed version folder.
- Partial/failed downloads do not leave a version marked installed.
- Deleting a version updates registry state consistently.

**Risk / decision:**
- GitHub asset naming is not guaranteed; confirm exact supported artifact patterns before implementation.
