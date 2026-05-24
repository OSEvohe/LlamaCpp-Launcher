# Story 11 — Parity verification + comparison harness

**Depends on:** Story 10

Build and run a side-by-side comparison harness that validates Rust output against Python.

### Scope
- `tests/parity.rs` or a standalone script that:
  1. Starts the Python server on port A and the Rust server on port B.
  2. Sends the same sequence of requests to both.
  3. Asserts JSON responses are byte-identical.
  4. Tests state compatibility: write state with Python, read with Rust (and vice versa).

### Files
| Action | Path |
|--------|------|
| Create | `rust-version/tests/parity.rs` |

### Acceptance criteria
- ✅ All API endpoints return identical JSON from Python and Rust servers.
- ✅ State files written by Python are readable by Rust with identical values.
- ✅ State files written by Rust are readable by Python with identical values.
- ✅ Zero diffs in the comparison harness.

### Verification
```powershell
cd rust-version
cargo test --test parity
```

### Non-goals
- Performance comparison is not required.
