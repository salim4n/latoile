-- Reviewer V2 must be bound to the exact executor run whose immutable visual
-- evidence it may cite. Existing rows remain readable with NULL and are
-- deliberately treated as legacy/untrusted by the V2 approval gate.
ALTER TABLE run ADD COLUMN reviewed_run_id TEXT REFERENCES run (id);

CREATE INDEX run_by_review_subject ON run (reviewed_run_id);

CREATE TRIGGER review_subject_requires_reviewer_insert
BEFORE INSERT ON run
WHEN NEW.reviewed_run_id IS NOT NULL AND NEW.role_id != 'reviewer'
BEGIN
    SELECT RAISE(ABORT, 'only a Reviewer run can bind a review subject');
END;

CREATE TRIGGER review_subject_requires_reviewer_update
BEFORE UPDATE OF reviewed_run_id, role_id ON run
WHEN NEW.reviewed_run_id IS NOT NULL AND NEW.role_id != 'reviewer'
BEGIN
    SELECT RAISE(ABORT, 'only a Reviewer run can bind a review subject');
END;

CREATE TRIGGER immutable_review_subject
BEFORE UPDATE OF reviewed_run_id ON run
WHEN OLD.reviewed_run_id IS NOT NULL
 AND NEW.reviewed_run_id IS NOT OLD.reviewed_run_id
BEGIN
    SELECT RAISE(ABORT, 'a Reviewer run has one immutable review subject');
END;
