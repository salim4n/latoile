# Guardian Checklist — LaToile

Run before every merge. Everything must be ✅.

| # | Check | Command / method | Status |
|---|-------|------------------|--------|
| 1 | `core` stays pure | `grep -rn "tokio\|sqlx\|axum\|reqwest" crates/core/src/` → empty | ☐ |
| 2 | HTTP confined to the server | `grep -rln "axum::" crates/ \| grep -v crates/server` → empty | ☐ |
| 3 | SQL centralized | `grep -rn "sqlx::query" crates/ \| grep -v "store\|vault"` → empty | ☐ |
| 4 | Spawn centralized | `grep -rn "Command::new" crates/ \| grep -v "crates/agents\|crates/preview"` → empty | ☐ |
| 5 | Handlers hold no logic | review: every handler does extract → validate → delegate | ☐ |
| 6 | No error leakage | review: no response contains an internal error chain | ☐ |
| 7 | Secrets only via vault | `grep -rn "sk-\|Bearer\|token" crates/ --include="*.rs" \| grep -v "vault\|test"` → reviewed | ☐ |
| 8 | SSE validated on the web side | `grep -rn "as SessionEvent\|as .*Event" web/src/` → empty | ☐ |
| 9 | No mocks in production | `grep -rn "fixtures\|mock" web/src/ \| grep -v "fixtures/"` → empty | ☐ |
| 10 | Tests hermetic and green | `cargo test` on a clean machine; state transitions covered | ☐ |

## Anti-drift (lessons from the Firetower audit, 2026-08-15)

- Any file approaching 400 lines is a split candidate **in the same PR** that takes it there.
- A capability added to an agent CLI is never hardcoded outside the `agents/` port.
- Comments describing behavior must match the code; a false comment is worse than none (real cases: `db.rs:601`, `transport.rs:80`, `events.ts:3` in Firetower).
- Docs (`architecture-spec.md`, `adrs.md`) are updated in the PR that changes the decision.
