# Architecture Contract — LaToile

Verifiable rules. Any PR that violates one is rejected, regardless of functional value.

## 1. Layers and dependencies

- `core` has no dependencies and performs no I/O. Native `async fn` port methods
  are contracts only; `core` owns no runtime.
- `app` use cases and supervision orchestrate through `core` ports. SQL is
  confined to its explicit `app/src/store` persistence adapter.
- `server` contains all of axum. Handlers extract, validate and delegate; task
  position reordering is the documented plain-data store exception.
- Adapters (`agents`, `preview`, `github`, `vault`, persistence) implement the ports; the domain never names them.

Automated check: `scripts/guardian.sh`. Its boundary probes include:

```sh
rg '^\s*(tokio|sqlx|axum|reqwest)\s*=' crates/core/Cargo.toml  # → empty
rg -l 'axum::' crates -g '*.rs' | rg -v '^crates/server/'      # → empty
rg -l 'sqlx::query' crates -g '*.rs' | rg -v '^crates/(app/src/store|vault)/' # → empty
```

## 2. Files

- One use case = one file in `app/src/use_cases/`. A handler = a function that delegates.
- A production module approaching 400 lines is split unless its header explains
  why the behavior remains one cohesive unit. Colocated focused tests may take a
  source file past that guide; a 2,000-line catch-all module is never accepted.
- State machines (`Task`, `Run`, `SpecVersion`, `Approval`, `Preview`,
  `Delivery`) live in `core`, with exhaustive, tested transitions. No state
  transitions exist outside `core`.

## 3. Agents

- Every agent process goes through `agents/`; preview dev servers go through
  `preview/`; Git commands go through `github/`. These are the only process
  spawning adapters.
- New runs receive explicit `ProjectId` context before persistence. A failed ACP
  handshake must not leave an active task/run row (ADR-007).
- Permissions hard-reject `.env`, Docker and workspace escape. Read-only tools
  may run once; executor mutations create a sanitized, exact-once `Approval`.
  Timeout, cancellation and restart close pending permission decisions.
- The Manager never receives destructive execution permissions; it does not write code.
- Raw ACP tool input and hidden reasoning never enter events, run artifacts,
  approval payloads, logs or canary evidence.

## 4. Data

- Embedded migrations, applied at startup; never edit a merged migration destructively.
- Partial-unique invariants (active run/task, preview/project, approved spec/project) are DB indexes **and** state-machine guards.
- `EVENT` is append-only; `seq` is the only SSE cursor.
- Design artifacts never go into the DB (ADR-003).
- Finished runs may store only bounded evidence: base/head SHA, lifecycle
  activity, commits, changed paths and diff statistics. Raw diffs stay in Git.
- A review rejection has an immutable owner comment and at most one linked
  corrective run. The Reviewer completes before a human review approval exists.
- `DELIVERY.local_sha = DELIVERY.remote_sha`; URL presence must agree with
  `pushed` versus `pull_request_open` status.

## 5. Delivery

- Project creation accepts repository identity and owner choices, never a host
  checkout path or claimed default branch. `WorkspaceProvisioner` discovers and
  returns canonical facts.
- `DeliverProject` refuses active runs, pending approvals, non-done tasks,
  missing granted Reviewer decisions and missing executor SHA evidence.
- `WorkBranchPublisher` verifies checkout containment, origin, current branch,
  clean worktree and approved-SHA ancestry; it pushes without force and then
  verifies the remote ref.
- Pull Request creation is idempotent. LaToile has no merge operation.

## 6. Errors and secrets

- Error responses: `{code, message}`; internal details go to `tracing`, never to the client.
- No plaintext secrets: everything goes through `vault` (XChaCha20-Poly1305, root key outside the DB). Secret values are never logged.
- Every route except `/api/health` sits behind the token. Query-token auth is
  restricted to preview proxy paths for iframe compatibility.

## 7. Frontend

- Data fetching only through the transport module (generated client or hooks); no direct `fetch` in components.
- The bearer-aware SSE reader is confined to `events.ts`, parses frames without
  casts and exposes only event kind/payload strings. External Reviewer JSON is
  runtime-validated by `reviewPayload.ts` and degrades safely when malformed.
- Mobile-first: every screen is designed at 390px viewport first.
- No mock data outside a `fixtures/` directory clearly excluded from real routes (Firetower lesson V-M2).

## 8. Tests and evidence

- `cargo test --workspace`, web checks and Python safety tests are hermetic:
  green on a clean machine with no external service.
- Every `core` state machine has transition tests, including refused transitions.
- Process supervision has process-group kill, cancellation, timeout and
  readiness tests.
- The real-provider V1 canary is opt-in and excluded from default suites. A
  completion claim needs bounded canary evidence: run ids, event cursor,
  local/remote SHA equality and Pull Request URL.
