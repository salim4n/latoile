# LaToile — Spécification d'architecture

> **Date** : 2026-08-15
> **Statut** : validé par le propriétaire (Phases 1-4)
> **Origine** : session app-architect-brainstorm, Mode A (greenfield) éclairée par l'audit de Firetower et l'analyse d'AionCore/AionUi et IgnitionRAG.

## 1. Vision

**LaToile est un workbench de gestion de projet AI-native, mono-utilisateur, self-hosté.** L'unité centrale est le **projet**, pas la conversation (AionUi) ni la session détachée (Firetower). L'utilisateur discute avec un agent **Manager** par projet ; le Manager traduit la discussion en spécifications (via l'Architecte), en tâches, et en runs d'agents exécutants (backend, frontend, reviewer). Le rendu de l'application en construction est visible en direct (preview web, viewport mobile d'abord). Rien ne merge sans approbation humaine explicite.

Nom : la Toile (Webway, WH40k) — le réseau parallèle dans lequel les agents travaillent ; aussi « la toile » = le web.

## 2. Décisions structurantes (actées)

| # | Décision | Alternative rejetée |
|---|----------|---------------------|
| D1 | Mono-utilisateur d'abord, self-hosté VPS, token unique | Multi-tenant / SaaS — reporté, non anticipé dans le modèle au-delà du token |
| D2 | Réutilisation **sélective** des patterns AionCore | Fork dur — couplage à leur cadence, codebase tierce de 24 crates |
| D3 | Preview = apps web, mobile-first | Toutes cibles (mobile émulateur, desktop) — hors V1 |
| D4 | Rôles fixes : Manager, Architecte, Backend, Frontend, Reviewer | Équipes configurables — V2 |
| D5 | Branche de travail unique par projet (V1) | Branche par run + intégration — vrai parallélisme, V2 |
| D6 | Chat libre uniquement avec le Manager ; exécutants = runs structurés | Chat avec chaque agent |
| D7 | Maquettes HTML = contrat visuel du CodeurFrontend | Maquettes comme simple référence |
| D8 | Artefacts de design dans le repo du projet (`design/`), métadonnées en DB | Tout en base — rend les artefacts invisibles à git et aux agents |
| D9 | Auth par token partout, preview incluse | Preview ouverte |
| D10 | Preview auto-rechargée à la fin d'un run frontend | Rechargement manuel |

## 3. Modèle de domaine

### 3.1 Bounded contexts

| Contexte | Responsabilité | Entités |
|---|---|---|
| Projet | cycle de vie, lien repo, état | `Project` |
| Design | spec versionnée, artefacts | `SpecVersion` |
| Orchestration | rôles, tâches, runs, approbations | `Role`, `Task`, `Run`, `Approval` |
| Conversation | le fil Manager | `Conversation`, `Message` |
| Preview | dev server, port, état | `Preview` |
| Intégrations | GitHub, catalogue de skills | (clients infrastructure) |
| Journal | événements, curseur SSE | `Event` |
| Secrets | tokens chiffrés | `Secret` |

### 3.2 Invariants

1. Un seul run actif par tâche (contrainte DB : index unique partiel).
2. Une seule preview active par projet (idem).
3. Une seule `SpecVersion` `approved` par projet (idem).
4. `Task.status = done` exige une `Approval` `granted` de kind `review` — machine à états du domaine, pas le SQL.
5. Un run exécutant porte : tâche + extraits de spec + skill du rôle. Jamais de chat libre.
6. La preview sert toujours la `work_branch` déclarée ; si elle sert autre chose, l'UI l'affiche (`stale`).
7. Le Manager ne reçoit jamais de permission dangereuse ; il ne code pas.

### 3.3 Événements domaine

`SpecVersionCreated/Approved` · `TaskReady` · `RunStarted/Blocked/Finished` · `ApprovalRequested/Granted/Rejected` · `PreviewReady/Stale/Error` · `MessagePosted` — tous appendés dans `EVENT(seq, project_id, kind, payload)`, curseur SSE monotone.

### 3.4 Rôles (table `ROLE`, ids stables)

| id | Skill dédié | Mode de vie | Sortie |
|----|------------|-------------|--------|
| `manager` | skill chef-de-projet (à écrire) | session ACP persistante, resume par message | messages + actions |
| `architect` | `app-architect-brainstorm` | run éphémère | `SpecVersion` + fichiers `design/` |
| `backend` | skill backend (à écrire) | run éphémère | commits + summary |
| `frontend` | skill frontend (à écrire) | run éphémère | commits + summary |
| `reviewer` | skill review (à écrire) | run éphémère | verdict → `Approval` |

## 4. Modèle de données (ER)

```mermaid
erDiagram
    PROJECT ||--o{ SPEC_VERSION : "a des versions de spec"
    PROJECT ||--o{ TASK : "découpé en"
    PROJECT ||--o| PREVIEW : "une preview active max"
    PROJECT ||--o{ EVENT : "émet"
    PROJECT ||--|| CONVERSATION : "une seule"
    CONVERSATION ||--o{ MESSAGE : "fil permanent"
    SPEC_VERSION ||--o{ TASK : "matérialisée en"
    TASK ||--o{ RUN : "exécutée par"
    ROLE ||--o{ RUN : "joué par"
    RUN ||--o{ APPROVAL : "demande"

    PROJECT {
        text id PK "ulid"
        text name
        text slug UK
        text github_repo "owner/name"
        text default_branch
        text work_branch "unique en V1"
        text local_path "checkout sur le VPS"
        text status "draft | specced | building | live"
        text dev_command "ex: pnpm dev --port $PORT"
        text created_at
        text updated_at
    }
    SPEC_VERSION {
        text id PK "ulid"
        text project_id FK
        integer version
        text status "draft | approved | superseded"
        text design_dir "chemin design/ dans le repo"
        text architect_run_id FK "nullable"
        text created_at
    }
    ROLE {
        text id PK "manager | architect | backend | frontend | reviewer"
        text label
        text skill_path
        text cli "claude | codex"
        text system_prompt_path
    }
    TASK {
        text id PK "ulid"
        text project_id FK
        text spec_version_id FK
        text role_id FK
        text title
        text description
        text status "ready | in_progress | review | changes_requested | done"
        integer position
        text created_at
        text updated_at
    }
    RUN {
        text id PK "ulid"
        text task_id FK
        text role_id FK
        text triggered_by "user | manager"
        text acp_session_id
        text status "starting | running | blocked | finished | error | cancelled"
        integer input_tokens
        integer output_tokens
        text summary
        text started_at
        text ended_at
    }
    APPROVAL {
        text id PK "ulid"
        text run_id FK
        text kind "spec | review | permission"
        text status "pending | granted | rejected"
        text payload "json : diff, question, verdict"
        text decided_at
    }
    CONVERSATION {
        text id PK "ulid"
        text project_id FK "unique"
        text created_at
    }
    MESSAGE {
        text id PK "ulid"
        text conversation_id FK
        text author "user | manager"
        text content "markdown"
        text actions "json : tâches créées, runs lancés, liens"
        text created_at
    }
    PREVIEW {
        text id PK "ulid"
        text project_id FK
        integer port
        text status "starting | ready | stale | error | stopped"
        text branch
        integer pid
        text started_at
    }
    EVENT {
        integer seq PK "curseur SSE monotone"
        text project_id FK
        text kind
        text payload "json"
        text created_at
    }
    SECRET {
        text name PK
        text ciphertext "XChaCha20-Poly1305, AAD = name"
        text wrapped_key "clé par secret, wrappée par root key"
        text created_at
        text rotated_at
    }
```

Contraintes : index uniques partiels sur runs actifs/tâche, previews actives/projet, spec approved/projet. Audit fields partout. Soft delete uniquement sur `PROJECT` (`deleted_at`).

## 5. Architecture

### 5.1 Crates

```
crates/
├── core/      domaine pur : entités, machines à états, événements, ports (traits). Zéro I/O, zéro async
├── agents/    canal ACP : spawn supervisé, sessions, permissions, usage
├── preview/   supervision dev server, allocation de ports, reverse proxy
├── github/    client API GitHub
├── vault/     secrets (XChaCha20-Poly1305, root key hors DB)
├── app/       use cases : SendMessage, DispatchTask, GrantApproval, EnsurePreview…
├── server/    HTTP axum, SSE, assets embarqués, auth token — extraire, valider, déléguer
└── cli/       binaire : `latoile serve`, migrations au démarrage
web/           React + Vite + Tailwind, mobile-first, embarqué via rust-embed
```

Graphe : `core` au centre ; `app` dépend de `core` et des ports ; `agents/preview/github/vault` implémentent les ports ; `server` ne parle qu'à `app` ; `cli` assemble. Aucune dépendance remontante.

### 5.2 Séquence critique

```mermaid
sequenceDiagram
    participant Toi
    participant S as server
    participant A as app
    participant M as Manager (ACP persistant)
    participant F as CodeurFrontend (ACP éphémère)
    participant P as preview
    participant DB as SQLite

    Toi->>S: POST /projects/:id/messages "fais la page login"
    S->>A: SendMessage
    A->>M: resume + message + contexte projet
    M-->>A: réponse + actions [CreateTask, StartRun]
    A->>DB: TASK + RUN + EVENT
    A->>F: spawn run (tâche + spec + skill frontend)
    F->>F: code, commit sur work_branch
    F-->>A: RunFinished + summary
    A->>P: EnsurePreview
    P-->>A: PreviewReady
    A-->>Toi: SSE : réponse + RunFinished + PreviewReady
    A->>DB: TASK → review, spawn Reviewer
    A-->>Toi: SSE ApprovalRequested
    Toi->>S: POST /approvals/:id granted
    A->>DB: TASK → done
```

### 5.3 Contrat API (V1)

| Méthode | Route | Rôle |
|---|---|---|
| GET | `/api/projects`, `/api/projects/:id` | liste / détail |
| POST | `/api/projects` | créer + lier repo |
| GET | `/api/github/repos` | picker de repo |
| GET/POST | `/api/projects/:id/messages` | fil Manager |
| GET/PATCH | `/api/projects/:id/tasks` | board, réordonnancement |
| POST | `/api/spec-versions/:id/approve` | valider une spec |
| GET | `/api/runs/:id` | détail (événements, diff) |
| POST | `/api/approvals/:id` | `{granted\|rejected, comment}` |
| GET | `/api/projects/:id/preview/*` | reverse proxy dev server (token requis) |
| GET | `/api/events?after=<seq>` | SSE, reprise par curseur |
| GET | `/api/roles` | rôles + skills |

Erreurs : `{code, message}` ; jamais de chaîne interne renvoyée au client (leçon V-H3 de l'audit Firetower).

### 5.4 Écrans (mobile-first)

1. **Inbox** — approvals pending + runs bloqués
2. **Projet** — chat Manager / board tâches / preview (viewport mobile par défaut, toggle desktop)
3. **Review** (P0) — diff + verdict reviewer + **maquette cible côte à côte du rendu**
4. **Nouveau projet** — pick repo → brief initial au Manager

## 6. Stack Decision Record

| Couche | Choix | Justification | Rejeté |
|---|---|---|---|
| Langage backend | Rust | crate officielle `agent-client-protocol` v2, réutilisation patterns AionCore, expérience Firetower | Bun/Hono — coupe des crates, runtime en plus |
| HTTP | axum 0.8 | choix des deux références | actix |
| DB | SQLite + sqlx, migrations embarquées | mono-utilisateur self-hosté | Postgres — multi-tenant seulement |
| Frontend | React + Vite + Tailwind, embarqué (rust-embed) | binaire unique, zéro serveur Node | Next.js, Electron |
| Canal agents | crate `agent-client-protocol` + spawn supervisé (pattern aionui-process) | statut/permissions/cancel structurés | tmux/PTY — dette documentée |
| Preview | subprocess dev server + reverse proxy axum + SSE reload | HMR traverse le proxy | conteneurs — réservé multi-tenant |
| GitHub | REST + token chiffré (vault) | éprouvé | OAuth device-flow — utile aux autres utilisateurs |
| Temps réel | SSE `/events` mono-canal | suffisant solo | WS bidirectionnel |
| Style | monolithe modulaire, workspace Cargo | équipe de 1 | microservices |

## 7. Risques

| Risque | Prob. | Impact | Mitigation |
|---|---|---|---|
| ACP perd des capacités natives des CLIs | Moyenne | Moyen | connecteurs directs claude/codex possibles plus tard (precedent AionCore) ; le port `agents/` isole le choix |
| Skills rôles à écrire (manager, backend, frontend, reviewer) | Certaine | Moyen | chantier séparé, commence par améliorer app-architect-brainstorm (Phase 4.6) |
| Deux runs se marchent dessus sur la branche unique | Moyenne | Moyen | invariant « un run actif par tâche » + dispatch séquentiel par défaut ; D5 réversible en V2 |
| Dev servers zombies sur le VPS | Moyenne | Faible | supervision + reap par identité de processus (pattern aionui-process) |
| Scope creep vers le SaaS | Moyenne | Élevé | D1 : tout multi-tenant est hors périmètre écrit |

## 8. Hors périmètre (écrit, pour ne pas re-débattre)

Multi-utilisateurs et auth au-delà du token · équipes configurables · branche par run et parallélisme · preview mobile émulateur/desktop · chat direct avec les exécutants · conteneurisation des previews.

---
*Spécification produite par app-architect-brainstorm. Aucun code source généré.*
