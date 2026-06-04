# Step 4 — Façade finale + adaptation API + migration des tests

**Goal:** recâbler l’API publique sans casser `server.rs`, puis redistribuer les tests vers les nouveaux modules.

**Scope:**
- `rust-version/src/server.rs`
- `rust-version/src/lib.rs`
- `rust-version/src/main.rs`
- `rust-version/tests/*.rs`
- nouveaux tests unitaires sous `rust-version/src/service/`

**Steps:**
1. Réduire `LlamaLauncherService` à une façade d’injection/orchestration.
2. Adapter imports/modules (`crate::service::LlamaLauncherService`) sans changer les endpoints.
3. Déplacer les tests co-localisés de `service.rs` vers les nouveaux composants.
4. Garder un filet de sécurité via tests unitaires + `api_endpoints.rs` + `integration.rs`.

**Acceptance:**
- `server.rs` compile avec la même API métier ou un diff d’adaptation minimal
- `service.rs` n’est plus un god object ni un fichier de tests géant
- les responsabilités et tests sont distribués par composant
- la refactorisation est vérifiable par `cargo test --manifest-path rust-version\Cargo.toml`

**Pattern fit:**
- suit le pattern déjà visible dans `monitoring/service.rs` : état encapsulé, TDA, helpers stateless séparés
