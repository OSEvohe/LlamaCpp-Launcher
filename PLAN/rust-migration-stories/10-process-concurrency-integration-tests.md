# Story 10 — Process lifecycle + concurrency integration tests

**Depends on:** Story 9

Port integration tests for process lifecycle, log tailing, and concurrency.

### Scope
- `tests/integration.rs` — tests covering:
  - Launch → status → stop → restart cycle (with a dummy process).
  - Log tailing with markers (append, truncation, rewrite).
  - Concurrent requests to the API (port `test_concurrency.py`).
  - Headless startup (port `test_headless_startup.py`).
  - Startup regression (port `test_startup_regression.py`).

### Files
| Action | Path |
|--------|------|
| Create | `rust-version/tests/integration.rs` |

### Acceptance criteria
- ✅ Launch → stop → restart cycle completes without errors.
- ✅ PID file is created on launch and cleaned up on stop.
- ✅ Log tailing correctly handles append, truncation, and equal-size rewrite.
- ✅ Concurrent API requests do not cause data races or panics.

### Verification
```powershell
cd rust-version
cargo test --test integration
```
