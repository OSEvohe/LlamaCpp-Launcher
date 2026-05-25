# Monitoring performances — prompt/generation speed

## Objectif
Suivi des vitesses d'inférence (prompt processing, text generation) extraites du log `llama-server.log`, avec statut chargement modèle, dernier prompt, et reset.

## Critères d'acceptation
- ✅ `monitoring.rs` : regex sur les lignes timing llama.cpp → `prompt_tps`, `gen_tps`, `model_loaded`, `last_prompt`
- ✅ `service.rs` : `get_perf_stats()` + `reset_perf_stats()`
- ✅ `server.rs` : `GET /api/perf` + `POST /api/perf/reset`
- ✅ `cargo test` passe

## Fichiers
`monitoring.rs`, `service.rs`, `server.rs`, `lib.rs`

## Risques
Format des lignes timing varie selon version `llama.cpp` → regex tolérants + tests sur échantillons.
