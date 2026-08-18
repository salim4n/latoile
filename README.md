# LaToile

**An AI-native, self-hosted project workbench.** You talk to a Manager agent; it orchestrates a fixed-role agent team — Architect, Backend, Frontend, Reviewer — that specs, builds, and verifies. You watch the application take shape live (web preview, mobile viewport first), and nothing merges without your explicit approval.

> *La Toile* — the parallel network your agents travel through while you stay on the surface.

## Why

Coding agents block. They finish a turn, hit a permission prompt, or drift off-spec — and you are the bottleneck. Existing tools answer with either a chat window per agent (you are the router) or autonomous loops (you are the janitor). LaToile answers with **project management**: one Manager per project, a spec that precedes code, tasks dispatched to specialized roles, and a single inbox of decisions waiting for you.

## How it works

```
You ──chat──► Manager ──► Architect ──► Spec v1 (design/ in the repo)
                │                            │
                │                            ▼
                │              Tasks ──► Backend / Frontend runs ──► commits
                │                            │
                │                            ▼
                │              Reviewer ──► verdict + diff
                │                            │
                ◄────── approval inbox ◄─────┘
```

- **The project is the central entity** — not the conversation, not the session.
- **Spec before code** — every task references an approved, versioned specification. Design artifacts (domain model, ER diagrams, HTML mockups) live in the project's own `design/` directory, so git gives you history, diff, and review for free.
- **Mockups are the visual contract** — the HTML mockups produced at design time sit side-by-side with the live render in the Review screen. The frontend agent builds toward a target, not a vibe.
- **Fixed roles, dedicated skills** — each role is bound to its own skill file (system prompt + playbook). The Architect runs a structured architecture-brainstorm method; the Reviewer never merges, only recommends.
- **Human approval is a hard invariant** — enforced by the domain state machine, not by convention.
- **Live preview** — the project's dev server is supervised and reverse-proxied into the UI, mobile viewport by default. When a frontend run finishes, the preview reloads itself.

## Architecture at a glance

| Layer | Choice | Why |
|---|---|---|
| Backend | Rust (axum), modular monolith in a Cargo workspace | Single binary on a VPS; the official `agent-client-protocol` crate is Rust |
| Agent channel | [Agent Client Protocol](https://agentclientprotocol.com) with supervised spawning | Structured status, permissions, cancellation — no terminal scraping |
| Database | SQLite via sqlx, embedded migrations | Single-user, self-hosted — Postgres can wait for multi-tenancy |
| Frontend | React + Vite + Tailwind, embedded in the binary | No Node server to run; mobile-first is CSS, not a framework |
| Realtime | Server-Sent Events with a monotonic cursor | One channel is enough for one user |
| Secrets | Envelope encryption (XChaCha20-Poly1305), root key outside the DB | A database backup alone opens nothing |

The full design — decisions D1–D10, the ER model, the API contract, the risk register — is in [`docs/architecture-spec.md`](docs/architecture-spec.md).

## Running

Requirements: Rust 1.85+, Node 22+ with pnpm, Git, the provider CLIs (`claude`
and/or `codex`), and at least one complete ACP pair on PATH
(`claude` + `claude-agent-acp` or `codex` + `codex-acp`).

```sh
cd web && pnpm install && pnpm build   # build the embedded web UI (repeat per change)
cargo run -p latoile-cli -- serve      # or a release binary: cargo build --release -p latoile-cli
```

`serve` prints the local URL and the bearer token — paste it into the UI (set `LATOILE_TOKEN` to choose your own; `latoile token` prints it back). State lives in `~/.local/share/latoile` (`--home` overrides): the SQLite database and the vault's `master.key`.

Open **Settings** to connect Claude or Codex with the provider's own login
flow and choose the provider used by each fixed role. Provider credentials
stay owned by their CLI; LaToile stores only the role routing. Store the
GitHub token through the encrypted vault (interactive input is hidden):

```sh
cargo run -p latoile-cli -- secret set github_token
cargo run -p latoile-cli -- secret list       # names only, never values
cargo run -p latoile-cli -- backup create --output /safe/private/latoile-backup
```

The web UI is embedded via rust-embed: **release builds bake `web/dist` into the binary; debug builds read it live from disk**, so `pnpm build` + refresh suffices while developing — or `pnpm dev` for the Vite server, which proxies `/api` to port 7700. A placeholder `index.html` committed in `web/dist` keeps a fresh clone compiling before the first web build. Web checks: `pnpm lint`, `pnpm test` (vitest), `pnpm build` (typecheck + bundle).

## Documentation

- [`docs/architecture-spec.md`](docs/architecture-spec.md) — complete architecture specification
- [`docs/adrs.md`](docs/adrs.md) — accepted decisions and their rejected alternatives
- [`ARCHITECTURE_CONTRACT.md`](ARCHITECTURE_CONTRACT.md) — verifiable rules (layers, secrets, errors, tests)
- [`docs/guardian-checklist.md`](docs/guardian-checklist.md) — anti-drift checks to run before merging
- [`docs/v1-canary.md`](docs/v1-canary.md) — opt-in real-provider vertical-slice proof and cleanup
- [`docs/operations.md`](docs/operations.md) — release smoke, systemd, restart recovery and backup/restore

## Status

**V1 vertical slice implemented and canary-verified.** The product provisions a real GitHub checkout, runs the Manager and executor team through authenticated Claude or Codex ACP adapters, blocks for sanitized permission decisions, records bounded Git evidence, refreshes the live preview, runs the Reviewer before asking the owner, and delivers an approved work branch through an idempotent Pull Request. The web UI implements the five design-contract screens in French and English.

The default Rust, web and Python safety suites are hermetic. The separate opt-in [real-provider canary](docs/v1-canary.md) proved the complete journey with Codex ACP, including an exact local/remote SHA and a live Pull Request. This is evidence for the V1 path, not a claim that the deferred V2 scope below exists.

### Roadmap

- [x] Cargo workspace skeleton with a pure `core` from commit one
- [x] Real GitHub checkout provisioning and one project work branch
- [x] Native provider login/status/logout and persisted role routing
- [x] Project + persistent Manager chat + executable action blocks
- [x] Dedicated Manager, Architect, Backend, Frontend and Reviewer skills
- [x] Executor supervision with bounded Git evidence and fail-closed ACP permissions
- [x] Supervised live preview with auto-reload
- [x] Reviewer-before-human flow, corrective runs and audited decisions
- [x] Review screen with verdict, findings, diff and spec/render comparison
- [x] Owner-controlled push, remote SHA verification and idempotent Pull Request
- [x] Opt-in real-provider V1 vertical-slice canary
- [x] Blocking startup recovery, preview health reconciliation, paired backup/restore and release smoke

Deferred beyond V1: multi-user auth, configurable teams, branch-per-run parallelism, WebSocket preview proxying, non-web previews and automatic merge. The [verified V1 limits](docs/architecture-spec.md#71-verified-v1-limits) also call out the unwired automatic Architect pass, fresh Manager context assembly, project status promotion and screenshot/pixel-diff review.

## Contributing

The repository is active. Run [`scripts/guardian.sh`](scripts/guardian.sh) before proposing a change; architecture or product feedback belongs in GitHub issues. LaToile never merges a delivered Pull Request automatically.

## License

AGPL-3.0-only. If you serve a modified LaToile over a network, you publish your changes.
