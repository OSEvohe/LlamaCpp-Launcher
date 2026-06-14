# Coding Bench

Bench automatisé minimal pour comparer plusieurs modèles/quantizations sur ce repo.

## Ce que fait le runner

- crée un `git worktree` temporaire par run
- injecte un prompt coding + les fichiers de contexte de la tâche
- appelle une API compatible OpenAI `chat/completions`
- attend un patch `git diff`
- applique le patch
- lance la commande de vérification de la tâche
- enregistre un résumé JSON + Markdown

## Pré-requis

- Python 3.10+
- `git`
- un endpoint compatible OpenAI Chat Completions

## Fichiers

- `bench/tasks.json` — tâches du bench
- `bench/models.example.json` — exemple de config modèles
- `bench/run_coding_bench.py` — runner
- `bench/webui.py` — mini serveur web local
- `bench/webui.html` — interface web

## Config modèles

Copier `bench/models.example.json` vers un fichier local, puis remplir `base_url` et `model`.

Exemple :

```json
[
  {
    "name": "unsloth-q5km",
    "base_url": "http://127.0.0.1:8080/v1",
    "model": "Qwen3.6-27B-Q5_K_M",
    "api_key": "",
    "temperature": 0.0,
    "max_tokens": 8192
  }
]
```

## Lancer

```powershell
python bench\run_coding_bench.py --models-file bench\models.local.json
```

UI web :

```powershell
python bench\webui.py
```

Puis ouvrir `http://127.0.0.1:8765`.

Filtrer :

```powershell
python bench\run_coding_bench.py --models-file bench\models.local.json --task monitoring-api-test --model unsloth-q5km
```

## Résultats

Le runner écrit sous `bench/results/<timestamp>/` :

- `summary.json`
- `summary.md`
- un dossier par `modele__tache` avec :
  - `prompt.txt`
  - `response.txt`
  - `patch.diff`
  - `apply.log`
  - `verify.log`

## Score automatique

- `2` = patch appliqué + vérification OK
- `1` = patch appliqué mais vérification KO
- `0` = erreur API, patch invalide, ou aucun patch exploitable
