//! The `latoile secret` subcommand — writing and listing vault secrets.
//! Values come from stdin only, never argv: shell history must not hold a
//! token. Prompts and confirmations go to stderr; names (never values) to
//! stdout.

use clap::Subcommand;
use std::path::{Path, PathBuf};

#[derive(clap::Args)]
pub struct SecretArgs {
    #[command(subcommand)]
    pub action: SecretCommand,

    /// Override the database file (default: <home>/latoile.db).
    #[arg(long, env = "LATOILE_DB")]
    pub db: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum SecretCommand {
    /// Store a secret. The value is read from stdin — never from argv, so
    /// it stays out of shell history.
    Set {
        /// The secret name, e.g. github_token.
        name: String,
    },

    /// List secret names — never values.
    List,
}

/// The vault on the same home/db resolution as `serve`.
async fn open_vault(
    home: &Path,
    db: Option<PathBuf>,
) -> Result<latoile_vault::Vault, Box<dyn std::error::Error>> {
    let db = db.unwrap_or_else(|| home.join("latoile.db"));
    let (root, _) = latoile_vault::load_root_key(home)?;
    Ok(latoile_vault::Vault::open(&db, root).await?)
}

/// `secret set`: prompt on stderr, read one line from stdin (piped input
/// works the same), store, confirm. The value is never echoed anywhere.
pub async fn secret_set(
    home: &Path,
    db: Option<PathBuf>,
    name: &str,
    input: &mut impl std::io::BufRead,
    err: &mut impl std::io::Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a secret needs a name".into());
    }
    write!(err, "value for {name}: ")?;
    err.flush()?;
    let mut value = String::new();
    input.read_line(&mut value)?;
    let value = value.trim_end_matches(['\r', '\n']);
    if value.is_empty() {
        return Err("empty value — nothing stored".into());
    }
    let vault = open_vault(home, db).await?;
    latoile_core::ports::SecretStore::put(&vault, name, value).await?;
    writeln!(err, "{name} stored")?;
    Ok(())
}

/// Interactive `secret set`: read without terminal echo. Piped input keeps
/// using [`secret_set`] so automation never needs a pseudo-terminal.
pub async fn secret_set_interactive(
    home: &Path,
    db: Option<PathBuf>,
    name: &str,
    err: &mut impl std::io::Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a secret needs a name".into());
    }
    let value = rpassword::prompt_password(format!("value for {name}: "))?;
    if value.is_empty() {
        return Err("empty value — nothing stored".into());
    }
    let vault = open_vault(home, db).await?;
    latoile_core::ports::SecretStore::put(&vault, name, &value).await?;
    writeln!(err, "{name} stored")?;
    Ok(())
}

/// `secret list`: names only, one per line.
pub async fn secret_list(
    home: &Path,
    db: Option<PathBuf>,
    out: &mut impl std::io::Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let vault = open_vault(home, db).await?;
    for name in vault.names().await? {
        writeln!(out, "{name}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Piped stdin, tempdir home: set stores, list names, and the value
    /// never appears in any output.
    #[tokio::test]
    async fn secret_set_and_list_round_trip() {
        let home = std::env::temp_dir().join(format!(
            "latoile-secret-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&home).unwrap();

        let mut input = std::io::Cursor::new("gho_supersecret\n");
        let mut err = Vec::new();
        secret_set(&home, None, "github_token", &mut input, &mut err)
            .await
            .unwrap();
        let err = String::from_utf8(err).unwrap();
        assert!(err.contains("value for github_token:"));
        assert!(err.contains("github_token stored"));
        assert!(
            !err.contains("gho_supersecret"),
            "the value must never echo"
        );

        let mut out = Vec::new();
        secret_list(&home, None, &mut out).await.unwrap();
        assert_eq!(String::from_utf8(out).unwrap().trim(), "github_token");

        // And it actually decrypts back.
        let vault = open_vault(&home, None).await.unwrap();
        let stored = latoile_core::ports::SecretStore::get(&vault, "github_token")
            .await
            .unwrap();
        assert_eq!(stored.as_deref(), Some("gho_supersecret"));

        std::fs::remove_dir_all(&home).ok();
    }

    #[tokio::test]
    async fn an_empty_value_stores_nothing() {
        let home =
            std::env::temp_dir().join(format!("latoile-secret-empty-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let mut input = std::io::Cursor::new("\n");
        let mut err = Vec::new();
        assert!(
            secret_set(&home, None, "github_token", &mut input, &mut err)
                .await
                .is_err()
        );
        std::fs::remove_dir_all(&home).ok();
    }
}
