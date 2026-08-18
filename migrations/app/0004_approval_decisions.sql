-- Owner decision audit and the idempotent corrective-run link.

ALTER TABLE approval ADD COLUMN decision_comment TEXT;
ALTER TABLE approval ADD COLUMN corrective_run_id TEXT REFERENCES run (id);

CREATE UNIQUE INDEX one_approval_per_corrective_run
    ON approval (corrective_run_id) WHERE corrective_run_id IS NOT NULL;
