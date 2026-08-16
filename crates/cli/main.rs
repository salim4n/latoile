//! The `latoile` binary. Composition root: loads configuration, runs
//! migrations, wires adapters to ports, and starts the server.
//!
//! One home directory holds everything: the SQLite database, the vault's
//! `master.key`, the agent workspace. `/api/health` is the only open route;
//! everything else needs the bearer token printed at startup (D9).

use clap::{Parser, Subcommand};
use std::path::Path;
use latoile_server::{ServerConfig, TOKEN_ENV};
use std::net::IpAddr;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "latoile",
    version,
    about = "Your AI-native project workbench, on your own server.",
    long_about = None,
)]
struct Cli {
    /// Where LaToile keeps its state: database, vault key, workspace.
    #[arg(long, env = "LATOILE_HOME", global = true)]
    home: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start the server. The default when no subcommand is given.
    Serve(ServeArgs),

    /// Print the token in effect, or how to set one — without starting
    /// anything.
    Token,
}

#[derive(clap::Args, Clone)]
struct ServeArgs {
    /// The SQLite database file.
    #[arg(long, env = "LATOILE_DB")]
    db: Option<PathBuf>,

    #[arg(long, env = "LATOILE_PORT", default_value_t = 7700)]
    port: u16,

    /// The interface to bind. Keep this loopback unless something in front
    /// terminates TLS — the token is the only auth.
    #[arg(long, env = "LATOILE_BIND", default_value = "127.0.0.1")]
    bind: IpAddr,

    /// Agent sessions run with this as their root directory.
    #[arg(long, env = "LATOILE_WORKSPACE")]
    workspace: Option<PathBuf>,

    /// Role skill preambles (`<dir>/<skill>/SKILL.md`).
    #[arg(long, env = "LATOILE_SKILLS_DIR")]
    skills_dir: Option<PathBuf>,

    /// The bearer token. Overrides `LATOILE_TOKEN`; when neither is set, one
    /// is generated and printed at startup.
    #[arg(long)]
    token: Option<String>,
}

impl Default for ServeArgs {
    fn default() -> Self {
        Self {
            db: None,
            port: 7700,
            bind: IpAddr::from([127, 0, 0, 1]),
            workspace: None,
            skills_dir: None,
            token: None,
        }
    }
}

/// The vendored role skills (repo `skills/`), embedded in release builds.
#[derive(rust_embed::Embed)]
#[folder = "../../skills"]
struct Skills;

/// Seed `<home>/skills` from the embedded copy. Existing files are never
/// overwritten — the user's edits win over upgrades.
fn ensure_skills(home: &Path) -> std::io::Result<()> {
    let root = home.join("skills");
    for path in Skills::iter() {
        let target = root.join(path.as_ref());
        if target.exists() {
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = Skills::get(&path).expect("iterated paths exist");
        std::fs::write(&target, &file.data)?;
    }
    Ok(())
}

/// `~/.local/share/latoile`, or `$XDG_DATA_HOME/latoile` when set.
fn default_home() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("latoile");
        }
    }
    let home = std::env::var("HOME").expect("HOME is always set on unix");
    PathBuf::from(home).join(".local/share/latoile")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let home = cli.home.unwrap_or_else(default_home);

    match cli.command {
        Some(Command::Token) => token(),
        Some(Command::Serve(args)) => serve(home, args).await,
        None => serve(home, ServeArgs::default()).await,
    }
}

/// What `latoile token` answers: the token the server would use, or how to
/// choose one. A generated token only exists while `serve` runs, so there is
/// nothing to print ahead of time — the honest answer is the guidance.
fn token() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::var(TOKEN_ENV) {
        Ok(value) if !value.trim().is_empty() => println!("{value}"),
        _ => println!(
            "No token is set. Either:\n  \
             - start `latoile serve` and copy the generated token it prints, or\n  \
             - choose one: export {}=<token> (then `latoile token` prints it back)",
            TOKEN_ENV
        ),
    }
    Ok(())
}

async fn serve(home: PathBuf, args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let db = args.db.unwrap_or_else(|| home.join("latoile.db"));
    let config = ServerConfig {
        token: args.token,
        workspace: args.workspace.unwrap_or_else(|| home.join("workspace")),
        skills_dir: args.skills_dir.unwrap_or_else(|| home.join("skills")),
        config_home: home.clone(),
        github_api_base: None,
    };

    // The database file's parent must exist before sqlx creates the file.
    std::fs::create_dir_all(&home)?;
    // Role skills: the vendored defaults, unless the user edited them.
    ensure_skills(&home)?;
    if let Some(parent) = db.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let (router, token, token_source, driver) = latoile_server::build(&config, &db).await?;

    let listener = tokio::net::TcpListener::bind((args.bind, args.port)).await?;
    let addr = listener.local_addr()?;

    eprintln!("latoile {}", env!("CARGO_PKG_VERSION"));
    eprintln!("  url:       http://{addr}");
    eprintln!("  database:  {}", db.display());
    match token_source {
        "generated" => eprintln!("  token:     {token}   (generated — paste it into the UI)"),
        source => eprintln!("  token:     {token}   (from {source})"),
    }

    latoile_server::serve(listener, router, shutdown()).await?;
    driver.abort();
    eprintln!("latoile: stopped");
    Ok(())
}

async fn shutdown() {
    if tokio::signal::ctrl_c().await.is_ok() {
        eprintln!("\nlatoile: shutting down…");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_invocation_is_serve_with_defaults() {
        let cli = Cli::try_parse_from(["latoile"]).unwrap();
        assert!(cli.command.is_none(), "no subcommand → serve");
        assert!(cli.home.is_none());
    }

    #[test]
    fn serve_takes_port_bind_and_home() {
        let cli = Cli::try_parse_from([
            "latoile",
            "--home",
            "/tmp/lt",
            "serve",
            "--port",
            "9999",
            "--bind",
            "0.0.0.0",
        ])
        .unwrap();
        assert_eq!(cli.home.as_deref(), Some(std::path::Path::new("/tmp/lt")));
        match cli.command {
            Some(Command::Serve(args)) => {
                assert_eq!(args.port, 9999);
                assert_eq!(args.bind, IpAddr::from([0, 0, 0, 0]));
                assert_eq!(args.db, None);
            }
            _ => panic!("expected serve"),
        }
    }

    #[test]
    fn the_defaults_are_loopback_and_7700() {
        let cli = Cli::try_parse_from(["latoile", "serve"]).unwrap();
        match cli.command {
            Some(Command::Serve(args)) => {
                assert_eq!(args.port, 7700);
                assert!(args.bind.is_loopback());
            }
            _ => panic!("expected serve"),
        }
    }

    #[test]
    fn token_is_a_subcommand() {
        let cli = Cli::try_parse_from(["latoile", "token"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Token)));
    }

    /// The whole stack on ephemeral state: migrations, vault key creation,
    /// the router, and an answer from `/api/health` without a token.
    #[tokio::test]
    async fn serve_smoke_test_health_without_token() {
        let home = std::env::temp_dir().join(format!(
            "latoile-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&home).unwrap();
        let db = home.join("latoile.db");
        let config = ServerConfig {
            token: Some("smoke".into()),
            workspace: home.join("workspace"),
            skills_dir: home.join("skills"),
            config_home: home.clone(),
            github_api_base: None,
        };
        let (router, token, source, driver) = latoile_server::build(&config, &db).await.unwrap();
        driver.abort(); // the smoke test never supervises anything
        assert_eq!(token, "smoke");
        assert_eq!(source, "config");

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            latoile_server::serve(listener, router, std::future::pending::<()>()).await
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream
            .write_all(b"GET /api/health HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let text = String::from_utf8_lossy(&response);
        assert!(text.starts_with("HTTP/1.1 200"), "{text}");
        assert!(text.contains("\"status\":\"ok\""), "{text}");

        server.abort();
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn the_vendored_skills_are_seeded_without_overwriting() {
        let home = std::env::temp_dir().join(format!("latoile-skills-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();

        ensure_skills(&home).unwrap();
        let manager = home.join("skills/project-manager/SKILL.md");
        assert!(manager.exists(), "the manager skill was seeded");
        let seeded = std::fs::read_to_string(&manager).unwrap();
        assert!(seeded.contains("latoile-actions"), "wire format documented");

        // A user edit survives a re-seed.
        std::fs::write(&manager, "mine").unwrap();
        ensure_skills(&home).unwrap();
        assert_eq!(std::fs::read_to_string(&manager).unwrap(), "mine");

        std::fs::remove_dir_all(&home).ok();
    }
}
