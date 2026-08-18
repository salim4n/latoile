-- Sanitized executor evidence for review and delivery. The raw diff remains
-- in Git; SQLite stores bounded metadata only.

ALTER TABLE run ADD COLUMN base_sha TEXT;
ALTER TABLE run ADD COLUMN head_sha TEXT;
ALTER TABLE run ADD COLUMN artifacts TEXT;
