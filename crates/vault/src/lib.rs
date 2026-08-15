//! The vault. Every secret LaToile holds is envelope-encrypted
//! (XChaCha20-Poly1305, per-secret key wrapped by a root key, ciphertext bound
//! to its name via AAD). The root key comes from the environment or a
//! 0600-permission key file — never from the database, so a database backup
//! alone opens nothing.
//!
//! Secret values are never logged. Implements ports defined in `latoile-core`.
