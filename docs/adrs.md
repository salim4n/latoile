# ADR-001 — Monolithe modulaire Rust + SQLite + binaire unique, pas de fork d'AionCore

- **Date** : 2026-08-15
- **Statut** : accepté

## Contexte

LaToile partage 80 % de son ADN technique avec AionCore (daemon Rust local, agents CLI, HTTP+temps réel, SQLite). Trois options : forker AionCore, consommer AionCore comme dépendance, ou réécrire en réutilisant ses patterns.

## Décision

Réécriture d'un monolithe modulaire en workspace Cargo, avec réutilisation sélective : la crate officielle `agent-client-protocol`, le pattern de supervision `aionui-process` (reap d'orphelins conditionné par identité), le spawn builder (env scrubbing, kill_on_drop, kill d'arbre).

## Raisons

- L'unité de LaToile est le **projet** ; celle d'AionCore est la **conversation**. Forker revient à déporter le mismatch de domaine dans chaque table et chaque écran.
- Un fork d'un projet tiers actif (24 crates, cadence soutenue) fait de leur roadmap une dépendance permanente.
- Licence AionCore contradictoire (Cargo.toml MIT vs LICENSE Apache-2.0) — patterns réimplémentés, pas de copie verbatim.
- Le squelette de déploiement (binaire unique, SQLite, migrations au démarrage, assets embarqués) est déjà maîtrisé (Firetower).

## Conséquences

+ Domaine propre dès le premier commit ; `core` pur (zéro I/O) dès le départ.
− On renonce au catalogue multi-agents, au JWT/CSRF, à `aionrs`, au MCP intégré — à réintroduire au besoin, jamais par anticipation.

---

# ADR-002 — Canal ACP pour tous les agents ; deux modes de vie (Manager persistant, exécutants éphémères)

- **Date** : 2026-08-15
- **Statut** : accepté

## Contexte

Trois approches observées : PTY/tmux (Firetower — statut par heuristiques, dette documentée), ACP via adaptateurs épinglés (IgnitionRAG acp-runner), direct CLI pour claude/codex + ACP pour le reste (AionCore, après avoir pourtant construit l'infra ACP générique).

## Décision

Tous les agents passent par la crate `agent-client-protocol` v2 derrière un port `agents/` défini dans `core`. Le Manager a une session persistante par projet (resume à chaque message) ; les exécutants sont des runs éphémères (spawn → tâche → mort). Les permissions suivent le pattern AionCore : heuristique allow/approval/reject (auto-rejet : `.env`, chemins absolus, `docker`) puis file d'approbation humaine.

## Raisons

- Statut, cancel, permissions, usage structurés — exactement ce qui manque à Firetower pour Codex.
- Le port isole le choix : si ACP perd des capacités natives (leçon AionCore), un connecteur direct `claude`/`codex` peut remplacer l'adaptateur sans toucher au domaine.

## Conséquences

+ Une seule abstraction d'agent pour tout le système.
− Dépendance aux adaptateurs ACP versionnés ; le handshake doit vérifier la version et refuser poliment l'incompatible (pattern IgnitionRAG : version épinglée + prompt canari).

---

# ADR-003 — Les artefacts de design vivent dans le repo du projet ; la DB ne stocke que des métadonnées

- **Date** : 2026-08-15
- **Statut** : accepté

## Contexte

Les specs (domaine, ER, ADRs, maquettes HTML) sont des artefacts de première classe (D7 : la maquette est le contrat visuel). Deux stockages possibles : la base, ou le système de fichiers dans le repo.

## Décision

`design/` dans le repo du projet ; `SPEC_VERSION.design_dir` pointe dessus. Git fournit historique, diff et review. Les agents lisent et écrivent ces fichiers comme n'importe quel artefact du projet.

## Conséquences

+ Visibilité git gratuite ; les maquettes sont servies statiquement pour l'écran Review (maquette côte à côte du rendu).
− La cohérence « version approuvée ↔ contenu du répertoire » repose sur le workflow (commit avant approbation), pas sur la DB.

---

# ADR-004 — Branche de travail unique par projet en V1

- **Date** : 2026-08-15
- **Statut** : accepté, réversible en V2

## Contexte

Firetower isole chaque session dans une branche/worktree. LaToile veut une preview toujours cohérente et un modèle simple.

## Décision

Un projet = une `work_branch` ; tous les runs y committent ; la preview la sert. Le séquentiel est la règle (un run actif par tâche, dispatch ordonné). Le parallélisme (branche par run + intégration) est explicitement repoussé.

## Conséquences

+ Preview jamais ambiguë ; pas d'étape de merge intermédiaire ; modèle mental trivial.
− Risque de conflit si deux runs touchent les mêmes fichiers — accepté, surveillé ; si la douleur apparaît, ADR de bascule vers branche-par-run.
