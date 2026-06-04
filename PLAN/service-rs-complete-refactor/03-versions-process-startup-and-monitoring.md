# Step 3 — Versions, process, startup, monitoring

**Goal:** séparer les workflows runtime et d’installation en services spécialisés.

**Scope:**
- `rust-version/src/service/mod.rs`
- `rust-version/src/process.rs`
- `rust-version/src/options.rs`
- `rust-version/src/monitoring/service.rs`
- `rust-version/src/versions/*.rs`

**Steps:**
1. Extraire le registre de versions actives/installées dans un `VersionService`.
2. Extraire téléchargement, progression, annulation, uninstall dans un `InstallService`.
3. Extraire `launch/stop/restart/is_server_running` dans un `ProcessService`.
4. Extraire `apply_startup_profile` dans un `StartupService` qui orchestre profils, options et process.
5. Limiter la couche service monitoring à une façade dédiée au payload API et aux stats/perf.

**Acceptance:**
- plus de mélange entre état persistant, orchestration async et process lifecycle
- les dépendances vers `versions`, `process`, `options`, `monitoring` passent par services ciblés
- les méthodes longues (`start_install_version`, `build_monitoring_payload`, `restart`, `launch`) sont découpées

**Risk:**
- l’install async écrit aujourd’hui directement dans `global.json` ; il faudra clarifier la stratégie de synchronisation pour éviter les régressions
