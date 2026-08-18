# Operations — release, recovery, backup and service supervision

This guide is for the V1 single-user, single-binary deployment. LaToile owns
its SQLite database, encrypted secret rows and supervised child processes. A
reverse proxy owns TLS; GitHub remains the durable remote for project code.

## Runtime requirements

Install these commands for the account that runs the service:

| Command | Purpose |
|---|---|
| `git` and `sh` | checkout, evidence, delivery and project dev commands |
| `claude` + `claude-agent-acp` | Claude provider, when selected in Settings |
| `codex` + `codex-acp` | Codex provider, when selected in Settings |
| Google Chrome or Chromium | isolated deterministic P0 baseline capture |
| project toolchains | only what each checkout's `dev_command` and tasks need |

The maintained ACP adapter packages are
`@zed-industries/claude-agent-acp` and
`@agentclientprotocol/codex-acp`. At least one complete provider pair must be
on the service account's `PATH`. Connect it through **Settings** after the
server starts. Provider credentials remain owned by the provider CLI.

LaToile discovers `google-chrome-stable`, `google-chrome`, `chromium` or
`chromium-browser` in standard Linux locations (and Google Chrome on macOS).
For a pinned operator-managed build, set
`LATOILE_CAPTURE_BROWSER=/absolute/path/to/chromium`. Every successful baseline
records the product version, executable SHA-256 and rendered font fingerprint;
changing that environment makes a repeat mismatch explicit instead of silently
replacing approved evidence.

Live comparison uses the same pinned browser/font environment. The capture
browser starts with a cleared process environment, allows HTTP requests only
to the exact supervised `127.0.0.1:<preview-port>` origin and blocks HTTPS,
file, FTP and every WebSocket. A browser/font mismatch records an invalid,
actionable comparison rather than calculating a misleading similarity score.

The subsequent Reviewer run is permanently bound to that executor run. Its V2
response must echo the exact evidence ids and hashes shown in the prompt; the
server reloads project, approved spec and comparison rows before writing the
approval. A missing, stale, cross-project, hash-mismatched, invalid or blocking
set yields `changes_requested` and the grant endpoint refuses it. Historic V1
approval records remain visible after migration but cannot be granted as
trusted evidence; relaunch the Reviewer to obtain V2.

Store the GitHub token through stdin, never as an argument:

```sh
/usr/local/bin/latoile --home /var/lib/latoile secret set github_token
```

## Build and verify the release artifact

The web bundle must exist before the Rust release build because release mode
embeds `web/dist` into the binary:

```sh
cd web
pnpm install --frozen-lockfile
cd ..
./scripts/release-smoke.sh
sudo install -m 0755 target/release/latoile /usr/local/bin/latoile
```

The smoke script starts only `target/release/latoile` on disposable state. It
checks embedded migrations, `/api/health`, the embedded React entry point,
skill seeding, pre-listener recovery of a blocked run/permission/preview,
backup/restore, preservation of workspace sentinels and a second start from
restored state. CI runs the same script on Ubuntu.

## Supervised service

Use a dedicated account and keep the HTTP listener on loopback behind a
TLS-terminating reverse proxy. Example `/etc/systemd/system/latoile.service`:

```ini
[Unit]
Description=LaToile AI project workbench
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=latoile
Group=latoile
Environment=HOME=/var/lib/latoile
EnvironmentFile=-/etc/latoile/latoile.env
ExecStart=/usr/local/bin/latoile --home /var/lib/latoile serve --bind 127.0.0.1 --port 7700
Restart=on-failure
RestartSec=3
TimeoutStopSec=40
KillMode=control-group
StateDirectory=latoile
StateDirectoryMode=0700
WorkingDirectory=/var/lib/latoile
UMask=0077
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/latoile

[Install]
WantedBy=multi-user.target
```

`KillMode=control-group` is intentional: systemd reaps the ACP adapters and
dev-server process trees if the parent is killed. LaToile itself kills its
process groups on graceful `SIGTERM`. On restart it never signals a PID read
from SQLite, because that number may already belong to another process.

After installing or changing the unit:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now latoile
curl --fail --silent http://127.0.0.1:7700/api/health
sudo journalctl -u latoile -n 100 --no-pager
```

A healthy response is `{"status":"ok","database":"ok"}`. The endpoint is
open by design so the local supervisor and reverse proxy can probe it; every
product route still requires the bearer token.

## Restart recovery contract

Recovery completes before the HTTP router is returned and the listener is
opened:

| Persisted state | Restart result | Owner's next action |
|---|---|---|
| executor run `starting/running/blocked` | run `error`; in-progress task returns to `ready` | re-dispatch the task |
| pending permission | rejected with `permission session unavailable: lost to a server restart` | inspect, then re-dispatch if still wanted |
| Reviewer run | run `error`; bounded `changes_requested` fallback appears | retry review/correct the task |
| preview `starting/ready/stale` | preview `error`; stored PID cleared | click/start preview again |

The periodic supervisor also marks a `ready` preview `error` when its owned
process exits after startup. Both paths journal an actionable event; neither
deletes a project checkout or a secret.

## Backup

Create a private backup directory. SQLite uses `VACUUM INTO`, so the snapshot
is consistent even while the service writes in WAL mode. Every encrypted row
is opened with the current root key before the backup is accepted.

```sh
sudo -u latoile /usr/local/bin/latoile --home /var/lib/latoile \
  backup create --output /var/backups/latoile/2026-08-18T1600Z
```

The output contains:

- `latoile.db` — a standalone SQLite snapshot;
- `master.key` — the matching root key, mode `0600` inside a mode `0700`
  directory;
- `manifest.txt` — format and creation metadata, never a secret value.

Treat the whole directory as secret material. If `LATOILE_MASTER_KEY` supplies
the key, `backup create` intentionally writes that key into this restricted
backup so the database is recoverable. Encrypt and move the backup off-host.

Project checkouts are deliberately excluded: restore never deletes them, but
the backup also cannot recover an unpushed commit. Deliver work branches to
GitHub and, if local-only repositories matter, snapshot
`/var/lib/latoile/workspace` separately.

## Restore drill

Restore refuses to overwrite either `latoile.db` or `master.key`. Keep the
service stopped, move the old pair to a rollback directory, and leave the
workspace exactly where it is:

```sh
sudo systemctl stop latoile
sudo -u latoile mkdir -m 0700 /var/lib/latoile/rollback-before-restore
sudo -u latoile mv /var/lib/latoile/latoile.db \
  /var/lib/latoile/rollback-before-restore/
sudo -u latoile mv /var/lib/latoile/master.key \
  /var/lib/latoile/rollback-before-restore/
sudo -u latoile /usr/local/bin/latoile --home /var/lib/latoile \
  backup restore --input /var/backups/latoile/2026-08-18T1600Z
sudo systemctl start latoile
curl --fail --silent http://127.0.0.1:7700/api/health
```

Restore first validates a disposable database copy, applies embedded
migrations, checks SQLite integrity and verifies every encrypted row with the
bundled key. A `.restore-in-progress` marker prevents the service from
starting between installation of the key and database. Normal errors clean
temporary files; if the host crashes mid-restore, keep the service stopped
and inspect that marker plus the rollback directory before continuing.

When the deployment uses `LATOILE_MASTER_KEY`, restore the matching key in the
external secret manager. The bundled key-file restore refuses to run while
that environment variable is set, because the file would otherwise be
silently ignored.
