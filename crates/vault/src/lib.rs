//! The vault. Every secret LaToile holds is envelope-encrypted
//! (XChaCha20-Poly1305, per-secret key wrapped by a root key, ciphertext bound
//! to its name via AAD). The root key comes from the environment or a
//! 0600-permission key file — never from the database, so a database backup
//! alone opens nothing.
//!
//! Secret values are never logged. Implements ports defined in `latoile-core`.
//! SQL lives here and in `app/src/store` only (architecture contract §1).

mod crypto;
mod root;
mod store;

pub use crypto::RootKey;
pub use root::{Source, ENV as MASTER_KEY_ENV};
pub use store::Vault;

/// The key-file resolver, so start-up can ask for the key without naming the
/// module.
pub use root::load as load_root_key;

use latoile_core::ports::PortError;
use std::path::PathBuf;

/// What the vault reports. A secret's *value* is never part of an error —
/// only its name, which is not a secret.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("database error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// The root key cannot be read, parsed, or used. Nothing is sealed or
    /// opened without it.
    #[error("the root key is unavailable: {0}")]
    KeyUnavailable(String),
    /// The key file's permissions were loosened. Refused, not repaired —
    /// someone touched it and that deserves attention.
    #[error("the key file {0} is readable beyond this account (mode {1:o}); refusing to use it")]
    InsecureKeyFile(PathBuf, u32),
    /// Wrong key, tampered byte, or renamed row — deliberately one error for
    /// all of them; which one is a detail an attacker would like.
    #[error("this secret did not verify — wrong key or altered row")]
    DecryptionFailed,
    /// Absence is normally `Ok(None)`; this is for callers that require one.
    #[error("secret not found: {0}")]
    NotFound(String),
    /// Reading or writing the key file itself failed.
    #[error("i/o on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl From<VaultError> for PortError {
    fn from(e: VaultError) -> Self {
        PortError(e.to_string())
    }
}
