# Bench — Agent Notes

## Scope

This folder is a small local coding-benchmark subproject.

Primary files:
- `run_coding_bench.py` — benchmark runner
- `webui.py` — local web UI server
- `webui.html` — web UI
- `tasks.json` — benchmark tasks
- `models.example.json` — model config example

## Goal

Compare local models/quantizations on reproducible coding tasks with:
- repo context injection
- patch application
- verification command execution
- saved artifacts under `bench/results/`

## Guardrails

- Keep diffs small and bench-focused.
- Do not turn this into a general orchestration framework.
- Prefer stdlib Python unless a dependency is clearly necessary.
- Preserve compatibility with Windows + PowerShell usage in this repo.
- Keep output formats stable when possible:
  - `summary.json`
  - `summary.md`
  - per-run artifact files

## Patch/output preference

Preferred model output format:
- OpenCode-style `*** Begin Patch` / `*** End Patch`

Fallback accepted by the runner:
- unified `git diff`

## Verification

Useful checks:

```powershell
python bench\run_coding_bench.py --models-file bench\models.example.json --dry-run
python bench\webui.py
```

Then verify:
- `http://127.0.0.1:8765/`
- `http://127.0.0.1:8765/api/config`

## Local-only files

Do not commit machine-specific bench files unless explicitly asked:
- `bench/results/`
- `bench/models.*.local.json`
- `bench/models.*.remote-*.json`
