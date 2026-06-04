# Step 2 — Persistance + domaine profils/settings

**Goal:** isoler toute la logique CRUD, validation et mapping des profils/settings hors de la façade.

**Scope:**
- `rust-version/src/service/mod.rs`
- `rust-version/src/config.rs`
- `rust-version/src/models.rs`
- nouveaux fichiers sous `rust-version/src/service/`

**Steps:**
1. Extraire les accès `profiles.json` / `global.json` dans des repositories dédiés.
2. Déplacer les helpers de coercion dans un composant de validation unique.
3. Extraire `load/save/add/delete/duplicate/update profile` dans un `ProfileService`.
4. Extraire `load/save/update global` dans un `GlobalSettingsService`.
5. Déplacer les règles métier (`default profile`, unicité `start_on_boot`) dans des méthodes TDA dédiées.

**Acceptance:**
- la façade ne manipule plus directement le JSON ni le filesystem de persistance
- `update_profile` n’est plus un bloc monolithique
- les règles métier profil/settings sont testées hors façade
- les limites 5/10/20 sont tenables par service après découpage complémentaire si besoin

**Dependencies:**
- dépend des types `Profile`, `GlobalSettings`, `InstalledVersion`
- impact direct sur les tests unitaires actuellement dans `service.rs`
