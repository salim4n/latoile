# LaToile

**Un workbench de gestion de projet AI-native, self-hosté.** Tu discutes avec un Manager IA ; lui orchestre une équipe d'agents à rôles fixes — Architecte, Backend, Frontend, Reviewer — qui spécifient, codent et vérifient. Tu regardes l'application se construire en direct (preview web, mobile d'abord), et rien ne merge sans ton approbation.

> La Toile — le réseau parallèle dans lequel tes agents travaillent pendant que tu restes à la surface.

## État

**Conception.** Ce dépôt contient actuellement le package d'architecture complet, produit par une session de brainstorm structuré (domaine → stack → données → architecture) informée par l'audit de deux codebases réelles :

- [`docs/architecture-spec.md`](docs/architecture-spec.md) — vision, décisions actées (D1–D10), modèle de domaine, schéma ER, couches, contrat API, écrans, risques
- [`docs/adrs.md`](docs/adrs.md) — les 4 décisions fondatrices et leurs alternatives rejetées
- [`ARCHITECTURE_CONTRACT.md`](ARCHITECTURE_CONTRACT.md) — règles vérifiables (couches, secrets, erreurs, tests)
- [`docs/guardian-checklist.md`](docs/guardian-checklist.md) — contrôles anti-dérive avant merge

## Principes

- **Le projet est l'unité centrale**, pas la conversation.
- **La spec précède le code** — versionnée, dans `design/` du repo du projet, maquettes comprises (contrat visuel du frontend).
- **Approbation humaine obligatoire** — le Reviewer propose, toi seul disposes.
- **Un binaire, une base SQLite, zéro dépendance externe** — Rust + axum + sqlx, agents pilotés via [Agent Client Protocol](https://agentclientprotocol.com).

## Licence

AGPL-3.0-only. Si tu sers une version modifiée de LaToile en réseau, tu publies tes changements.
