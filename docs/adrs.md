# ADR-001 — Modular Rust monolith + SQLite + single binary; no AionCore fork

- **Date**: 2026-08-15
- **Status**: accepted

## Context

LaToile shares roughly 80% of its technical DNA with AionCore (local Rust daemon, CLI agents, HTTP + realtime, SQLite). Three options: fork AionCore, consume it as a dependency, or rewrite while reusing its patterns.

## Decision

Rewrite as a modular monolith in a Cargo workspace, with selective reuse: the official `agent-client-protocol` crate, the `aionui-process` supervision pattern (identity-gated orphan reaping), and the spawn builder policy (env scrubbing, kill_on_drop, process-tree kill).

## Rationale

- LaToile's unit is the **project**; AionCore's is the **conversation**. Forking would push that domain mismatch into every table and every screen.
- Forking an active third-party project (24 crates, fast release cadence) makes their roadmap a permanent dependency.
- AionCore's licensing is contradictory (Cargo.toml says MIT, LICENSE says Apache-2.0) — patterns are reimplemented, no verbatim copying.
- The deployment skeleton (single binary, SQLite, migrations at startup, embedded assets) is already mastered (Firetower).

## Consequences

+ Clean domain from the first commit; a pure `core` (zero I/O) from day one.
− We give up the multi-agent catalog, JWT/CSRF, `aionrs`, and built-in MCP — to be reintroduced on demand, never by anticipation.

---

# ADR-002 — ACP channel for all agents; two agent lifecycles (persistent Manager, ephemeral executors)

- **Date**: 2026-08-15
- **Status**: accepted

## Context

Three observed approaches: PTY/tmux (Firetower — heuristics-based status, documented debt), ACP via pinned adapters (IgnitionRAG acp-runner), direct CLI for claude/codex + ACP for the rest (AionCore — which reached this point *after* building generic ACP infrastructure).

## Decision

All agents go through the `agent-client-protocol` v2 crate behind an `agents/` port defined in `core`. The Manager holds a persistent session per project (resumed on each message); executors are ephemeral runs (spawn → task → exit). Permissions follow the AionCore pattern: allow/approval/reject heuristics (auto-reject: `.env`, absolute paths, `docker`), then a human approval queue.

## Rationale

- Structured status, cancellation, permissions, and usage — exactly what Firetower lacks for Codex.
- The port isolates the choice: if ACP loses native capabilities (the AionCore lesson), a direct `claude`/`codex` connector can replace the adapter without touching the domain.

## Consequences

+ One agent abstraction across the whole system.
− Dependency on versioned ACP adapters; the handshake must verify versions and politely refuse incompatible ones (IgnitionRAG pattern: pinned version + canary prompt).

---

# ADR-003 — Design artifacts live in the project repo; the DB stores metadata only

- **Date**: 2026-08-15
- **Status**: accepted

## Context

Specs (domain, ER, ADRs, HTML mockups) are first-class artifacts (D7: the mockup is the visual contract). Two storage options: the database, or the filesystem inside the repository.

## Decision

`design/` inside the project repo; `SPEC_VERSION.design_dir` points at it. Git provides history, diff, and review. Agents read and write these files like any other project artifact.

## Consequences

+ Free git visibility; mockups are served statically for the Review screen (mockup next to live render).
− The "approved version ↔ directory contents" consistency rests on the workflow (commit before approval), not on the database.

---

# ADR-004 — Single work branch per project in V1

- **Date**: 2026-08-15
- **Status**: accepted, reversible in V2

## Context

Firetower isolates each session in its own branch/worktree. LaToile wants an always-coherent preview and a simple model.

## Decision

One project = one `work_branch`; all runs commit to it; the preview serves it. Sequential is the rule (one active run per task, ordered dispatch). Parallelism (branch-per-run + integration) is explicitly deferred.

## Consequences

+ Preview is never ambiguous; no intermediate merge step; trivial mental model.
− Collision risk if two runs touch the same files — accepted and monitored; if the pain shows up, a new ADR switches to branch-per-run.
