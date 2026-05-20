# LLama Launcher (Python TUI)

Interface terminal interactive (Textual) pour configurer et lancer `llama-server` en background.

## Lancer

Depuis PowerShell, dans ce dossier:

```powershell
pip install textual
python .\launcher.py
```

## Fonctionnalites

- Interface Textual (fluide, sans clignotement)
- Edition des options principales (`model`, `host`, `port`, `ctx-size`, etc.)
- Boutons `Launch`, `Stop`, `Restart`
- Logs `stdout` / `stderr` + `Live tail on/off`
- Profils sauvegardables (plusieurs profils)
- Reglages globaux:
  - chemin vers `llama-server.exe`
  - liste variable de dossiers a scanner pour trouver les `.gguf`

## Fichiers sauvegardes

Les donnees sont stockees dans `.launcher/`:

- `global.json` (reglages globaux)
- `profiles.json` (profils)
- `llama-server.pid`
- `llama-server.log`
- `llama-server.err.log`
