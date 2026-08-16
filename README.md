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

Requirements: Rust (see `rust-toolchain.toml`), Node 22+ with pnpm, and the agent ACP adapters on PATH (`claude-agent-acp` and/or `codex-acp`).

```sh
cd web && pnpm install && pnpm build   # build the embedded web UI (repeat per change)
cargo run -p latoile-cli -- serve      # or a release binary: cargo build --release -p latoile-cli
```

`serve` prints the local URL and the bearer token — paste it into the UI (set `LATOILE_TOKEN` to choose your own; `latoile token` prints it back). State lives in `~/.local/share/latoile` (`--home` overrides): the SQLite database and the vault's `master.key`.

The web UI is embedded via rust-embed: **release builds bake `web/dist` into the binary; debug builds read it live from disk**, so `pnpm build` + refresh suffices while developing — or `pnpm dev` for the Vite server, which proxies `/api` to port 7700. A placeholder `index.html` committed in `web/dist` keeps a fresh clone compiling before the first web build. Web checks: `pnpm lint`, `pnpm test` (vitest), `pnpm build` (typecheck + bundle).

## Documentation

- [`docs/architecture-spec.md`](docs/architecture-spec.md) — complete architecture specification
- [`docs/adrs.md`](docs/adrs.md) — the four founding decisions and their rejected alternatives
- [`ARCHITECTURE_CONTRACT.md`](ARCHITECTURE_CONTRACT.md) — verifiable rules (layers, secrets, errors, tests)
- [`docs/guardian-checklist.md`](docs/guardian-checklist.md) — anti-drift checks to run before merging

## Status

**Working prototype.** The full backend stack exists — domain core, SQLite store, ACP agent channel, encrypted vault, GitHub client, preview supervisor, HTTP/SSE server, CLI — and the web UI (React + Vite + Tailwind) implements the design mockups' screens. The spec was produced by a structured architecture brainstorm informed by audits of two real codebases (a tmux/PTY-based agent control plane, and an ACP-based agent desktop runtime). Every rule in the architecture contract is a lesson from those audits turned into a starting invariant.

### Roadmap

- [x] Cargo workspace skeleton with a pure `core` from commit one
- [x] Agent channel over ACP with permission policy
- [x] Project + Manager chat + task board
- [x] Supervised live preview with auto-reload
- [x] Review screen skeleton: verdict + approve/request-changes
- [ ] Role skills: manager, backend, frontend, reviewer playbooks
- [ ] Orchestrator pass: execute Manager actions, run/review loop, approval side-effects
- [ ] Review screen: diff + verdict + mockup side-by-side (needs reviewer output)

## Contributing

Not open yet — the codebase doesn't exist. Design feedback via issues is welcome.

## License

AGPL-3.0-only. If you serve a modified LaToile over a network, you publish your changes.
