-- Pin the exact Architect skill bundle and the reproducible design-package
-- Git evidence. Nullable evidence columns are populated atomically only
-- after the isolated package run has passed confinement and inventory checks.

ALTER TABLE architecture_session ADD COLUMN skill_name TEXT;
ALTER TABLE architecture_session ADD COLUMN skill_digest TEXT;
ALTER TABLE architecture_session ADD COLUMN operating_mode TEXT
    CHECK (operating_mode IN ('greenfield', 'reverse_engineering'));
ALTER TABLE architecture_session ADD COLUMN package_status TEXT NOT NULL DEFAULT 'not_started'
    CHECK (package_status IN ('not_started', 'generating', 'draft_ready'));
ALTER TABLE architecture_session ADD COLUMN package_design_dir TEXT;
ALTER TABLE architecture_session ADD COLUMN package_base_sha TEXT;
ALTER TABLE architecture_session ADD COLUMN package_head_sha TEXT;
ALTER TABLE architecture_session ADD COLUMN package_tree_sha TEXT;
ALTER TABLE architecture_session ADD COLUMN package_digest TEXT;
ALTER TABLE architecture_session ADD COLUMN package_changed_files TEXT;
ALTER TABLE architecture_session ADD COLUMN package_diff_stat TEXT;

ALTER TABLE spec_version ADD COLUMN architecture_session_id TEXT
    REFERENCES architecture_session (id);
ALTER TABLE spec_version ADD COLUMN skill_name TEXT;
ALTER TABLE spec_version ADD COLUMN skill_digest TEXT;
ALTER TABLE spec_version ADD COLUMN operating_mode TEXT
    CHECK (operating_mode IN ('greenfield', 'reverse_engineering'));
ALTER TABLE spec_version ADD COLUMN package_digest TEXT;
ALTER TABLE spec_version ADD COLUMN package_commit_sha TEXT;
ALTER TABLE spec_version ADD COLUMN package_tree_sha TEXT;
