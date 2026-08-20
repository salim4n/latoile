# Real-provider greenfield visual-contract canary

This canary is the opt-in proof for LaToile's complete differentiating journey.
It is outside the default Cargo and web test suites because it consumes real
Claude or Codex sessions, a real installed Chromium and a private disposable
GitHub repository.

## Prerequisites

- `git`, `gh`, `node`, Rust/Cargo and Python 3;
- Google Chrome or Chromium, or `LATOILE_CAPTURE_BROWSER` pointing to it;
- `gh auth login`, with permission to create private repositories and PRs;
- one authenticated provider CLI plus its ACP adapter:
  - Claude: `claude auth status` and `claude-agent-acp`;
  - Codex: `codex login status` and `codex-acp`.

The script fails during `preflight` before creating anything when a binary or
authentication is missing. It reads the GitHub token from `gh` only to place it
in the canary's ephemeral encrypted LaToile vault. It never prints or persists
that token.

## Run

From the repository root, choose the provider that is already authenticated:

```sh
python3 scripts/v1_canary.py run --provider codex --approve-permissions
```

`--approve-permissions` is the explicit owner action allowing sanitized ACP
mutation requests inside this one disposable checkout. Hard-denied operations
remain denied by LaToile's policy.

The canary refuses a dirty LaToile checkout, records its exact commit, creates a
private `latoile-v1-canary-*` repository whose initial Git tree has no files,
and exercises:

1. a real persistent Socratic Architect discovery through the Manager surface, including the mandatory first owner challenge;
2. the complete pinned `app-architect-brainstorm` package and manifest;
3. immutable spec validation, Git commit/tree proof and real Chromium baseline;
4. a frontend executor that serves the approved mockup with a deliberate 16 px regression;
5. server evidence that classifies that regression as `blocking`;
6. a finished Reviewer V2 run before the first human approval request;
7. an explicit owner rejection and one linked visual corrective run;
8. distinct corrective evidence classified `passed` against the same baseline;
9. up to two additional, auditable code-review correction cycles when a real
   Reviewer still reports a concrete blocking finding after visual parity;
10. unchanged approved design bytes, a corrected live preview and
    Reviewer-before-human ordering for every cycle;
11. explicit approval, exact pushed SHA and a live Pull Request.

Success prints the PR URL, the exact local/remote SHA and the path to a bounded
`evidence.json`. The artifact contains only identifiers, counts, statuses,
event sequence numbers, content/Git hashes and the PR URL. Prompts, question
text, owner answers, provider responses, credentials and hidden reasoning are
not retained.

On failure the same artifact records `first_broken_seam` and a concise safe
diagnostic. Architecture failures retain only session status, phase, package
state, fixed failure reason, pinned role provenance and question counts.
Baseline approval failures retain only comparison id, status, fixed failure
code and recovery action; mockup bytes and browser/provider prose stay
excluded. The remote repository is retained so the failure remains inspectable.

## Cleanup

Cleanup is deliberately separate. Copy the exact command printed by the run:

```sh
python3 scripts/v1_canary.py cleanup \
  --evidence .latoile-canary/<run>/evidence.json \
  --confirm <owner>/latoile-v1-canary-<run>
```

The command refuses repositories outside the canary naming boundary or any
confirmation that does not exactly match the evidence. By default it archives
only the remote disposable repository and updates, rather than deletes, the
local evidence. This preserves the PR while making the fixture read-only. Pass
`--delete` only when permanent removal is wanted and the authenticated GitHub
token has the `delete_repo` scope.

## Verified V1 evidence

The completion claim in the README is backed by this retained run, not by the
hermetic test doubles:

| Field | Evidence |
|---|---|
| Date | 2026-08-18 |
| LaToile commit | `303503fa84960f349f8e7c7036631c6667d86cf6` |
| Provider | Codex through `codex-acp` |
| Result | `success`, Reviewer verdict `approve`, preview `ready` |
| Event cursor | `24` |
| Local SHA | `7438ccd02adc35468aa29f99b6e034469f8af163` |
| Remote SHA | `7438ccd02adc35468aa29f99b6e034469f8af163` |
| Pull Request | [disposable canary PR #1](https://github.com/salim4n/latoile-v1-canary-20260818-130417-98dbb1/pull/1) |

The repository remains private and disposable. The local evidence artifact is
ignored by Git because it also carries cleanup state; the table above is the
bounded, reviewable release proof.

## Epic merge gate

The brief-to-visual-review epic may close only when all three commands pass on
the same clean commit:

```sh
./scripts/guardian.sh
./scripts/release-smoke.sh
python3 scripts/v1_canary.py run --provider codex --approve-permissions
```

The first two are hermetic and mandatory in CI/release sign-off. The third is
stateful, paid and operator-triggered. Its evidence must show an empty initial
tree, greenfield Architect provenance, `blocking → passed`, distinct evidence
and executor ids, one unchanged baseline digest, Reviewer-before-human event
ordering, equal local/remote SHA and the PR URL. Archive the disposable remote
with the printed cleanup command after recording the bounded evidence below.

## Greenfield visual-contract evidence

The full greenfield claim is backed by this retained, bounded schema-v2 run
from the exact implementation commit:

| Field | Evidence |
|---|---|
| Date | 2026-08-18 UTC |
| LaToile commit | `fca8a6e625b3cbf282e52dcd78a34cdbf4d67556` |
| Provider | Codex through `codex-acp` |
| Empty initial repository | `true` |
| Architect discovery | 6 durable owner answers; `app-architect-brainstorm`; `greenfield` |
| Pinned skill SHA-256 | `19befb2a28f9f2d513c7dd056b63f83a545349aceed4452cccd25d35ccb60727` |
| Architecture package | 16 files; package `c8c1bb44a7474eb8b9d8368b1c66f6ab15d833c645d683009cf4d0d5790fc101` |
| Immutable spec | commit `21d9810ebc9303de8f113d3d43b9c05e0f3576d0`; tree `68d579ba3eb92c2f49d49f4f85dbf9a6d9251855` |
| Browser/baseline | `Chrome/151.0.7922.138`; baseline `890222bb69bd9a32c75e8ee31a8a1373c00ffd21a2c1235e6b6ce76e30d35ffa` |
| Initial comparison | `blocking`; 18,563 changed pixels; 56,395 ppm; geometry delta 16,000; AX changes 0 |
| Initial review | `changes_requested`; gate `visual_evidence_blocking`; 1 blocking finding |
| Corrected comparison | `passed`; 0 changed pixels; 0 ppm; geometry delta 0; AX changes 0 |
| Corrected review | `approve`; trusted/approvable; 0 findings |
| Baseline reuse | `true`; corrected render PNG digest equals the immutable baseline digest |
| Reviewer ordering | reviewer finished at events 44/55, before approval requests 45/56 |
| Owner control | initial review rejected; corrected trusted review explicitly granted |
| Additional review cycles | 0 needed; the bounded two-cycle repair path remained available |
| Delivery | preview `ready`; Pull Request `open` |
| Local/remote SHA | `5b54ab7e90c6fa0eda23eeab5ec476f868a42a4f` = `5b54ab7e90c6fa0eda23eeab5ec476f868a42a4f` |
| Pull Request | [greenfield canary PR #1](https://github.com/salim4n/latoile-v1-canary-20260818-232044-3f862b/pull/1) |
| Cleanup | disposable private repository archived; PR and local bounded evidence retained |

The initial and corrected comparisons have distinct run/evidence ids but the
same comparison id and baseline digest. The owner could not approve the first
review, and the final approval was requested only after the corrected Reviewer
run finished. This proves the automated brief-to-architecture-to-mockups-to-
validation path and the real capture/pixel-diff correction path together; it
does not assert deferred V2 features or production deployment.
