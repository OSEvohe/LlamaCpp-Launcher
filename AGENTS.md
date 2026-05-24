# LLama Launcher — Agent Notes

## Quick start

```powershell
cargo run --manifest-path rust-version\Cargo.toml -- --api-host 192.168.192.1 --api-port 7891
cargo run --release --manifest-path rust-version\Cargo.toml -- --api-host 192.168.192.1 --api-port 7890
```

## Service management

```powershell
sc.exe query LlamaLauncher
sc.exe stop LlamaLauncher
sc.exe start LlamaLauncher
```

## Running tests

```powershell
cargo test --manifest-path rust-version\Cargo.toml
```

## Architecture

- **`rust-version/src/main.rs`** — CLI entrypoint (`--api-host`, `--api-port`, install/uninstall task/service)
- **`rust-version/src/service.rs`** — facade metier
- **`rust-version/src/server.rs`** — serveur HTTP API + dashboard
- **`rust-version/src/service_install.rs`** — integration Scheduled Task / SCM service

## State directory

All runtime state lives in `.launcher/` (gitignored):
- `global.json` — global settings (exe path, model dirs, API host/port)
- `profiles.json` — profiles with `advanced_favorites` / `advanced_values`
- `llama-server.pid` / `llama-server.log` — runtime artifacts

## Windows-specific

- Build and service install tested on Windows.
- Service name: `LlamaLauncher`.

## Conventions

- Release service en `192.168.192.1:7890`.
- Debug local en `192.168.192.1:7891`.
