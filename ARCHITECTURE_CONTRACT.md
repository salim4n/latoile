# Architecture Contract — LaToile

Verifiable rules. Any PR that violates one is rejected, regardless of functional value.

## 1. Layers and dependencies

- `core` imports **nothing**: no tokio, sqlx, axum, reqwest. Zero I/O, zero async.
- `app` orchestrates through `core` ports (traits); it knows neither axum nor sqlx.
- `server` contains all of axum; it contains **no** logic: extract, validate, delegate to `app`.
- Adapters (`agents`, `preview`, `github`, `vault`, persistence) implement the ports; the domain never names them.

Checks:

```sh
grep -rn "tokio\|sqlx\|axum\|reqwest" crates/core/src/        # → empty
grep -rln "axum::" crates/ | grep -v "crates/server"          # → empty
grep -rn "sqlx::query" crates/ | grep -v "crates/app/src/store\|crates/vault"  # → empty
```

## 2. Files

- One use case = one file in `app/src/use_cases/`. A handler = a function that delegates.
- No file exceeds ~400 lines without a written justification in its header comment (lesson: Firetower's `api.rs`, 2,323 lines).
- State machines (`Task`, `Run`, `SpecVersion`, `Preview`) live in `core`, with exhaustive, tested transitions. No state transitions outside `core`.

## 3. Agents

- Every agent process goes through `agents/`: supervised spawn, `kill_on_drop`, process group, registry enrollment. No `Command::new` anywhere else.
- Permissions: auto-reject on `.env`, absolute paths, `docker`; anything else non-trivial goes through an `Approval`.
- The Manager never receives destructive execution permissions; it does not write code.

## 4. Data

- Embedded migrations, applied at startup; never edit a merged migration destructively.
- Partial-unique invariants (active run/task, preview/project, approved spec/project) are DB indexes **and** state-machine guards.
- `EVENT` is append-only; `seq` is the only SSE cursor.
- Design artifacts never go into the DB (ADR-003).

## 5. Errors and secrets

- Error responses: `{code, message}`; internal details go to `tracing`, never to the client.
- No plaintext secrets: everything goes through `vault` (XChaCha20-Poly1305, root key outside the DB). Secret values are never logged.
- Every route sits behind the token, preview included.

## 6. Frontend

- Data fetching only through the transport module (generated client or hooks); no direct `fetch` in components.
- SSE events are validated (zod) before entering the cache — no casts (Firetower lesson V-M3).
- Mobile-first: every screen is designed at 390px viewport first.
- No mock data outside a `fixtures/` directory clearly excluded from real routes (Firetower lesson V-M2).

## 7. Tests

- `cargo test` is hermetic: green on a clean machine, no external services.
- Every `core` state machine has transition tests, including refused transitions.
- Process supervision has an orphan-reaping test.
