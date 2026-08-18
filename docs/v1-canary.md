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

1. a real persistent Socratic Architect discovery through the Manager surface;
2. the complete pinned `app-architect-brainstorm` package and manifest;
3. immutable spec validation, Git commit/tree proof and real Chromium baseline;
4. a frontend executor that serves the approved mockup with a deliberate 16 px regression;
5. server evidence that classifies that regression as `blocking`;
6. a finished Reviewer V2 run before the first human approval request;
7. an explicit owner rejection and exactly one linked corrective run;
8. distinct corrective evidence classified `passed` against the same baseline;
9. unchanged approved design bytes, a corrected live preview and a second Reviewer-before-human cycle;
10. explicit approval, exact pushed SHA and a live Pull Request.

Success prints the PR URL, the exact local/remote SHA and the path to a bounded
`evidence.json`. The artifact contains only identifiers, counts, statuses,
event sequence numbers, content/Git hashes and the PR URL. Prompts, question
text, owner answers, provider responses, credentials and hidden reasoning are
not retained.

On failure the same artifact records `first_broken_seam` and a concise safe
diagnostic. The remote repository is retained so the failure remains
inspectable.

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

No completion claim is made here until a successful schema-v2 evidence artifact
from the exact implementation commit has been archived and summarized.
