# Story 12 — Release build + cutover preparation

**Depends on:** Story 11

Produce the final release binary and document the cutover procedure.

### Scope
- Build release binary: `cargo build --release`.
- Verify binary size and startup time.
- Update `PLAN/rust-migration.md` with cutover status.
- Create a rollback checklist.

### Files
| Action | Path |
|--------|------|
| Modify | `PLAN/rust-migration.md` |

### Acceptance criteria
- ✅ `cargo build --release` produces `target/release/llama-launcher.exe` (or chosen binary name).
- ✅ Binary starts, serves dashboard, and responds to API requests.
- ✅ Binary size is reasonable (< 30 MB).
- ✅ Cutover procedure is documented and tested (task swap + rollback).
- ✅ All tests pass: `cargo test --all-targets`.

### Verification
```powershell
cd rust-version
cargo build --release
cargo test --all-targets
.\target\release\llama-launcher.exe --api-port 0
```
