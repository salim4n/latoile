//! Where the one key that isn't in the database comes from.
//!
//! Two places, in order:
//!
//! 1. `LATOILE_MASTER_KEY` — base64. This is how a container or anything with
//!    a secret manager in front of it supplies one. Nothing is written to
//!    disk.
//! 2. `<config home>/master.key`, mode `0600`, created on first run — the
//!    same arrangement as an ssh private key: a file only this account can
//!    read, sitting outside the thing it protects.
//!
//! A key file with looser permissions is refused, not tightened: someone
//! changed it, and that is worth a human's attention rather than a silent
//! repair.
//!
//! **What losing it costs.** Every secret. There is no recovery and that is
//! the point — a backup of the database is not a backup of the credentials.

use crate::crypto::RootKey;
use crate::VaultError;
use std::path::{Path, PathBuf};

/// The environment variable that overrides everything.
pub const ENV: &str = "LATOILE_MASTER_KEY";

const FILE: &str = "master.key";

/// Which of the two answered, so start-up can say so out loud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Environment,
    File(PathBuf),
    /// Created just now. Worth a different line in the log: it means anything
    /// sealed under a previous key is no longer readable.
    NewFile(PathBuf),
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Environment => write!(f, "{ENV}"),
            Self::File(p) | Self::NewFile(p) => write!(f, "{}", p.display()),
        }
    }
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> VaultError + '_ {
    move |source| VaultError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Find the root key, or make one. Synchronous: it runs once at start-up.
pub fn load(home: &Path) -> Result<(RootKey, Source), VaultError> {
    load_with(home, std::env::var(ENV).ok())
}

/// The env value is a parameter so tests never touch the process environment.
fn load_with(home: &Path, env: Option<String>) -> Result<(RootKey, Source), VaultError> {
    if let Some(text) = env {
        if !text.trim().is_empty() {
            let key = RootKey::decode(&text).map_err(|_| {
                VaultError::KeyUnavailable(format!(
                    "{ENV} is set but is not a LaToile root key. Unset it to fall back to \
                     the key file, or set it to the key this deployment sealed its secrets with"
                ))
            })?;
            return Ok((key, Source::Environment));
        }
    }

    let path = home.join(FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let key = RootKey::decode(&text).map_err(|_| {
                VaultError::KeyUnavailable(format!(
                    "{} is not a root key. It has not been touched — every secret in the \
                     database is sealed with whatever used to be in it, so restore that file \
                     rather than deleting it",
                    path.display()
                ))
            })?;
            refuse_if_world_readable(&path)?;
            Ok((key, Source::File(path)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = RootKey::generate();
            write_new(&path, &key)?;
            Ok((key, Source::NewFile(path)))
        }
        Err(e) => Err(io(&path)(e)),
    }
}

/// A key someone else on the machine can read is not a key. Refuse to start.
#[cfg(unix)]
fn refuse_if_world_readable(path: &Path) -> Result<(), VaultError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).map_err(io(path))?.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(VaultError::InsecureKeyFile(path.to_path_buf(), mode));
    }
    Ok(())
}

/// Elsewhere the filesystem has no equivalent, so this is left to the
/// account's own profile directory.
#[cfg(not(unix))]
fn refuse_if_world_readable(_path: &Path) -> Result<(), VaultError> {
    Ok(())
}

/// Write the key where nothing can half-write it: into a neighbouring file
/// restricted to `0600` *before* anything is in it, then rename. A crash
/// partway through a direct write would leave a truncated key, and a
/// truncated root key is every secret gone.
fn write_new(path: &Path, key: &RootKey) -> Result<(), VaultError> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(io(parent))?;

    let temp = path.with_extension("key.new");
    create_restricted(&temp)?;
    std::fs::write(&temp, format!("{}\n", &*key.encode())).map_err(io(&temp))?;

    std::fs::rename(&temp, path).map_err(io(path))?;
    Ok(())
}

/// Create the file empty at `0600`, whatever the umask would have made it.
#[cfg(unix)]
fn create_restricted(path: &Path) -> Result<(), VaultError> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(io(path))?;
    Ok(())
}

#[cfg(not(unix))]
fn create_restricted(_path: &Path) -> Result<(), VaultError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sealed_round_trip(key: &RootKey) -> Vec<u8> {
        let sealed = key.seal("test", b"a token").unwrap();
        key.open("test", &sealed).unwrap().to_vec()
    }

    #[test]
    fn the_first_run_makes_a_key_and_the_second_finds_it() {
        let home = tempfile::tempdir().unwrap();

        let (first, source) = load_with(home.path(), None).unwrap();
        assert!(matches!(source, Source::NewFile(_)), "{source:?}");
        let sealed = first.seal("test", b"a token").unwrap();

        let (again, source) = load_with(home.path(), None).unwrap();
        assert!(matches!(source, Source::File(_)), "{source:?}");
        assert_eq!(
            &*again.open("test", &sealed).unwrap(),
            b"a token",
            "restarting must not orphan what was already sealed"
        );
    }

    #[test]
    fn the_environment_wins_and_writes_nothing() {
        let home = tempfile::tempdir().unwrap();
        let key = RootKey::generate();
        let encoded = key.encode().to_string();

        let (loaded, source) = load_with(home.path(), Some(encoded)).unwrap();
        assert_eq!(source, Source::Environment);
        assert!(
            !home.path().join(FILE).exists(),
            "a supplied key must not be copied to disk"
        );
        assert_eq!(sealed_round_trip(&loaded), b"a token");
    }

    /// The dangerous case: a damaged key file must stop the server, not be
    /// replaced with a fresh one that silently orphans every secret.
    #[test]
    fn a_damaged_key_file_is_never_overwritten() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(FILE);
        std::fs::write(&path, "half a k").unwrap();

        assert!(load_with(home.path(), None).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "half a k",
            "the file must be exactly as it was"
        );
    }

    #[test]
    fn a_nonsense_environment_value_stops_start_up() {
        let home = tempfile::tempdir().unwrap();
        assert!(
            load_with(home.path(), Some("obviously not a key".into())).is_err(),
            "better to refuse than to seal under junk"
        );
    }

    #[test]
    fn an_empty_environment_value_falls_through_to_the_file() {
        let home = tempfile::tempdir().unwrap();
        let (_, source) = load_with(home.path(), Some("  ".into())).unwrap();
        assert!(matches!(source, Source::NewFile(_)));
    }

    #[cfg(unix)]
    #[test]
    fn the_key_file_is_created_0600() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();

        load_with(home.path(), None).unwrap();
        let mode = std::fs::metadata(home.path().join(FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    /// A file loosened behind our back stops the server: someone touched it,
    /// and that deserves attention, not a silent repair.
    #[cfg(unix)]
    #[test]
    fn a_world_readable_key_file_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();

        load_with(home.path(), None).unwrap();
        let path = home.path().join(FILE);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let err = load_with(home.path(), None).unwrap_err();
        assert!(
            matches!(err, VaultError::InsecureKeyFile(_, 0o644)),
            "{err:?}"
        );
        // And it was not "fixed" for us.
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
    }
}
