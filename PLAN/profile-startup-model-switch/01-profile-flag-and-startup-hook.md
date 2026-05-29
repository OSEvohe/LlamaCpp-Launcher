# Step 1 — Profile flag + startup hook

**Goal:** Persist a per-profile boolean flag and apply the flagged profile automatically when the launcher starts.

**Scope:**
- `rust-version/src/models.rs`
- `rust-version/src/service.rs`
- `rust-version/src/main.rs`
- `rust-version/src/config.rs`

**Steps:**
1. Add a new boolean field on `Profile` with default `false` and backward-compatible deserialization.
2. Thread the field through profile duplication and partial update logic.
3. Define the selection rule for startup (`0` or `1` profile enabled recommended; if one is enabled, build the command and launch/restart from startup code).
4. Call the startup helper from both normal boot and Windows service boot.

**Acceptance:**
- Missing field in existing `profiles.json` keeps old installs working.
- Saving/loading/duplicating profiles preserves the flag.
- No enabled profile => startup behavior stays unchanged.
- One enabled profile => launcher applies that profile automatically on boot.
- Multiple enabled profiles have deterministic handling.

**Decision to confirm:**
- Recommended: only one profile can be enabled at a time; enabling one clears the flag on the others.
