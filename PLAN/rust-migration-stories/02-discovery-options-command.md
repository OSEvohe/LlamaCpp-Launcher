# Story 2 — Model discovery + options parsing + command assembly

**Depends on:** Story 1

Port the pure-logic modules: `.gguf` file scanning, `llama-server --help` option parsing, and command-line assembly.

### Scope
- `src/discovery.rs` — `scan_gguf_models(model_dirs)` recursive `.gguf` file finder.
- `src/options.rs` — `resolve_llama_server_path()`, `parse_help_options(help_text)`, `load_options_from_exe(exe_path)` using `std::process::Command` to run `--help`.
- `src/command.rs` — `build_command(exe, profile, options)`, `canonical_adv_key()`, `favorite_string_value()` including MTP dedup.

### Files
| Action | Path |
|--------|------|
| Create | `rust-version/src/discovery.rs` |
| Create | `rust-version/src/options.rs` |
| Create | `rust-version/src/command.rs` |
| Modify | `rust-version/src/main.rs` |

### Acceptance criteria
- ✅ `scan_gguf_models()` returns sorted, deduplicated list of `.gguf` paths.
- ✅ `parse_help_options()` correctly extracts option keys, aliases, arity, defaults from `llama-server --help` output.
- ✅ `build_command()` produces the same argument list as Python `cmd_module.build_command()` for an identical profile.
- ✅ MTP flags (`--spec-type`, `--spec-draft-n-max`) appear exactly once when `enable_mtp=true`, even if also in `advanced_favorites`.
- ✅ `build_command()` omits MTP flags when `enable_mtp=false`.

### Verification
```powershell
cd rust-version
cargo test
```

### Non-goals
- No process spawning. No HTTP server.
