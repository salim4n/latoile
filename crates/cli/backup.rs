//! Paired application-state backup. The SQLite snapshot and external vault
//! root key must travel together; project checkouts deliberately do not.
//! Restore never overwrites either live file and leaves the workspace alone.

use clap::Subcommand;
use latoile_app::store::Store;
use latoile_vault::{RootKey, Vault};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const RESTORE_MARKER: &str = ".restore-in-progress";
const MANIFEST: &str = "manifest.txt";
const DATABASE: &str = "latoile.db";
const MASTER_KEY: &str = "master.key";

#[derive(clap::Args)]
pub struct BackupArgs {
    #[command(subcommand)]
    pub action: BackupCommand,

    /// Override the live database file (default: <home>/latoile.db).
    #[arg(long, env = "LATOILE_DB")]
    pub db: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum BackupCommand {
    /// Create a private directory containing a consistent database and key.
    Create {
        #[arg(long)]
        output: PathBuf,
    },
    /// Restore into a stopped deployment without touching its workspace.
    Restore {
        #[arg(long)]
        input: PathBuf,
    },
}

pub async fn run(home: &Path, args: BackupArgs) -> Result<(), Box<dyn std::error::Error>> {
    let mut stdout = std::io::stdout();
    let environment_key = std::env::var(latoile_vault::MASTER_KEY_ENV).ok();
    match args.action {
        BackupCommand::Create { output } => {
            create_with_key(home, args.db, &output, environment_key, &mut stdout).await
        }
        BackupCommand::Restore { input } => {
            restore_with_key(home, args.db, &input, environment_key, &mut stdout).await
        }
    }
}

async fn create_with_key(
    home: &Path,
    db: Option<PathBuf>,
    output: &Path,
    environment_key: Option<String>,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let database = db.unwrap_or_else(|| home.join(DATABASE));
    if !database.is_file() {
        return Err(format!("database does not exist: {}", database.display()).into());
    }
    if output.exists() {
        return Err(format!("backup destination already exists: {}", output.display()).into());
    }
    let root_key = existing_root_key(home, environment_key.as_deref())?;

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = sibling_temp(output, "partial");
    let mut guard = PendingPath::directory(partial.clone())?;

    let source = Store::open(&database).await?;
    source.integrity_check().await?;
    let vault = Vault::open(&database, root_key.clone()).await?;
    let secret_count = vault.verify_all().await?;

    let snapshot = partial.join(DATABASE);
    source.backup_to(&snapshot).await?;
    restrict_file(&snapshot)?;
    let copy = Store::open(&snapshot).await?;
    copy.integrity_check().await?;

    write_restricted(
        &partial.join(MASTER_KEY),
        format!("{}\n", &*root_key.encode()).as_bytes(),
    )?;
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    write_restricted(
        &partial.join(MANIFEST),
        format!(
            "format=latoile-backup-v1\ncreated_unix={created}\ndatabase={DATABASE}\nmaster_key={MASTER_KEY}\n"
        )
        .as_bytes(),
    )?;

    drop(copy);
    drop(vault);
    drop(source);
    std::fs::rename(&partial, output)?;
    guard.commit();
    writeln!(
        out,
        "backup created at {} (database + master key, {secret_count} secret names verified)",
        output.display()
    )?;
    Ok(())
}

async fn restore_with_key(
    home: &Path,
    db: Option<PathBuf>,
    input: &Path,
    environment_key: Option<String>,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    if environment_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
    {
        return Err(format!(
            "{} is set; restore the matching key in the external secret manager or unset it before restoring the bundled key file",
            latoile_vault::MASTER_KEY_ENV
        )
        .into());
    }
    validate_manifest(input)?;
    let source_database = input.join(DATABASE);
    let source_key = input.join(MASTER_KEY);
    if !source_database.is_file() || !source_key.is_file() {
        return Err("backup is missing latoile.db or master.key".into());
    }

    let database = db.unwrap_or_else(|| home.join(DATABASE));
    let target_key = home.join(MASTER_KEY);
    if database.exists() || target_key.exists() {
        return Err(format!(
            "restore never overwrites live state; move {} and {} aside after stopping the service",
            database.display(),
            target_key.display()
        )
        .into());
    }
    std::fs::create_dir_all(home)?;
    if let Some(parent) = database.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let marker = home.join(RESTORE_MARKER);
    let validation_database = sibling_temp(&database, "validate");
    let install_database = sibling_temp(&database, "install");
    let install_key = sibling_temp(&target_key, "install");
    let mut guard = RestoreGuard::begin(
        marker.clone(),
        vec![
            validation_database.clone(),
            install_database.clone(),
            install_key.clone(),
        ],
    )?;

    std::fs::copy(&source_database, &validation_database)?;
    restrict_file(&validation_database)?;
    let encoded_key = std::fs::read_to_string(&source_key)?;
    let root_key = RootKey::decode(&encoded_key)?;

    let candidate = Store::open(&validation_database).await?;
    candidate.integrity_check().await?;
    let vault = Vault::open(&validation_database, root_key.clone()).await?;
    let secret_count = vault.verify_all().await?;
    candidate.backup_to(&install_database).await?;
    restrict_file(&install_database)?;
    write_restricted(
        &install_key,
        format!("{}\n", &*root_key.encode()).as_bytes(),
    )?;
    drop(vault);
    drop(candidate);

    // The marker makes this pair installation fail-closed if the machine
    // crashes between the two renames: `serve` refuses to start while it is
    // present. Existing workspace directories are never touched.
    std::fs::rename(&install_key, &target_key)?;
    if let Err(error) = std::fs::rename(&install_database, &database) {
        if std::fs::rename(&target_key, &install_key).is_err() {
            guard.preserve_marker();
        }
        return Err(error.into());
    }
    std::fs::remove_file(&marker)?;
    guard.commit();
    writeln!(
        out,
        "backup restored into {} (workspace preserved, {secret_count} secret names verified)",
        home.display()
    )?;
    Ok(())
}

fn existing_root_key(
    home: &Path,
    environment_key: Option<&str>,
) -> Result<RootKey, Box<dyn std::error::Error>> {
    if let Some(value) = environment_key.filter(|value| !value.trim().is_empty()) {
        return Ok(RootKey::decode(value)?);
    }
    let path = home.join(MASTER_KEY);
    if !path.is_file() {
        return Err(format!(
            "master key does not exist: {}; refusing to create an incomplete backup",
            path.display()
        )
        .into());
    }
    refuse_loose_permissions(&path)?;
    Ok(RootKey::decode(&std::fs::read_to_string(path)?)?)
}

fn validate_manifest(input: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let manifest = std::fs::read_to_string(input.join(MANIFEST))?;
    if !manifest
        .lines()
        .any(|line| line == "format=latoile-backup-v1")
        || !manifest.lines().any(|line| line == "database=latoile.db")
        || !manifest.lines().any(|line| line == "master_key=master.key")
    {
        return Err("unsupported or incomplete LaToile backup manifest".into());
    }
    Ok(())
}

fn sibling_temp(path: &Path, purpose: &str) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("latoile");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{purpose}-{}-{nonce}", std::process::id()))
}

fn write_restricted(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    Ok(())
}

fn restrict_file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn refuse_loose_permissions(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
        if mode != 0o600 {
            return Err(format!(
                "master key {} has mode {mode:o}; expected 600",
                path.display()
            )
            .into());
        }
    }
    Ok(())
}

struct PendingPath {
    path: PathBuf,
    committed: bool,
}

impl PendingPath {
    fn directory(path: PathBuf) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700).create(&path)?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir(&path)?;
        Ok(Self {
            path,
            committed: false,
        })
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for PendingPath {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

struct RestoreGuard {
    marker: PathBuf,
    temporary: Vec<PathBuf>,
    committed: bool,
    preserve_marker: bool,
}

impl RestoreGuard {
    fn begin(marker: PathBuf, temporary: Vec<PathBuf>) -> std::io::Result<Self> {
        write_restricted(&marker, b"LaToile restore in progress\n")?;
        Ok(Self {
            marker,
            temporary,
            committed: false,
            preserve_marker: false,
        })
    }

    fn commit(&mut self) {
        self.committed = true;
    }

    fn preserve_marker(&mut self) {
        self.preserve_marker = true;
    }
}

impl Drop for RestoreGuard {
    fn drop(&mut self) {
        if !self.committed {
            for path in &self.temporary {
                let _ = std::fs::remove_file(path);
                let wal = PathBuf::from(format!("{}-wal", path.display()));
                let shm = PathBuf::from(format!("{}-shm", path.display()));
                let _ = std::fs::remove_file(wal);
                let _ = std::fs::remove_file(shm);
            }
            if !self.preserve_marker {
                let _ = std::fs::remove_file(&self.marker);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use latoile_core::ports::{ProjectStore, SecretStore};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "latoile-backup-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn backup_restore_preserves_the_key_database_and_workspace() {
        let root = temp_root("round-trip");
        let source_home = root.join("source");
        let target_home = root.join("target");
        let output = root.join("backup");
        std::fs::create_dir_all(&source_home).unwrap();
        std::fs::create_dir_all(target_home.join("workspace/repo/.git")).unwrap();
        std::fs::write(target_home.join("workspace/repo/keep.txt"), "keep").unwrap();

        let source_db = source_home.join(DATABASE);
        let source_store = Store::open(&source_db).await.unwrap();
        let project = latoile_core::Project::new(
            latoile_core::ProjectId::new("p-backup").unwrap(),
            "Backup",
            "backup",
            "owner/backup",
            "work",
            target_home.join("workspace/repo").display().to_string(),
            "npm run dev -- --port $PORT",
        )
        .unwrap();
        ProjectStore::save(&source_store, &project).await.unwrap();
        let root_key = RootKey::generate();
        write_restricted(
            &source_home.join(MASTER_KEY),
            format!("{}\n", &*root_key.encode()).as_bytes(),
        )
        .unwrap();
        let source_vault = Vault::open(&source_db, root_key).await.unwrap();
        source_vault
            .put("github_token", "not-printed")
            .await
            .unwrap();
        drop(source_vault);
        drop(source_store);

        let mut create_output = Vec::new();
        create_with_key(&source_home, None, &output, None, &mut create_output)
            .await
            .unwrap();
        assert!(output.join(DATABASE).is_file());
        assert!(output.join(MASTER_KEY).is_file());
        assert!(!String::from_utf8(create_output)
            .unwrap()
            .contains("not-printed"));

        let mut restore_output = Vec::new();
        restore_with_key(&target_home, None, &output, None, &mut restore_output)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(target_home.join("workspace/repo/keep.txt")).unwrap(),
            "keep"
        );
        assert!(!target_home.join(RESTORE_MARKER).exists());

        let restored_store = Store::open(&target_home.join(DATABASE)).await.unwrap();
        restored_store.integrity_check().await.unwrap();
        assert!(ProjectStore::get(&restored_store, &project.id)
            .await
            .unwrap()
            .is_some());
        let restored_key =
            RootKey::decode(&std::fs::read_to_string(target_home.join(MASTER_KEY)).unwrap())
                .unwrap();
        let restored_vault = Vault::open(&target_home.join(DATABASE), restored_key)
            .await
            .unwrap();
        assert_eq!(restored_vault.verify_all().await.unwrap(), 1);
        assert_eq!(
            restored_vault.get("github_token").await.unwrap().as_deref(),
            Some("not-printed")
        );
        assert!(
            restore_with_key(&target_home, None, &output, None, &mut Vec::new())
                .await
                .is_err()
        );

        drop(restored_vault);
        drop(restored_store);
        std::fs::remove_dir_all(root).ok();
    }
}
