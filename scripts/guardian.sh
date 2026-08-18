#!/usr/bin/env bash
# Executable form of docs/guardian-checklist.md. The real-provider canary is
# intentionally opt-in and is documented separately.
set -euo pipefail

LATOILE_REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$LATOILE_REPO_ROOT"

LATOILE_RG_BIN=${LATOILE_RG_BIN:-rg}
LATOILE_CARGO_BIN=${LATOILE_CARGO_BIN:-cargo}

if ! command -v "$LATOILE_RG_BIN" >/dev/null 2>&1; then
  echo "guardian: rg is required" >&2
  exit 1
fi

fail_on_matches() {
  local label=$1
  local matches=$2
  if [[ -n "$matches" ]]; then
    echo "guardian: $label" >&2
    echo "$matches" >&2
    exit 1
  fi
}

core_dependencies=$(
  "$LATOILE_RG_BIN" -n '^\s*(tokio|sqlx|axum|reqwest)\s*=' crates/core/Cargo.toml || true
)
fail_on_matches "core gained an I/O or runtime dependency" "$core_dependencies"

axum_outside_server=$(
  "$LATOILE_RG_BIN" -l 'axum::' crates --glob '*.rs' \
    | "$LATOILE_RG_BIN" -v '^crates/server/' || true
)
fail_on_matches "axum escaped crates/server" "$axum_outside_server"

sql_outside_store=$(
  "$LATOILE_RG_BIN" -l 'sqlx::query' crates --glob '*.rs' \
    | "$LATOILE_RG_BIN" -v '^crates/(app/src/store|vault)/' || true
)
fail_on_matches "SQL escaped persistence adapters" "$sql_outside_store"

spawn_outside_adapters=$(
  "$LATOILE_RG_BIN" -l '(^|[^A-Za-z])Command::new|tokio::process::Command|std::process::Command' \
    crates --glob '*.rs' \
    | "$LATOILE_RG_BIN" -v '^crates/(agents|preview|github)/' || true
)
fail_on_matches "process spawning escaped agents/preview/github" "$spawn_outside_adapters"

component_fetches=$(
  "$LATOILE_RG_BIN" -n 'fetch\(' web/src --glob '*.ts' --glob '*.tsx' \
    --glob '!*.test.ts' --glob '!*.test.tsx' \
    | "$LATOILE_RG_BIN" -v '^web/src/(api|events)\.ts:' || true
)
fail_on_matches "a component bypasses the transport modules" "$component_fetches"

merge_api=$(
  "$LATOILE_RG_BIN" -n 'merge_pull_request|pulls/.+/merge' crates/app crates/github --glob '*.rs' || true
)
fail_on_matches "GitHub delivery contains a merge operation" "$merge_api"

stale_promises=$(
  "$LATOILE_RG_BIN" -n \
    'skill \(to be written\)|orchestrator pass|Review screen skeleton|codebase doesn.t exist' \
    README.md docs ARCHITECTURE_CONTRACT.md crates --glob '*.md' --glob '*.rs' || true
)
fail_on_matches "stale implementation promise remains" "$stale_promises"

"$LATOILE_CARGO_BIN" clippy --workspace --all-targets -- -D warnings
"$LATOILE_CARGO_BIN" test --workspace

if command -v pnpm >/dev/null 2>&1; then
  (cd web && pnpm lint && pnpm test && pnpm build)
else
  (cd web && npm run lint && npm test && npm run build)
fi

python3 -m unittest discover -s scripts/tests -p 'test_*.py'
python3 -m py_compile scripts/v1_canary.py
bash -n scripts/release-smoke.sh

echo "guardian: all hermetic boundaries and tests passed"
