# Checklist gardien — LaToile

À exécuter avant chaque merge. Tout doit être ✅.

| # | Vérification | Commande / méthode | Statut |
|---|--------------|--------------------|--------|
| 1 | `core` reste pur | `grep -rn "tokio\|sqlx\|axum\|reqwest" crates/core/src/` → vide | ☐ |
| 2 | HTTP confiné au serveur | `grep -rln "axum::" crates/ \| grep -v crates/server` → vide | ☐ |
| 3 | SQL centralisé | `grep -rn "sqlx::query" crates/ \| grep -v "store\|vault"` → vide | ☐ |
| 4 | Spawn centralisé | `grep -rn "Command::new" crates/ \| grep -v "crates/agents\|crates/preview"` → vide | ☐ |
| 5 | Handlers sans logique | revue : chaque handler fait extraire → valider → déléguer | ☐ |
| 6 | Pas de fuite d'erreurs | revue : aucune réponse ne contient de chaîne d'erreur interne | ☐ |
| 7 | Secrets uniquement via vault | `grep -rn "sk-\|Bearer\|token" crates/ --include="*.rs" \| grep -v "vault\|test"` → examiné | ☐ |
| 8 | SSE validé côté web | `grep -rn "as SessionEvent\|as .*Event" web/src/` → vide | ☐ |
| 9 | Pas de mock en prod | `grep -rn "fixtures\|mock" web/src/ \| grep -v "fixtures/"` → vide | ☐ |
| 10 | Tests hermétiques et verts | `cargo test` sur machine propre ; transitions d'état couvertes | ☐ |

## Anti-dérive (leçons de l'audit Firetower, 2026-08-15)

- Tout fichier qui approche 400 lignes est un candidat au découpage **dans la même PR** qui l'y amène.
- Une capacité ajoutée à un agent CLI ne se code jamais « en dur » hors du port `agents/`.
- Les commentaires qui décrivent un comportement doivent correspondre au code ; un commentaire faux est pire qu'absent (cas réels : `db.rs:601`, `transport.rs:80`, `events.ts:3` de Firetower).
- Les docs (`architecture-spec.md`, `adrs.md`) se mettent à jour dans la PR qui change la décision.
