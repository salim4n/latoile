-- Server-produced evidence comparing one executor run against one immutable
-- approved baseline. Binary artifacts remain in the capture adapter store.
CREATE TABLE visual_comparison (
    id                        TEXT PRIMARY KEY,
    spec_version_id           TEXT NOT NULL REFERENCES spec_version (id) ON DELETE CASCADE,
    project_id                TEXT NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    run_id                    TEXT NOT NULL REFERENCES run (id) ON DELETE CASCADE,
    comparison_id             TEXT NOT NULL,
    manifest_digest           TEXT NOT NULL,
    package_commit_sha        TEXT NOT NULL,
    baseline_png_digest       TEXT NOT NULL,
    status                    TEXT NOT NULL CHECK (status IN ('invalid', 'blocking', 'reservation', 'passed')),
    changed_pixels            INTEGER NOT NULL,
    total_pixels              INTEGER NOT NULL,
    pixel_ratio_micros        INTEGER NOT NULL CHECK (pixel_ratio_micros BETWEEN 0 AND 1000000),
    max_geometry_delta_milli  INTEGER NOT NULL,
    accessibility_changes     INTEGER NOT NULL,
    render_png_digest         TEXT,
    pixel_diff_digest         TEXT,
    heatmap_png_digest        TEXT,
    geometry_diff_digest      TEXT,
    accessibility_diff_digest TEXT,
    environment_digest        TEXT,
    browser_version           TEXT,
    font_fingerprint          TEXT,
    failure_code              TEXT,
    failure_message           TEXT,
    recovery_action           TEXT,
    created_at                TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at                TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (run_id, comparison_id),
    CHECK (
        (status != 'invalid'
          AND total_pixels > 0
          AND changed_pixels >= 0
          AND changed_pixels <= total_pixels
          AND render_png_digest IS NOT NULL
          AND pixel_diff_digest IS NOT NULL
          AND heatmap_png_digest IS NOT NULL
          AND geometry_diff_digest IS NOT NULL
          AND accessibility_diff_digest IS NOT NULL
          AND environment_digest IS NOT NULL
          AND browser_version IS NOT NULL
          AND font_fingerprint IS NOT NULL
          AND failure_code IS NULL
          AND failure_message IS NULL
          AND recovery_action IS NULL)
        OR
        (status = 'invalid'
          AND changed_pixels = 0
          AND total_pixels = 0
          AND pixel_ratio_micros = 0
          AND max_geometry_delta_milli = 0
          AND accessibility_changes = 0
          AND render_png_digest IS NULL
          AND pixel_diff_digest IS NULL
          AND heatmap_png_digest IS NULL
          AND geometry_diff_digest IS NULL
          AND accessibility_diff_digest IS NULL
          AND environment_digest IS NULL
          AND browser_version IS NULL
          AND font_fingerprint IS NULL
          AND failure_code IS NOT NULL
          AND failure_message IS NOT NULL
          AND recovery_action IS NOT NULL)
    )
);

CREATE INDEX visual_comparison_by_run
    ON visual_comparison (run_id, comparison_id);

CREATE INDEX visual_comparison_by_project
    ON visual_comparison (project_id, spec_version_id, run_id);

CREATE TRIGGER immutable_complete_visual_comparison
BEFORE UPDATE ON visual_comparison
WHEN OLD.status != 'invalid'
BEGIN
    SELECT RAISE(ABORT, 'complete visual comparisons are immutable');
END;
