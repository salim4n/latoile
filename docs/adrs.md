# ADR-001 — Modular Rust monolith + SQLite + single binary; no AionCore fork

- **Date**: 2026-08-15
- **Status**: accepted

## Context

LaToile shares roughly 80% of its technical DNA with AionCore (local Rust daemon, CLI agents, HTTP + realtime, SQLite). Three options: fork AionCore, consume it as a dependency, or rewrite while reusing its patterns.

## Decision

Rewrite as a modular monolith in a Cargo workspace, with selective reuse: the official `agent-client-protocol` crate and the process-supervision patterns observed in AionCore (`kill_on_drop`, dedicated process groups and bounded shutdown).

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

All agents go through the `agent-client-protocol` v2 crate behind an `agents/` port defined in `core`. The Manager holds a persistent session per project (resumed on each message); executors are ephemeral runs (spawn → task → exit). Permissions follow a fail-closed allow/ask/reject policy: `.env`, Docker and paths outside the project checkout are hard-denied; executor mutations create a sanitized human approval; Manager mutations are rejected.

Each fixed role is routed to either Claude or Codex through a persisted
setting. The provider's native CLI owns login/status/logout; LaToile only
supervises that interactive flow and then launches the matching ACP adapter.
Changing executor routing applies to the next run. Changing Manager routing
evicts the persistent session so its next message starts with the new adapter.

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

---

# ADR-005 — Reviewer evidence precedes the human decision

- **Date**: 2026-08-18
- **Status**: accepted and canary-verified

## Context

An executor saying “done” is not decision-grade evidence. Asking the owner at that point makes the owner perform the review and turns the approval inbox into a status feed. The product has a dedicated Reviewer role and a visual contract, so the machine review must happen before human attention is requested.

## Decision

When an executor finishes, LaToile persists bounded Git evidence, moves the task to `review`, refreshes the preview, and starts a fresh Reviewer run. The Reviewer receives the task, approved spec excerpts, visual-contract references, base/head SHAs and sanitized artifacts. Only a terminal, schema-validated Reviewer result creates the human review approval.

A granted review moves the task to `done`. A rejected review requires an owner comment and starts exactly one corrective run linked from the immutable decision. Reviewer spawn/transport failure creates an honest fallback `changes_requested` approval instead of silently skipping review.

## Rejected alternatives

- Ask the owner directly after executor completion: cheaper in tokens, but makes every owner a manual reviewer.
- Let the Reviewer modify code: collapses executor and judge into one authority and destroys the value of an independent verdict.
- Drop malformed Reviewer output: hides a failed control; the fallback approval keeps the failure visible and non-deliverable.

## Consequences

+ The owner decides from a localized verdict, diff excerpt and spec/render comparison.
+ No approved task can bypass the Reviewer-before-human ordering.
− Every executor run pays Reviewer latency and provider usage, accepted because owner attention is the scarcer resource.

---

# ADR-006 — GitHub delivery is explicit, verified and never merges

- **Date**: 2026-08-18
- **Status**: accepted and canary-verified

## Context

A green task board does not prove that the checkout being pushed is the code the owner approved. A dirty worktree, wrong branch, mismatched origin, missing executor commit or retry after a partial GitHub failure can publish different code or create duplicate PRs.

## Decision

Delivery is one explicit owner action after all selected tasks are `done`. The app supplies the approved executor SHAs to a dedicated `WorkBranchPublisher` port. The GitHub adapter verifies the canonical checkout, stored origin, exact work branch, clean worktree and SHA ancestry, pushes without force, then reads the remote ref and requires `local_sha = remote_sha`.

The app persists `Delivery(status = pushed)` before calling the PR API. It then finds an existing open PR for the stored head/base pair or creates one and upgrades the same delivery to `pull_request_open`. A retry re-verifies the push and reuses the PR. No port or route exposes merge.

## Rejected alternatives

- Push automatically after review: removes the owner-controlled publication boundary.
- Trust `git push` exit status: does not prove the ref GitHub now serves equals the selected local commit.
- Open a new PR on every retry: turns network ambiguity into duplicate owner work.
- Merge from LaToile: expands an evidence and orchestration tool into a deployment authority.

## Consequences

+ The UI can show a durable PR URL and the exact verified SHA.
+ A PR API outage leaves truthful `pushed` evidence that a retry can complete.
− V1 delivers the whole project work branch; selecting a subset of commits needs branch-per-run integration in a later ADR.

---

# ADR-007 — New runs carry explicit project context before persistence

- **Date**: 2026-08-18
- **Status**: accepted after real-provider canary failure

## Context

The initial implementation resolved an executor directory by loading its run, task and project from SQLite. `DispatchTask` intentionally starts the ACP handshake before saving a new task/run so a failed spawn cannot leave an active database ghost. The first real-provider canary exposed the cycle: the channel needed a row that correctly did not exist yet.

## Decision

`AgentChannel::start_run` receives both `ProjectId` and the transient `Run`. Directory resolution uses the persisted project checkout directly. The new task and run are saved only after the handshake succeeds. Reviewer and corrective runs use the same explicit context.

## Rejected alternatives

- Persist `starting` rows before spawn: requires compensating writes and exposes transient ghosts to the board and restart recovery.
- Put `project_id` permanently on `RUN`: duplicates the `RUN → TASK → PROJECT` relation only to solve a pre-persistence concern.
- Let the agent choose a working directory: crosses the adapter trust boundary and permits workspace escape.

## Consequences

+ Adapter startup has the context it needs without weakening persistence atomicity.
+ The project path remains server-owned and is checked before any process spawn.
− The agent port carries one extra identifier that is derivable after persistence but necessary before it.

---

# ADR-008 — Invalidate process-backed state before startup readiness

- **Date**: 2026-08-18
- **Status**: accepted

## Context

Agent connections and preview registries are process-local. After a restart,
SQLite may still contain `starting`, `running`, `blocked`, `ready` or `stale`
rows, but the new process cannot own the corresponding callback, child handle
or permission responder. A persisted numeric PID is not proof of identity;
the operating system may have reused it.

## Decision

Server assembly performs recovery before returning the HTTP router. Every
active run is observed as `Lost` and follows the existing domain wind-down:
pending permissions reject fail-safe, executor tasks requeue, and a lost
Reviewer produces an owner-visible fallback decision. Every active preview is
marked `error` and its PID is cleared. The periodic driver performs the same
preview reconciliation when an owned ready process exits.

LaToile never kills a PID loaded from SQLite. The supported systemd service
uses `KillMode=control-group` to reap child trees on parent failure; graceful
shutdown still kills adapter-owned process groups directly.

## Rejected alternatives

- Resume an ACP session from its stored id: the responder and transport are
  gone, so this would display false progress and leave permissions unanswerable.
- Signal the stored PID: PID reuse can kill an unrelated process.
- Reconcile on the first timer tick: health could briefly advertise a state
  the new process does not own.

## Consequences

+ HTTP readiness never races stale active state.
+ Lost work has a deterministic next action and an audit event.
+ Crash orphans are reaped by an identity-safe service cgroup.
− A running task does not resume across process restart; it must be dispatched again.

---

# ADR-009 — Backup SQLite and the external vault key as one restore unit

- **Date**: 2026-08-18
- **Status**: accepted

## Context

SQLite is in WAL mode and can be written while a backup runs. Secret rows are
useless without the root key deliberately stored outside the database. A raw
file copy can miss WAL pages; a database-only backup can permanently orphan
credentials; an in-place restore can destroy the last recoverable pair.

## Decision

`latoile backup create` uses SQLite `VACUUM INTO`, checks integrity, opens every
encrypted row with the current root key, and writes the snapshot, root key and
versioned manifest into a private directory. `backup restore` refuses to
overwrite live state. It validates and migrates a disposable copy, verifies
the database/key pair, builds a standalone install database and uses a
`.restore-in-progress` marker while installing both files. It never touches
project checkouts.

## Rejected alternatives

- Copy `latoile.db` directly: unsafe while WAL contains committed pages.
- Back up the database without `master.key`: encrypted secrets would not be recoverable.
- Include checkouts in the state archive: repositories have a separate Git
  durability model and can make the operational backup unbounded.
- Add an overwrite flag: a typo could destroy the only live database/key pair.

## Consequences

+ Backup creation can run against the live SQLite database.
+ Restore failure is non-destructive and wrong-key backups fail validation.
+ Existing repositories survive restore drills unchanged.
− Unpushed repository work needs delivery to GitHub or a separate workspace snapshot.
