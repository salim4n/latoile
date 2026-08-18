#!/usr/bin/env bash
# Build the production artifact, start only that artifact on disposable state,
# exercise health + embedded UI, then prove backup/restore and a second start.
set -euo pipefail

LATOILE_REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$LATOILE_REPO_ROOT"

LATOILE_CARGO_BIN=${LATOILE_CARGO_BIN:-cargo}
LATOILE_RELEASE_BINARY=${LATOILE_RELEASE_BINARY:-$LATOILE_REPO_ROOT/target/release/latoile}

if [[ ${LATOILE_SKIP_BUILD:-0} != 1 ]]; then
  if command -v pnpm >/dev/null 2>&1; then
    (cd web && pnpm build)
  else
    (cd web && npm run build)
  fi
  "$LATOILE_CARGO_BIN" build --release -p latoile-cli
fi

if [[ ! -x "$LATOILE_RELEASE_BINARY" ]]; then
  echo "release smoke: binary is missing or not executable: $LATOILE_RELEASE_BINARY" >&2
  exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "release smoke: curl is required" >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "release smoke: python3 is required for the disposable recovery probe" >&2
  exit 1
fi

LATOILE_SMOKE_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/latoile-release-smoke.XXXXXX")
LATOILE_SERVER_PID=

cleanup() {
  if [[ -n "$LATOILE_SERVER_PID" ]] && kill -0 "$LATOILE_SERVER_PID" 2>/dev/null; then
    kill -TERM "$LATOILE_SERVER_PID" 2>/dev/null || true
    wait "$LATOILE_SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$LATOILE_SMOKE_ROOT"
}
trap cleanup EXIT INT TERM

start_and_probe() {
  local home=$1
  local label=$2
  local log="$LATOILE_SMOKE_ROOT/$label.log"
  : >"$log"
  "$LATOILE_RELEASE_BINARY" \
    --home "$home" serve --bind 127.0.0.1 --port 0 --token release-smoke \
    >"$log" 2>&1 &
  LATOILE_SERVER_PID=$!

  local url=
  for _ in $(seq 1 200); do
    if ! kill -0 "$LATOILE_SERVER_PID" 2>/dev/null; then
      echo "release smoke: server exited during $label startup" >&2
      tail -80 "$log" >&2
      return 1
    fi
    url=$(sed -n 's/^  url: *//p' "$log" | tail -1)
    if [[ -n "$url" ]]; then
      break
    fi
    sleep 0.1
  done
  if [[ -z "$url" ]]; then
    echo "release smoke: server did not report its URL" >&2
    tail -80 "$log" >&2
    return 1
  fi

  local health
  health=$(curl --fail --silent --show-error "$url/api/health")
  [[ "$health" == *'"status":"ok"'* ]]
  [[ "$health" == *'"database":"ok"'* ]]
  curl --fail --silent --show-error "$url/" | grep -q 'id="root"'
  [[ -s "$home/latoile.db" ]]
  [[ -s "$home/master.key" ]]
  [[ -s "$home/skills/project-manager/SKILL.md" ]]

  kill -TERM "$LATOILE_SERVER_PID"
  wait "$LATOILE_SERVER_PID"
  LATOILE_SERVER_PID=
}

LATOILE_FIRST_HOME="$LATOILE_SMOKE_ROOT/first-home"
LATOILE_RESTORED_HOME="$LATOILE_SMOKE_ROOT/restored-home"
LATOILE_BACKUP="$LATOILE_SMOKE_ROOT/backup"

start_and_probe "$LATOILE_FIRST_HOME" first
printf 'release-smoke-secret\n' | "$LATOILE_RELEASE_BINARY" \
  --home "$LATOILE_FIRST_HOME" secret set smoke_token

LATOILE_RECOVERY_CHECKOUT="$LATOILE_FIRST_HOME/workspace/recovery-checkout"
mkdir -p "$LATOILE_RECOVERY_CHECKOUT/.git"
printf 'must survive restart recovery\n' >"$LATOILE_RECOVERY_CHECKOUT/sentinel"
python3 - "$LATOILE_FIRST_HOME/latoile.db" "$LATOILE_RECOVERY_CHECKOUT" <<'PY'
import sqlite3
import sys

database, checkout = sys.argv[1:]
with sqlite3.connect(database) as connection:
    connection.execute(
        "INSERT INTO project "
        "(id,name,slug,github_repo,default_branch,work_branch,local_path,status,dev_command) "
        "VALUES (?,?,?,?,?,?,?,?,?)",
        (
            "recovery-project",
            "Recovery Probe",
            "recovery-probe",
            "owner/recovery-probe",
            "main",
            "work/recovery-probe",
            checkout,
            "specced",
            "false",
        ),
    )
    connection.execute(
        "INSERT INTO spec_version (id,project_id,version,status,design_dir) "
        "VALUES ('recovery-spec','recovery-project',1,'approved','design/')"
    )
    connection.execute(
        "INSERT INTO task "
        "(id,project_id,spec_version_id,role_id,title,description,status,position) "
        "VALUES ('recovery-task','recovery-project','recovery-spec','frontend',"
        "'Recovery task','Disposable restart proof','in_progress',0)"
    )
    connection.execute(
        "INSERT INTO run "
        "(id,task_id,role_id,triggered_by,acp_session_id,status) "
        "VALUES ('recovery-run','recovery-task','frontend','manager','lost-acp','blocked')"
    )
    connection.execute(
        "INSERT INTO approval (id,run_id,kind,status,payload) "
        "VALUES ('recovery-permission','recovery-run','permission','pending',"
        "'{\"schema_version\":1,\"request_id\":\"lost\",\"summary\":\"Modify files\"}')"
    )
    connection.execute(
        "INSERT INTO preview (id,project_id,port,status,branch,pid) "
        "VALUES ('recovery-preview','recovery-project',4199,'ready','work/recovery-probe',424242)"
    )
PY

start_and_probe "$LATOILE_FIRST_HOME" recovered
python3 - "$LATOILE_FIRST_HOME/latoile.db" "$LATOILE_RECOVERY_CHECKOUT/sentinel" <<'PY'
import sqlite3
import sys

database, sentinel = sys.argv[1:]
with sqlite3.connect(database) as connection:
    run = connection.execute(
        "SELECT status FROM run WHERE id='recovery-run'"
    ).fetchone()
    task = connection.execute(
        "SELECT status FROM task WHERE id='recovery-task'"
    ).fetchone()
    approval = connection.execute(
        "SELECT status,decision_comment FROM approval WHERE id='recovery-permission'"
    ).fetchone()
    preview = connection.execute(
        "SELECT status,pid FROM preview WHERE id='recovery-preview'"
    ).fetchone()
assert run == ("error",), run
assert task == ("ready",), task
assert approval[0] == "rejected" and "server restart" in approval[1], approval
assert preview == ("error", None), preview
with open(sentinel, encoding="utf-8") as stream:
    assert stream.read().strip() == "must survive restart recovery"
PY

"$LATOILE_RELEASE_BINARY" --home "$LATOILE_FIRST_HOME" \
  backup create --output "$LATOILE_BACKUP"

mkdir -p "$LATOILE_RESTORED_HOME/workspace/retained-repository/.git"
printf 'preserve repositories\n' >"$LATOILE_RESTORED_HOME/workspace/retained-repository/sentinel"
"$LATOILE_RELEASE_BINARY" --home "$LATOILE_RESTORED_HOME" \
  backup restore --input "$LATOILE_BACKUP"
grep -q 'preserve repositories' \
  "$LATOILE_RESTORED_HOME/workspace/retained-repository/sentinel"
start_and_probe "$LATOILE_RESTORED_HOME" restored
"$LATOILE_RELEASE_BINARY" --home "$LATOILE_RESTORED_HOME" secret list \
  | grep -q '^smoke_token$'

echo "release smoke: production binary, embedded UI, migrations, health and backup restore passed"
