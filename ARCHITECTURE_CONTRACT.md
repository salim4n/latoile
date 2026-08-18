# Architecture Contract — LaToile

Verifiable rules. Any PR that violates one is rejected, regardless of functional value.

## 1. Layers and dependencies

- `core` has no dependencies and performs no I/O. Native `async fn` port methods
  are contracts only; `core` owns no runtime.
- `app` use cases and supervision orchestrate through `core` ports. SQL is
  confined to its explicit `app/src/store` persistence adapter.
- `server` contains all of axum. Handlers extract, validate and delegate; task
  position reordering is the documented plain-data store exception.
- Adapters (`agents`, `preview`, `github`, `capture`, `vault`, persistence) implement the ports; the domain never names them.

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
  `preview/`; Git commands go through `github/`; isolated Chromium capture
  goes through `capture/`. These are the only process spawning adapters.
- New runs receive explicit `ProjectId` context before persistence. A failed ACP
  handshake must not leave an active task/run row (ADR-007).
- Permissions hard-reject `.env`, Docker and workspace escape. Read-only tools
  may run once; executor mutations create a sanitized, exact-once `Approval`.
  Timeout, cancellation and restart close pending permission decisions.
- The Manager never receives destructive execution permissions; it does not write code.
- Architect discovery is read-only and receives the complete ordered
  `app-architect-brainstorm` bundle. The session persists its SHA-256 and
  operating mode; a missing reference is a hard failure, never a thin fallback.
  The first turn must be a decision-rich question. A premature
  `ready_to_draft` is recentered once in the same ACP session without inventing
  an owner answer; a second attempt fails closed. After a durable owner answer,
  one invalid question/readiness contract may likewise be repaired in that
  same pinned session without replaying or fabricating owner input; the repaired
  turn is revalidated from scratch and a second invalid contract fails closed.
- Architect generation runs only in a detached worktree. Its permission scope
  permits static `.md`/`.html` files under one server-selected `design/v…/`
  root and rejects shell, path traversal and every production/config mutation.
  Only a bounded, inventory-complete package commit may fast-forward the live
  checkout (ADR-010).
- A visual manifest enumerates every package file and declares each P0 screen,
  state, locale, viewport, scale factor, stable comparison id and mockup. HTML
  pins the declared metadata and the shared design-token digest.
- Raw ACP tool input and hidden reasoning never enter events, run artifacts,
  approval payloads, logs or canary evidence.

## 4. Data

- Embedded migrations, applied at startup; never edit a merged migration destructively.
- Partial-unique invariants (active run/task, preview/project, approved spec/project) are DB indexes **and** state-machine guards.
- `EVENT` is append-only; `seq` is the only SSE cursor.
- Design artifacts never go into the DB (ADR-003).
- Architecture metadata pins skill digest, operating mode, package digest,
  manifest digest, commit SHA and tree SHA. Package bytes remain in Git; session + draft +
  creation event persist atomically.
- A spec can transition to approved only with a fresh verification matching
  every pinned digest. Supersession, approval, project status, waiting-task
  binding and the audit event persist in one transaction. Later design-tree
  drift blocks artifact reads, dispatch and Reviewer context until a new draft.
- Every P0 scenario pins route, synthetic fixture, theme, readiness selector,
  measured selectors and allowed masks. Before approval, isolated Chromium
  stores a real PNG, canonical DOM geometry and accessibility tree plus
  browser/font/environment hashes under the LaToile home. SQLite stores only
  bounded hashes/status; a ready baseline is immutable and missing or failed
  required baselines block approval and executor dispatch.
- Accessibility canonicalization removes only the `RootWebArea` transport URL
  that necessarily changes from `about:blank` baseline installation to the
  loopback live route. Capture V3 installs the same synthetic document base in
  both contexts so relative links resolve identically; semantic destination
  changes remain measured evidence.
- After a finished frontend run, the live route is replayed with that exact
  scenario contract. Any ready preview is first marked stale and its process is
  recycled, including after corrective runs, so capture cannot observe code
  cached by the previous executor. Chromium inherits no service environment
  and may reach only the supervised loopback origin; every other request and
  WebSocket is blocked. The server stores immutable render, pixel diff,
  heatmap, DOM and accessibility changes plus environment hashes. Fixed domain
  thresholds — never Reviewer prose — classify invalid, blocking, reservation
  or passed.
- Every Reviewer run stores one immutable `reviewed_run_id`. Reviewer V2 may
  submit judgement and visual applicability only. The server loads the complete
  evidence set for that finished executor run and current project/spec; model
  echoes cannot select or override it. The server rebuilds ids, hashes, status
  and metrics from SQLite, canonicalizes failed gates to
  `changes_requested`, and permits a grant only when `trusted_v2` and
  `approvable` are both true. Legacy V1 payloads remain readable but untrusted.
- Finished runs may store only bounded evidence: base/head SHA, lifecycle
  activity, commits, changed paths and diff statistics. Raw diffs stay in Git.
- A review rejection has an immutable owner comment and at most one linked
  corrective run. That run produces a distinct evidence set against the same
  approved baseline, so original decision and correction remain auditable. The
  Reviewer completes before a human review approval exists.
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
  runtime-validated by `reviewPayload.ts`; only its V2 server gate can enable
  approval, while malformed and legacy payloads degrade safely.
- The owner sees structured package findings and the exact static gallery from
  the pinned commit before the approval action becomes available.
- Baseline capture starts automatically after immutable validation. The owner
  sees progress, actionable failures, the authenticated real PNG and its
  browser/DOM/accessibility hashes before approval is enabled.
- Review renders only authenticated server artifacts tied to the V2 envelope.
  It exposes scenario, viewport and locale selection; side-by-side, keyboard-
  operable overlay and heatmap modes; server metrics; immutable provenance; and
  actionable invalid-capture reasons. A missing or non-approvable V2 gate keeps
  approval disabled at 390px and desktop widths.
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
- Browser-policy and immutable-repeat tests are hermetic. The installed-Chrome
  capture test is explicit/opt-in and proves a real PNG plus DOM/AX evidence.

## 9. Recovery and operations

- Startup reconciliation finishes before the HTTP listener opens. Every
  active pre-restart run becomes lost through the domain state machine;
  pending permissions close fail-safe and executor tasks become re-dispatchable.
- A fresh preview registry invalidates every active preview row and clears its
  PID. Runtime health polling does the same for an owned process that exits.
  LaToile never signals a PID loaded from SQLite; service-level cgroups reap
  crash orphans without risking PID reuse.
- A state backup is a consistent SQLite snapshot plus its matching external
  root key. Restore validates a disposable copy, verifies all encrypted rows,
  never overwrites live files and never removes project repositories.
- `scripts/release-smoke.sh` starts the release binary on disposable state and
  proves embedded assets, migrations, database health and backup/restore.
