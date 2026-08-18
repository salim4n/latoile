# Real-provider V1 canary

This canary is the opt-in proof for the complete LaToile V1 journey. It is
outside the default Cargo and web test suites because it consumes a real
Claude or Codex session and creates a private disposable GitHub repository.

## Prerequisites

- `git`, `gh`, `node`, Rust/Cargo and Python 3;
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

The canary creates a private `latoile-v1-canary-*` repository and exercises:

1. real Manager turn and draft spec;
2. explicit spec approval;
3. real frontend executor and committed evidence;
4. ready live preview;
5. real Reviewer result and explicit review approval;
6. verified branch push and Pull Request creation.

Success prints the PR URL, the exact local/remote SHA and the path to a bounded
`evidence.json`. The artifact contains only identifiers, statuses, the event
cursor, SHAs and the PR URL. Prompts, provider responses, credentials and
hidden reasoning are not retained.

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
