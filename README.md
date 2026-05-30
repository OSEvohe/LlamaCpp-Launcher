# LLama Launcher

Version Rust de LLama Launcher (API + dashboard web pour `llama-server`).

## Lancer

```powershell
# Debug (convention: port 7891)
cargo run -- --api-host <YOUR_LAN_IP> --api-port 7891

# Release locale
cargo run --release -- --api-host <YOUR_LAN_IP> --api-port 7890
```

## Build

```powershell
cargo build
cargo build --release
```

Binaires:
- `rust-version\target\debug\llama-launcher.exe`
- `rust-version\target\release\llama-launcher.exe`

## Service Windows (release)

```powershell
sc.exe create LlamaLauncher binPath= '"%ProgramFiles%\LLama Launcher\llama-launcher.exe" --api-host <YOUR_LAN_IP> --api-port 7890' start= auto DisplayName= 'LLama Launcher'
sc.exe start LlamaLauncher
```

## Donnees runtime

Les etats restent dans `.launcher/` (`global.json`, `profiles.json`, `llama-server.pid`, `llama-server.log`).
