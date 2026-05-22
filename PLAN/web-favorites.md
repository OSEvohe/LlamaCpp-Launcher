# Plan : Favoris avancés dans l'UI web

## Résumé

Ajouter un onglet **Advanced** au dashboard web permettant de gérer les
`advanced_favorites` et `advanced_values` des profils, déjà pleinement
fonctionnels dans le TUI. Le backend (modèles, API, persistance) est déjà
complet — le travail se concentre sur le frontend HTML/JS et quelques tests.

---

## Plan d'implémentation

### 1. Réutiliser l'API existante `/api/options` (S)
- **Objectif** : Confirmer que `GET /api/options` retourne bien la liste des
  options avancées parsées depuis `llama-server --help`.
- **Livrable** : Aucune modification backend ; validation manuelle ou test
  existant.
- **Taille** : S

### 2. Nouvel onglet "Advanced" dans dashboard.html (M)
- **Objectif** : Ajouter un onglet `<button data-tab="advanced">` et son
  panneau `#panel-advanced`, dans le style existant (tab system, card, grid).
- **Livrable** : HTML + CSS intégrés à `static/dashboard.html`.
- **Contenu du panneau** :
  - Bouton "Refresh options" → appel `GET /api/options`.
  - Champ de recherche (filter par key/alias/description), à l'instar du TUI.
  - Table : colonne option (key + alias court), colonne checkbox "Favori".
  - Section dynamique en dessous : pour chaque favori, un input texte pour
    la valeur (comme `#adv_favorites_fields` du TUI).
- **Taille** : M

### 3. JS : chargement et affichage des options/favoris (M)
- **Objectif** : Fonctions JS côté client pour :
  - `fetchOptions()` → charge les options depuis l'API.
  - `renderOptionsTable()` → génère la table + checkboxes.
  - `renderFavoritesSection()` → génère les inputs de valeurs pour les favoris
    actifs du profil sélectionné.
  - Recherche en temps réel (filter client-side).
- **Livrable** : Fonctions JS dans `<script>` de dashboard.html.
- **Taille** : M

### 4. JS : sauvegarde des favoris dans le PUT profile (S)
- **Objectif** : Étendre `doSaveProfile()` pour inclure
  `advanced_favorites` (list) et `advanced_values` (dict) dans le body JSON.
- **Livrable** : Modification de la fonction `doSaveProfile()` existante.
- **Taille** : S

### 5. JS : chargement des favoris dans l'éditeur de profil (S)
- **Objectif** : Quand on ouvre un profil (`openProfileEditor`), charger aussi
  ses favoris dans l'onglet Advanced (si visible) et dans l'éditeur.
- **Livrable** : Extension de `openProfileEditor()` + sync avec l'onglet Advanced.
- **Taille** : S

### 6. Sélection de profil dans l'onglet Advanced (S)
- **Objectif** : Ajouter un select de profil dans l'onglet Advanced pour
  savoir sur quel profil on travaille les favoris.
- **Livrable** : Select + handler dans le panneau advanced.
- **Taille** : S

### 7. Tests API : favoris dans les profiles (M)
- **Objectif** : Ajouter des cas de test dans `test_api_endpoints.py` pour
  vérifier que `PUT /api/profiles/:index` préserve et met à jour correctement
  `advanced_favorites` et `advanced_values`.
- **Livrable** : 3-4 tests unitaires (`test_put_profile_with_favorites`,
  `test_get_profile_favorites_persisted`, etc.).
- **Taille** : M

### 8. Test de regression global (S)
- **Objectif** : S'assurer que les tests existants passent toujours et que
  le dashboard reste fonctionnel.
- **Livrable** : Exécution `python -m pytest tests/` + vérification visuelle.
- **Taille** : S

---

## Critères d'acceptation

1. L'onglet Advanced affiche la liste complète des options du serveur.
2. L'utilisateur peut cocher/décocher des favoris par profil.
3. Les valeurs des favoris sont éditables et sauvegardées.
4. Les favoris sauvegardés persistent après rechargement de la page.
5. Les favoris sont correctement injectés dans la commande au launch
   (logique backend existante `build_command` — pas de modification requise).
6. Tous les tests existants passent sans regression.

---

## Validation

- **Tests unitaires** : `python -m pytest tests/test_api_endpoints.py`
- **Validation manuelle** :
  1. Ouvrir dashboard → onglet Advanced → cocher un favori → sauvegarder profil.
  2. Recharger la page → vérifier que le favori persiste.
  3. Lancer le serveur → vérifier dans les logs que l'option favorite est
     bien présente dans la commande construite.
  4. Comparer avec le comportement équivalent du TUI.

---

## Risques & questions ouvertes

| Risque / Question | Impact | Atténuation |
|---|---|---|
| Pas de dépendance externe (pas de framework JS) — tout en vanilla JS | Faible | Cohérent avec l'existant ; complexité gérée par la simplicité du scope |
| Nombre élevé d'options (>100) peut ralentir le rendu DOM | Moyen | Pagination ou debounce sur la recherche ; à évaluer à l'usage |
| Le TUI utilise `canonical_adv_key()` côté client ; le web doit reproduire la logique | Moyen | Réutiliser la logique côté serveur ou exposer un endpoint de résolution |
| Synchronisation TUI ↔ Web si les deux sont ouverts simultanément | Faible | Les deux écrivent dans le même fichier JSON ; comportement identique au reste de l'app |

**Décision ouverte** : Les favoris sont-ils gérés par profil (oui, c'est le
comportement TUI) ou globalement ? → Suivre le TUI : **par profil**.

---

## Fichiers touchés

| Fichier | Type de modification |
|---|---|
| `llama_launcher/static/dashboard.html` | Ajout onglet Advanced, HTML + CSS + JS |
| `tests/test_api_endpoints.py` | Tests favoris dans les profiles |
| `llama_launcher/server.py` | Aucun changement (API existante suffit) |
| `llama_launcher/api.py` | Aucun changement |
| `llama_launcher/models.py` | Aucun changement |
