# LLama Launcher

Launcher modulaire pour `llama-server` (llama.cpp) avec **TUI** (Textual), **dashboard web** et **API REST headless**.

## Lancer

```powershell
pip install textual

# Mode TUI (interface terminal)
python main.py

# Mode headless (API server uniquement, port 7890 par defaut)
python main.py --headless

# Personnaliser l'API host / port
python main.py --headless --api-host 0.0.0.0 --api-port 7890
```

## Modes d'execution

| Mode | Commande | Description |
|---|---|---|
| **TUI** | `python main.py` | Interface terminal Textual. Si un port API est configure, un sidecar HTTP s' lance en background. |
| **Headless** | `python main.py --headless` | API server uniquement, sans interface. Utile pour un controle distant via le dashboard web ou des scripts. |

## Architecture

```
llama_launcher/
├── main.py            # Entry point + parsing CLI (--headless, --api-host, --api-port)
├── api.py             # LlamaLauncherService (facade de toute la logique)
├── server.py          # HTTP API server (stdlib, zero dependances externes)
├── config.py          # Constantes de chemin + persistance JSON
├── models.py          # Dataclasses (Profile, GlobalSettings, LlamaOption)
├── command.py         # Assemblage de la commande llama-server
├── process.py         # Cycle de vie du processus (start / stop / restart)
├── discovery.py       # Scanner recursif des fichiers .gguf
├── monitoring.py      # RAM (WinAPI), VRAM (nvidia-smi), log tailing
├── options.py         # Parsing de llama-server --help pour les options avances
├── ui/app.py          # Interface TUI (Textual)
└── static/dashboard.html  # Dashboard web (vanilla JS, zero framework)
```

## Fonctionnalites

### TUI (Textual)
- Interface terminal fluide sans clignotement
- Edition des profils (`model`, `host`, `port`, `ctx-size`, `threads`, `n-gpu-layers`, `temp`, `top-p`, `batch-size`, `flash-attn`, `kv-cache-type`, `extra-args`)
- Gestion des **advanced favorites** : parcourir les options de `llama-server`, les etoiler par profil, definir des valeurs
- Boutons `Launch`, `Stop`, `Restart`
- Logs `stdout` en live tail
- Monitoring RAM / VRAM
- Decouverte automatique des modeles `.gguf`
- Profils sauvegardables (multiple profils)

### Dashboard Web
Accessible via le navigateur a l'adresse de l'API (ex: `http://127.0.0.1:7890/`).

Onglets :
- **Monitoring** — statut du serveur, RAM, VRAM, PID
- **Controls** — selection de profil, Launch / Stop / Restart
- **Profiles** — liste, creation, edition, suppression des profils
- **Settings** — chemin `llama-server.exe`, dossiers de modeles, API host/port
- **Logs** — streaming en temps reel des logs `llama-server`
- **Models** — decouverte et liste des `.gguf`
- **Advanced** — parcourir les options `llama-server`, etoiler des favoris par profil, definir leurs valeurs

### API REST
Tous les endpoints retournent du JSON.

| Methode | Endpoint | Description |
|---|---|---|
| `GET` | `/` | Dashboard HTML |
| `GET` | `/api/status` | Statut du serveur (running / PID) |
| `GET` | `/api/profiles` | Liste des profils |
| `GET` | `/api/profiles/:index` | Profil individuel |
| `POST` | `/api/profiles` | Creer un profil |
| `PUT` | `/api/profiles/:index` | Mettre a jour un profil (partiel) |
| `DELETE` | `/api/profiles/:index` | Supprimer un profil |
| `GET` | `/api/settings` | Reglages globaux |
| `PUT` | `/api/settings` | Mettre a jour les reglages |
| `GET` | `/api/options` | Options parses de `llama-server --help` |
| `GET` | `/api/models` | Modeles `.gguf` decouverts |
| `GET` | `/api/logs?last_size=N&last_marker=...` | Log tailing incrementiel |
| `GET` | `/api/monitoring` | RAM, VRAM, process RAM |
| `POST` | `/api/launch` | Lancer llama-server |
| `POST` | `/api/stop` | Arreter llama-server |
| `POST` | `/api/restart` | Restart llama-server |

## Advanced favorites

Les profils supportent deux champs avances :
- `advanced_favorites` — liste de flags `llama-server` a injecter dans la commande
- `advanced_values` — valeurs associees a chaque favori

Ces options sont assemblees par `command.py::build_command()` et injectees dans la commande finale. Elles sont gérables via l'onglet **Advanced** du dashboard web ou l'equivalent TUI.

## Fichiers sauvegardes

Les donnees sont stockees dans `.launcher/` :

| Fichier | Contenu |
|---|---|
| `global.json` | Reglages globaux (chemin exe, dossiers modeles, API host/port) |
| `profiles.json` | Profils (incluant advanced_favorites et advanced_values) |
| `llama-server.pid` | PID du processus en cours |
| `llama-server.log` | Sortie `stdout` de llama-server |

## Dependances

- **textual** — seule dependance externe (TUI)
- **stdlib Python** — API server (`http.server`), monitoring (`ctypes`, `subprocess`), zero dependance additionnelle
