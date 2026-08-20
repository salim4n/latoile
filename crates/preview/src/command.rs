use crate::PreviewError;
use std::path::Path;

const LEGACY_MISSING_COMMAND: &str =
    "printf 'LaToile: no dev command detected; configure dev_command for this project\\n' >&2; exit 64";

/// Resolve automatic preview commands against the current checkout, not only
/// the repository state that existed when the project was first registered.
pub(crate) async fn resolve(configured: &str, working_dir: &str) -> Result<String, PreviewError> {
    let configured = configured.trim();
    if !configured.is_empty() && configured != LEGACY_MISSING_COMMAND {
        return Ok(configured.to_string());
    }

    detect(Path::new(working_dir))
        .await
        .ok_or(PreviewError::NoDevCommand)
}

async fn detect(checkout: &Path) -> Option<String> {
    let package = checkout.join("package.json");
    if let Ok(bytes) = tokio::fs::read(&package).await {
        let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let script = ["dev", "start"].into_iter().find(|name| {
            json.pointer(&format!("/scripts/{name}"))
                .and_then(|value| value.as_str())
                .is_some()
        });
        if let Some(script) = script {
            let (runner, separator) = if checkout.join("pnpm-lock.yaml").exists() {
                ("pnpm", "--")
            } else if checkout.join("yarn.lock").exists() {
                ("yarn", "")
            } else if checkout.join("bun.lock").exists() || checkout.join("bun.lockb").exists() {
                ("bun run", "--")
            } else {
                ("npm run", "--")
            };
            return Some(format!("{runner} {script} {separator} --port $PORT").replace("  ", " "));
        }
    }

    checkout
        .join("Cargo.toml")
        .exists()
        .then(|| "cargo run".to_string())
}
