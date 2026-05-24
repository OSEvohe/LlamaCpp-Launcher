# Story 9 — Port API endpoint tests

**Depends on:** Story 7

Port the Python API endpoint test suite (~35 tests) to Rust.

### Scope
- `tests/api_endpoints.rs` — port all tests from `tests/test_api_endpoints.py`:
  - Status, profiles CRUD, duplicate, settings, models, logs, unknown route.
  - MTP field normalization (string→bool, string→int, invalid defaults).
  - Legacy `--spec-type` / `--spec-draft-n-max` migration.
  - `build_command` MTP flag dedup.
  - Malformed `advanced_favorites` / `advanced_values` resilience.
- Use `reqwest` for HTTP client, `tempfile` for isolated state dirs.

### Files
| Action | Path |
|--------|------|
| Create | `rust-version/tests/api_endpoints.rs` |

### Acceptance criteria
- ✅ All ~35 ported tests pass (`cargo test --test api_endpoints`).
- ✅ Test isolation: each test uses a fresh temp directory for `.launcher/` state.
- ✅ Tests exercise the same assertions as Python (status codes, JSON keys, field values).

### Verification
```powershell
cd rust-version
cargo test --test api_endpoints
```
