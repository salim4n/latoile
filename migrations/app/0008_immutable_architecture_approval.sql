-- The manifest is an independently addressable visual contract. Older draft
-- rows intentionally remain NULL and therefore cannot pass the new approval
-- verifier; the owner must generate a new architecture version.
ALTER TABLE architecture_session ADD COLUMN package_manifest_digest TEXT;
ALTER TABLE spec_version ADD COLUMN manifest_digest TEXT;
