-- Persistent Socratic Architect discovery. The ACP process is still
-- supervised in memory, but the workflow state and every owner answer are
-- durable and auditable. A restart may fail the live process without losing
-- what was decided.

CREATE TABLE architecture_session (
    id               TEXT PRIMARY KEY,
    project_id       TEXT NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    status           TEXT NOT NULL
                     CHECK (status IN ('discovering', 'awaiting_answer',
                                       'ready_to_draft', 'failed', 'cancelled')),
    phase            TEXT NOT NULL
                     CHECK (phase IN ('domain_discovery', 'requirements',
                                      'ux_discovery', 'ready_to_draft')),
    acp_session_id   TEXT,
    failure_reason   TEXT,
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((status = 'failed' AND failure_reason IS NOT NULL)
        OR (status != 'failed' AND failure_reason IS NULL)),
    CHECK ((status = 'ready_to_draft' AND phase = 'ready_to_draft')
        OR status != 'ready_to_draft')
);

CREATE UNIQUE INDEX one_active_architecture_session_per_project
    ON architecture_session (project_id)
    WHERE status IN ('discovering', 'awaiting_answer', 'ready_to_draft');

CREATE TABLE architecture_question (
    id          TEXT PRIMARY KEY,
    session_id  TEXT NOT NULL REFERENCES architecture_session (id) ON DELETE CASCADE,
    sequence    INTEGER NOT NULL,
    prompt      TEXT NOT NULL,
    status      TEXT NOT NULL CHECK (status IN ('open', 'answered')),
    answer      TEXT,
    asked_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    answered_at TEXT,
    UNIQUE (session_id, sequence),
    CHECK ((status = 'open' AND answer IS NULL AND answered_at IS NULL)
        OR (status = 'answered' AND answer IS NOT NULL AND answered_at IS NOT NULL))
);

CREATE UNIQUE INDEX one_open_architecture_question_per_session
    ON architecture_question (session_id) WHERE status = 'open';
