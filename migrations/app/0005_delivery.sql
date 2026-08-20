-- Verified push and Pull Request evidence for the owner-controlled delivery
-- action. A failed PR API call can leave an honest `pushed` record; retrying
-- upgrades the same project row once the open PR is found or created.

CREATE TABLE delivery (
    project_id       TEXT PRIMARY KEY REFERENCES project (id) ON DELETE CASCADE,
    work_branch      TEXT NOT NULL,
    local_sha        TEXT NOT NULL,
    remote_sha       TEXT NOT NULL,
    status           TEXT NOT NULL CHECK (status IN ('pushed', 'pull_request_open')),
    pull_request_url TEXT,
    delivered_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (local_sha = remote_sha),
    CHECK ((status = 'pushed' AND pull_request_url IS NULL)
        OR (status = 'pull_request_open' AND pull_request_url IS NOT NULL))
);
