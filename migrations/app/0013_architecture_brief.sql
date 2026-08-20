-- Keep the original owner brief bound to its architecture session. Package
-- generation must receive the scope authority as well as later Q/A decisions.

ALTER TABLE architecture_session ADD COLUMN brief TEXT NOT NULL DEFAULT '';
