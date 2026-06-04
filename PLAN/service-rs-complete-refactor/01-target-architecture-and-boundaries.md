# Step 1 — Architecture cible + frontières

**Goal:** définir la nouvelle arborescence `src/service/` et répartir chaque responsabilité de `service.rs` dans un service dédié.

**Scope:**
- `rust-version/src/service.rs`
- `rust-version/src/lib.rs`
- `rust-version/src/server.rs`

**Steps:**
1. Remplacer le module fichier unique par `rust-version/src/service/mod.rs`.
2. Créer des sous-dossiers par type (ex. `application/`, `repository/`, `support/`, `dto/` si nécessaire).
3. Définir une façade `LlamaLauncherService` réduite à l’orchestration.
4. Lister les contrats à conserver pour `server.rs` et les tests d’intégration.

**Acceptance:**
- chaque responsabilité majeure de `service.rs` a une destination claire
- la façade publique garde les points d’entrée utilisés par `server.rs`
- aucun nouveau module ne dépasse un rôle métier unique

**Suggested split:**
- `application/profile_service.rs`
- `application/global_settings_service.rs`
- `application/version_service.rs`
- `application/install_service.rs`
- `application/process_service.rs`
- `application/startup_service.rs`
- `application/monitoring_facade.rs`
- `repository/profile_repository.rs`
- `repository/global_settings_repository.rs`
- `support/value_coercer.rs`
- `support/file_tree_copier.rs`
- `support/executable_finder.rs`
