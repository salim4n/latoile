# Guardian Checklist — LaToile

Run the executable guardian from the repository root before every merge:

```sh
./scripts/guardian.sh
```

It fails on a boundary violation and then runs strict Clippy, all hermetic Rust
tests, web lint/tests/build, and the Python canary safety tests. It does **not**
run the paid, stateful real-provider canary.

| # | Boundary | Executable or review proof |
|---|---|---|
| 1 | `core` stays dependency-free and I/O-free | Guardian rejects runtime/I/O dependencies in `crates/core/Cargo.toml` |
| 2 | HTTP stays in `server` | Guardian rejects `axum::` outside `crates/server/` |
| 3 | SQL stays in persistence adapters | Guardian rejects `sqlx::query` outside `app/src/store` and `vault` |
| 4 | Process spawning stays in adapters | Guardian allows process commands only in `agents`, `preview` and `github` |
| 5 | Handlers delegate | Review every changed handler: extract → validate → use case/port → DTO; task position is the documented plain-data exception |
| 6 | Internal errors stay internal | Route tests assert `{code,message}`; review new mappings for adapter-chain leakage |
| 7 | Secrets stay vault-owned | Review adapter changes; never log secret values, authorization headers or provider output |
| 8 | ACP permissions stay fail-closed | Agent/app/server tests cover hard deny, sanitized block, exact-once decision, timeout, cancellation and restart |
| 9 | External payloads degrade safely | SSE reads stay in `events.ts`; Reviewer JSON stays behind `parseReviewPayload` |
| 10 | Components use transport modules | Guardian rejects direct `fetch` outside `api.ts` and `events.ts` |
| 11 | Persistence migrations stay append-only | Review: add a numbered migration; never rewrite a merged migration |
| 12 | Reviewer precedes the owner | Supervision/flow tests prove executor → preview → Reviewer → approval; malformed/failing review becomes a non-deliverable fallback |
| 13 | Delivery proves selected code | Delivery/GitHub tests cover clean branch, origin, SHA ancestry, exact remote SHA, partial failure and existing PR reuse |
| 14 | LaToile never merges | Guardian rejects a GitHub merge API; manual review confirms no merge route/button |
| 15 | Default suites stay hermetic | Guardian runs without provider auth or GitHub network calls; local loopback process fixtures are self-contained |
| 16 | V1 completion has real evidence | Run the opt-in [real-provider canary](v1-canary.md); retain bounded ids, cursor, equal SHAs and PR URL |

## Anti-drift rules

- Split a production module approaching 400 lines in the same change, unless
  its header explains why the behavior is one cohesive unit. Focused colocated
  tests do not count as production responsibilities.
- A capability added to a provider CLI is represented behind an existing or
  new `core` port; server handlers never call it directly.
- New task/run rows are persisted only after the ACP handshake succeeds. Pass
  the project explicitly to pre-persistence adapter calls (ADR-007).
- Comments and diagrams describe executable behavior. Remove a promise as soon
  as the corresponding pass ships.
- Update `architecture-spec.md`, `adrs.md`, this checklist and
  `ARCHITECTURE_CONTRACT.md` in the same PR that changes a structural decision.
