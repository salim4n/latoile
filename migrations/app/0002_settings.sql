-- Key/value settings. First citizen: role→provider routing (cost control —
-- which agent subscription works which role).

CREATE TABLE setting (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- The fixed team defaults to Claude everywhere.
INSERT INTO setting (key, value) VALUES
    ('routing.manager',   'claude'),
    ('routing.architect', 'claude'),
    ('routing.backend',   'claude'),
    ('routing.frontend',  'claude'),
    ('routing.reviewer',  'claude');
