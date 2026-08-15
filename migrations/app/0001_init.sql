-- LaToile application schema (architecture-spec.md §4).
--
-- Statuses are TEXT with CHECK constraints mirroring the core enums; the
-- partial-unique invariants (one active run per task, one active preview per
-- project, one approved spec per project) are enforced here as indexes AND in
-- the domain state machines (contract §4). Timestamps are TEXT ISO-8601 set
-- by SQLite — core entities expose no audit fields, so they stay DB-side.

CREATE TABLE project (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    slug           TEXT NOT NULL UNIQUE,
    github_repo    TEXT NOT NULL,
    default_branch TEXT NOT NULL,
    work_branch    TEXT NOT NULL,
    local_path     TEXT NOT NULL,
    status         TEXT NOT NULL
                   CHECK (status IN ('draft', 'specced', 'building', 'live')),
    dev_command    TEXT NOT NULL,
    deleted        INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE spec_version (
    id               TEXT PRIMARY KEY,
    project_id       TEXT NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    version          INTEGER NOT NULL,
    status           TEXT NOT NULL
                     CHECK (status IN ('draft', 'approved', 'superseded')),
    design_dir       TEXT NOT NULL,
    architect_run_id TEXT,
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (project_id, version)
);

-- One approved spec per project (invariant §3.2).
CREATE UNIQUE INDEX one_approved_spec_per_project
    ON spec_version (project_id) WHERE status = 'approved';

CREATE TABLE role (
    id                 TEXT PRIMARY KEY,
    label              TEXT NOT NULL,
    skill_path         TEXT,
    cli                TEXT NOT NULL CHECK (cli IN ('claude', 'codex')),
    system_prompt_path TEXT
);

-- The fixed V1 team (architecture-spec.md §3.3). Skills to be written.
INSERT INTO role (id, label, cli) VALUES
    ('manager',   'Manager',   'claude'),
    ('architect', 'Architect', 'claude'),
    ('backend',   'Backend',   'claude'),
    ('frontend',  'Frontend',  'claude'),
    ('reviewer',  'Reviewer',  'claude');

CREATE TABLE task (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    spec_version_id TEXT REFERENCES spec_version (id) ON DELETE SET NULL,
    role_id         TEXT NOT NULL REFERENCES role (id),
    title           TEXT NOT NULL,
    description     TEXT NOT NULL,
    status          TEXT NOT NULL
                    CHECK (status IN ('ready', 'in_progress', 'review',
                                      'changes_requested', 'done')),
    position        INTEGER NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE run (
    id              TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES task (id) ON DELETE CASCADE,
    role_id         TEXT NOT NULL REFERENCES role (id),
    triggered_by    TEXT NOT NULL CHECK (triggered_by IN ('user', 'manager')),
    acp_session_id  TEXT,
    status          TEXT NOT NULL
                    CHECK (status IN ('starting', 'running', 'blocked',
                                      'finished', 'error', 'cancelled')),
    input_tokens    INTEGER,
    output_tokens   INTEGER,
    summary         TEXT,
    started_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ended_at        TEXT
);

-- One active run per task (invariant §3.2.1); actives mirror
-- RunStatus::is_active in core.
CREATE UNIQUE INDEX one_active_run_per_task
    ON run (task_id) WHERE status IN ('starting', 'running', 'blocked');

CREATE TABLE approval (
    id         TEXT PRIMARY KEY,
    run_id     TEXT NOT NULL REFERENCES run (id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('spec', 'review', 'permission')),
    status     TEXT NOT NULL CHECK (status IN ('pending', 'granted', 'rejected')),
    payload    TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    decided_at TEXT
);

CREATE TABLE conversation (
    id         TEXT PRIMARY KEY,
    project_id TEXT NOT NULL UNIQUE REFERENCES project (id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE message (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversation (id) ON DELETE CASCADE,
    author          TEXT NOT NULL CHECK (author IN ('user', 'manager')),
    content         TEXT NOT NULL,
    actions         TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX message_by_conversation ON message (conversation_id, created_at);

CREATE TABLE preview (
    id         TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    port       INTEGER NOT NULL,
    status     TEXT NOT NULL
               CHECK (status IN ('starting', 'ready', 'stale', 'error', 'stopped')),
    branch     TEXT NOT NULL,
    pid        INTEGER,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- One active preview per project; actives mirror PreviewStatus::is_active.
CREATE UNIQUE INDEX one_active_preview_per_project
    ON preview (project_id) WHERE status IN ('starting', 'ready', 'stale');

-- Append-only journal. seq is the only SSE cursor (contract §4).
CREATE TABLE event (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id  TEXT NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    kind        TEXT NOT NULL
                CHECK (kind IN ('spec_version_created', 'spec_approved',
                                'task_ready', 'run_started', 'run_blocked',
                                'run_finished', 'approval_requested',
                                'approval_granted', 'approval_rejected',
                                'preview_ready', 'preview_stale',
                                'preview_error', 'message_posted')),
    payload     TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX event_by_project ON event (project_id, seq);

-- Ciphertext only. Written by the vault crate; no plaintext secret ever
-- touches the database (contract §5).
CREATE TABLE secret (
    name        TEXT PRIMARY KEY,
    ciphertext  TEXT NOT NULL,
    wrapped_key TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    rotated_at  TEXT
);
