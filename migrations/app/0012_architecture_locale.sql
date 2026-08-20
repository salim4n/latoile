-- Pin the owner-selected UI locale on the architecture session so package
-- prose and every visual scenario cannot silently switch languages.

ALTER TABLE architecture_session ADD COLUMN requested_locale TEXT NOT NULL DEFAULT 'en-US'
    CHECK (requested_locale IN ('en-US', 'fr-FR'));
