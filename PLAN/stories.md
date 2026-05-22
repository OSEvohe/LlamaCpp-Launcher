# Implementation Stories

Each story is self-contained and can be implemented in a single pass. Stories must be completed in order because each depends on the artifacts of the previous ones.

---

## Story 1 — Package skeleton + dependency manifest ✅ DONE
**Depends on:** nothing

Create the `llama_launcher/` package directory, `__init__.py`, `main.py` entry point, and `requirements.txt`.

### Files
| Action | Path |
|--------|------|
| Create | `llama_launcher/__init__.py` |
| Create | `llama_launcher/main.py` |
| Create | `requirements.txt` |

### `llama_launcher/main.py`
```python
"""Entry point for the LLama Launcher TUI."""
from llama_launcher.ui.app import LlamaLauncherApp

if __name__ == "__main__":
    LlamaLauncherApp().run()
```

### Acceptance criteria
- ✅ `pip install -r requirements.txt` succeeds.
- ✅ Module structure is valid.
- ✅ Existing `python launcher.py` still works (unchanged at this point).

---

## Story 2 — Extract data models ✅ DONE
**Depends on:** Story 1

Move `LlamaOption`, `GlobalSettings`, `Profile` dataclasses from `launcher.py` to `llama_launcher/models.py`.

### Files
| Action | Path |
|--------|------|
| Create | `llama_launcher/models.py` |
| Modify | `launcher.py` |

### Acceptance criteria
- ✅ `python launcher.py` runs identically.
- ✅ `from llama_launcher.models import Profile` works.
- ✅ No behavioral change.

---

## Story 3 — Extract persistence layer ✅ DONE
**Depends on:** Story 2

Move `ensure_state`, `load_global`, `save_global`, `load_profiles`, `save_profiles` and the path constants to `llama_launcher/config.py`.

### Files
| Action | Path |
|--------|------|
| Create | `llama_launcher/config.py` |
| Modify | `launcher.py` |

### Acceptance criteria
- ✅ `python launcher.py` runs identically.
- ✅ Profile load/save from config module works independently.

> **Shim note:** Review follow-ups added `ui/app.py` (re-export of
> `launcher.LlamaLauncherApp`), `ui/__init__.py`, `__main__.py`, and a
> deferred import in `main.py` so that `python -m llama_launcher` works
> now, before the full UI extraction in Story 7.

---

## Story 4 — Extract options discovery ✅ DONE
**Depends on:** Story 3

Move `resolve_llama_server_path`, `parse_help_options`, `load_options_from_exe` to `llama_launcher/options.py`.

### Files
| Action | Path |
|--------|------|
| Create | `llama_launcher/options.py` |
| Modify | `launcher.py` |

### Acceptance criteria
- ✅ `python launcher.py` runs identically.
- ✅ `load_options_from_exe` works from the new module.

---

## Story 5 — Extract process helpers + command builder + monitoring ✅ DONE
**Depends on:** Story 4

Extract four pure-business-logic modules: `process.py`, `command.py`, `monitoring.py`, `discovery.py`.

### Files
| Action | Path |
|--------|------|
| Create | `llama_launcher/process.py` |
| Create | `llama_launcher/command.py` |
| Create | `llama_launcher/monitoring.py` |
| Create | `llama_launcher/discovery.py` |
| Modify | `llama_launcher/ui/app.py` |
| Modify | `launcher.py` |

### Acceptance criteria
- ✅ `python launcher.py` runs identically.
- ✅ Each new module is importable and functional in isolation.

> **Scope shift:** The real `LlamaLauncherApp` was moved into
> `llama_launcher/ui/app.py` during this story (package-compatibility
> correction), and `launcher.py` became the backward-compat wrapper
> earlier than originally planned. This reduces Story 7's scope to
> wiring the service facade into the already-relocated app.
>
> **Non-blocking note:** Log polling still rereads the full file each
> tick; correctness is fine, performance follow-up only.

---

## Story 6 — Create the service facade (`api.py`) ✅ DONE
**Depends on:** Story 5

Create `llama_launcher/api.py` with a `LlamaLauncherService` class that encapsulates all business operations, delegating to the modules from Stories 2–5.

### Files
| Action | Path |
|--------|------|
| Create | `llama_launcher/api.py` |

### Acceptance criteria
- ✅ `LlamaLauncherService` is importable.
- ✅ All methods delegate correctly to their respective modules.
- ✅ No `textual` imports in `api.py`.

> **Reviewer fix:** `launch()` now calls `ensure_state()` before touching
> log/PID files, handling fresh app directories correctly.

---

## Story 7 — Refactor TUI to use the service facade ✅ DONE
**Depends on:** Story 6

Wire `LlamaLauncherService` into the already-relocated `ui/app.py`, replacing all direct module calls with `self.service.*` invocations. `launcher.py` remains the backward-compat wrapper.

### Files
| Action | Path |
|--------|------|
| Modify | `llama_launcher/ui/app.py` |
| Modify | `launcher.py` |

### Acceptance criteria
- ✅ `python launcher.py` launches the TUI with **identical** behavior.
- ✅ `python -m llama_launcher` works.
- ✅ No `textual` imports outside `llama_launcher/ui/`.
- ✅ All features preserved: profiles, global config, model discovery, advanced options, log tailing, monitoring, fullscreen toggle.

> **Review fixes:** Preserved original behavior for status messages,
> invalid-path distinction, launch/save ordering, and advanced-options
> refresh/clear.
>
> **Deferred UI polish (non-blocking):**
> - Saving an invalid server path still shows a generic success status
>   for global settings.
> - Stale `#adv_desc` visibility/content may remain after advanced-option
>   invalidation.

---

## Story 8 — Cleanup + top-level main.py ✅ DONE
**Depends on:** Story 7

Create a top-level `main.py` entry point and align `launcher.py` to delegate to the canonical `llama_launcher.main.main`.

### Files
| Action | Path |
|--------|------|
| Create | `main.py` |
| Modify | `launcher.py` |

### Acceptance criteria
- ✅ `python main.py` launches the TUI.
- ✅ `python -m llama_launcher` launches the TUI.
- ✅ `python launcher.py` still works (backward compat).

---

## Story 9 — Verification pass ✅ DONE
**Depends on:** Story 8

Final verification — no code changes, just testing.

### Verified
- ✅ `python -m compileall launcher.py main.py llama_launcher` succeeds
- ✅ `textual` imports confined to `llama_launcher/ui/app.py`
- ✅ `python launcher.py`, `python main.py`, `python -m llama_launcher` all launch the TUI
- ✅ All features operational: profiles, global config, model discovery, advanced options, launch/stop/restart, log tailing, monitoring, fullscreen

### Acceptance criteria
- ✅ All checklist items pass. The refactoring is complete.

> **Non-blocking follow-ups:**
> - Clearing a favorite only removes the raw-key entry, not a
>   canonical-key fallback in some mixed/migrated profiles.
> - `err.log` contract is slightly stale: the file is defined/cleaned
>   while stderr is actually redirected into stdout.
