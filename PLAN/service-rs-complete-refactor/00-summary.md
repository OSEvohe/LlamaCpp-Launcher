# service.rs — refactor complète

**Goal:** remplacer `rust-version/src/service.rs` par une architecture modulaire orientée objets, avec responsabilités séparées, un type principal par fichier, et une façade mince compatible avec `server.rs`.

**Constat actuel:** `service.rs` (~2619 LOC) mélange persistance, profils, settings globaux, versions, installation, process lifecycle, monitoring, helpers de coercion, filesystem helpers, et tests unitaires.

**Contraintes:**
- style OOP (`struct`/`trait` Rust = “classe”/interface)
- 1 type principal par fichier
- SOLID / Clean Code / DRY
- règle 5/10/20
- TDA
- arborescence en sous-dossiers par type, style Symfony
- préserver l’API publique attendue par `rust-version/src/server.rs`

**Hypothèses recommandées:**
- interpréter “classe” en Rust comme `struct` + `impl`, avec `trait` pour les contrats
- sortir les tests de `service.rs` vers des modules/unit tests dédiés
- remplacer `src/service.rs` par `src/service/mod.rs`

**Découpage:**
1. Cadrer l’architecture cible et les contrats.
2. Extraire la persistance + profil/settings.
3. Extraire versions/process/startup/monitoring.
4. Réadapter la façade publique, les imports et les tests.
