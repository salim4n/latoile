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
| 16 | V1 differentiation has real evidence | Run the opt-in [greenfield visual-contract canary](v1-canary.md) from a clean commit; retain bounded Architect provenance, blocking/replacement evidence ids, unchanged baseline, Reviewer event ordering, cursor, equal SHAs and PR URL |
| 17 | Restart state is fail-safe | Server tests prove pre-listener run/permission/preview recovery and runtime preview death reconciliation |
| 18 | Production artifact is executable | CI and release sign-off run `scripts/release-smoke.sh` against disposable state |
| 19 | Backup pairs database and key | CLI tests prove integrity, key verification, no overwrite and workspace preservation |
| 20 | Architect skill input is complete and pinned | Bundle tests require every reference and prove any byte change changes the SHA-256 |
| 21 | Architect output cannot escape | ACP adapter tests prove package-only permission scope, mandatory static inventory, rejected source mutation, unchanged live HEAD on refusal and exact fast-forward on success |
| 22 | Spec approval is immutable | Domain/app/agent/server/UI tests prove complete manifest metadata, Git/tree/content revalidation, drift refusal, atomic approval, exact-commit artifact rendering and pre-dispatch revalidation |
| 23 | Visual baselines are real and deterministic | Capture/app/server/UI tests prove network-blocked Chromium capture, explicit readiness/selectors/masks, PNG + DOM + AX hashes, immutable repeat, actionable failure and pre-approval/pre-dispatch gates |
| 24 | Live visual evidence is measured, not narrated | Capture/app/server tests prove exact loopback-only navigation, cleared browser environment, approved masks, real render/diff/heatmap artifacts, immutable hashes, fixed thresholds, invalid capture and a known 16 px regression |
| 25 | Reviewer V2 cannot replay or invent evidence | App/server tests bind one immutable reviewed run, load the full current project/spec evidence set server-side, prove model echoes cannot override it, canonicalize failed gates, and refuse grants for V1 or non-approvable V2 payloads |
| 26 | Owner decisions use real visual artifacts | Web API/UI tests prove bearer-authenticated baseline/render/heatmap loading, scenario/viewport/locale switching, side-by-side/overlay/diff modes, metrics/provenance, invalid recovery and disabled failed gates; flow tests prove one correction creates distinct evidence against the same baseline |
| 27 | Corrective capture cannot reuse an old preview process | App and driver tests prove frontend completion emits `PreviewStale`, recycles the supervised process before capture and does so idempotently; the real-provider canary must prove `blocking → passed` |
| 28 | Transport metadata cannot create a false visual reservation | Unit tests preserve semantic link URLs while excluding only the AX root transport URL; installed-Chromium tests prove exact HTTP bytes with relative links pass, changed link destinations reserve, and the deliberate 16 px regression still blocks under capture V3 |
| 29 | Architect cannot skip the first owner challenge | Agent prompt tests require a first-turn question; HTTP flow tests prove one premature `ready_to_draft` is recentered in the same session and a second attempt fails closed |
| 30 | Architect protocol drift cannot destroy a valid discovery silently | Agent and HTTP tests prove one post-answer contract repair stays in the same session with no owner input, enforces current-or-later phases, and fails closed on a second invalid turn |
| 31 | Reviewer verdicts do not invent hidden acceptance scope | Driver prompt tests require a concrete-blocker rubric, preserve server-owned visual truth and classify optional improvements or unstated scope as non-blocking |
| 32 | Architect cannot self-attest its skill provenance | Package tests deliberately emit wrong model provenance and prove the adapter binds schema, exact skill digest and mode from the server before validation and commit |
| 33 | Architect package repair is bounded and never weakens confinement | Adapter tests prove one invalid gallery is repaired in the same ACP/worktree, a third invalid result fails after exactly two repairs, and path escape still fails immediately without integration |

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
