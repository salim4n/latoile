-- Real browser baselines are immutable evidence attached to one immutable
-- architecture scenario. Artifact bytes live under LATOILE_HOME/baselines;
-- SQLite keeps only bounded provenance, hashes and actionable failures.
CREATE TABLE visual_baseline (
    spec_version_id      TEXT NOT NULL REFERENCES spec_version (id) ON DELETE CASCADE,
    project_id           TEXT NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    comparison_id        TEXT NOT NULL,
    manifest_digest      TEXT NOT NULL,
    package_commit_sha   TEXT NOT NULL,
    status               TEXT NOT NULL CHECK (status IN ('ready', 'failed')),
    png_digest           TEXT,
    geometry_digest      TEXT,
    accessibility_digest TEXT,
    environment_digest   TEXT,
    browser_version      TEXT,
    font_fingerprint     TEXT,
    failure_code         TEXT,
    failure_message      TEXT,
    recovery_action      TEXT,
    created_at           TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at           TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (spec_version_id, comparison_id),
    CHECK (
        (status = 'ready'
          AND png_digest IS NOT NULL
          AND geometry_digest IS NOT NULL
          AND accessibility_digest IS NOT NULL
          AND environment_digest IS NOT NULL
          AND browser_version IS NOT NULL
          AND font_fingerprint IS NOT NULL
          AND failure_code IS NULL
          AND failure_message IS NULL
          AND recovery_action IS NULL)
        OR
        (status = 'failed'
          AND png_digest IS NULL
          AND geometry_digest IS NULL
          AND accessibility_digest IS NULL
          AND environment_digest IS NULL
          AND browser_version IS NULL
          AND font_fingerprint IS NULL
          AND failure_code IS NOT NULL
          AND failure_message IS NOT NULL
          AND recovery_action IS NOT NULL)
    )
);

CREATE INDEX visual_baseline_by_project
    ON visual_baseline (project_id, spec_version_id, comparison_id);

CREATE TRIGGER immutable_ready_visual_baseline
BEFORE UPDATE ON visual_baseline
WHEN OLD.status = 'ready'
BEGIN
    SELECT RAISE(ABORT, 'ready visual baselines are immutable');
END;
