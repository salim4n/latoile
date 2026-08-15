# Contrat d'architecture — LaToile

Règles vérifiables. Toute PR qui en viole une est refusée, quelle que soit la valeur fonctionnelle.

## 1. Couches et dépendances

- `core` n'importe **rien** : pas de tokio, sqlx, axum, reqwest. Zéro I/O, zéro async.
- `app` orchestre via les ports (traits) de `core` ; il ne connaît pas axum ni sqlx.
- `server` contient tout axum ; il ne contient **aucune** logique : extraire, valider, déléguer à `app`.
- Les adaptateurs (`agents`, `preview`, `github`, `vault`, persistance) implémentent les ports ; le domaine ne les nomme jamais.

Vérifications :

```sh
grep -rn "tokio\|sqlx\|axum\|reqwest" crates/core/src/        # → vide
grep -rln "axum::" crates/ | grep -v "crates/server"          # → vide
grep -rn "sqlx::query" crates/ | grep -v "crates/app/src/store\|crates/vault"  # → vide
```

## 2. Fichiers

- Un use case = un fichier dans `app/src/use_cases/`. Un handler = une fonction qui délègue.
- Aucun fichier ne dépasse ~400 lignes sans justification écrite en commentaire d'en-tête (leçon : `api.rs` de Firetower, 2 323 lignes).
- Les machines à états (`Task`, `Run`, `SpecVersion`, `Preview`) vivent dans `core`, avec transitions exhaustives et testées. Aucune transition d'état en dehors de `core`.

## 3. Agents

- Tout processus agent passe par `agents/` : spawn supervisé, `kill_on_drop`, groupe de process, enregistrement dans le registry. Aucun `Command::new` ailleurs.
- Permissions : auto-rejet sur `.env`, chemins absolus, `docker` ; tout le reste non trivial passe par une `Approval`.
- Le Manager ne reçoit jamais de permission d'exécution destructrice ; il ne code pas.

## 4. Données

- Migrations embarquées, appliquées au démarrage ; jamais de modification destructive d'une migration mergée.
- Les invariants d'unicité partielle (run actif/tâche, preview/projet, spec approved/projet) sont des index DB **et** des gardes de la machine à états.
- `EVENT` est append-only ; `seq` est le seul curseur SSE.
- Les artefacts de design ne vont jamais en DB (ADR-003).

## 5. Erreurs et secrets

- Réponses d'erreur : `{code, message}` ; les détails internes vont dans `tracing`, jamais au client.
- Aucun secret en clair : tout passe par `vault` (XChaCha20-Poly1305, root key hors DB). Aucun log de valeur secrète.
- Toutes les routes sont derrière le token, preview comprise.

## 6. Frontend

- Data fetching uniquement via le module transport (client généré ou hooks) ; aucun `fetch` direct dans un composant.
- Les événements SSE sont validés (zod) avant d'entrer dans le cache — pas de cast (leçon V-M3 de Firetower).
- Mobile-first : chaque écran se conçoit viewport 390px d'abord.
- Aucune donnée mock en dehors d'un dossier `fixtures/` clairement exclu des routes réelles (leçon V-M2).

## 7. Tests

- `cargo test` est hermétique : vert sur machine propre, sans services externes.
- Toute machine à états de `core` a des tests de transitions, y compris les transitions refusées.
- La supervision des processus a un test de reap d'orphelins.
