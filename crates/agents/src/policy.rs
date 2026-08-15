//! Permission decisions — the answer LaToile gives when an agent asks
//! "may I?". Pure and total: the agent gets an answer immediately, and the
//! request is surfaced as an [`AgentUpdate::PermissionRequested`] so the
//! owner sees what was decided (journaled as `ApprovalRequested`).
//!
//! The rules (architecture contract §3):
//!
//! - **Reject** anything touching `.env`, anything invoking `docker`, and any
//!   absolute path outside the workspace.
//! - **Allow once** everything else — a coding agent that cannot edit the
//!   project it was spawned on is useless, and "once" (never "always") keeps
//!   each decision visible.
//!
//! A real human-in-the-loop round-trip (park the run, ask the owner, resume)
//! needs the app layer to orchestrate; until that wiring exists the policy is
//! deliberately fail-closed on the dangerous patterns rather than blocking
//! runs on everything.

use std::path::Path;

/// What the policy concluded about one tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    AllowOnce,
    Reject,
}

/// Tokens that are never allowed, wherever they appear.
const FORBIDDEN_NEEDLES: &[&str] = &[".env", "docker"];

/// Decide for one tool call. `title` and `raw_input` are what the agent
/// declared about the call; `workspace` is the only absolute path prefix a
/// call may touch.
pub fn decide(
    title: Option<&str>,
    raw_input: Option<&serde_json::Value>,
    workspace: &Path,
) -> Decision {
    let mut haystack = title.unwrap_or_default().to_string();
    if let Some(raw) = raw_input {
        haystack.push_str(&raw.to_string());
    }
    let lowered = haystack.to_lowercase();

    if FORBIDDEN_NEEDLES.iter().any(|n| lowered.contains(n)) {
        return Decision::Reject;
    }

    // An absolute path outside the workspace is an escape attempt. JSON
    // stringification leaves quotes around paths, which is exactly what
    // split on non-path characters cleans up.
    let outside = haystack
        .split(|c: char| !(c.is_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | '~')))
        .filter(|token| token.starts_with('/'))
        .any(|token| !Path::new(token).starts_with(workspace));
    if outside {
        return Decision::Reject;
    }

    Decision::AllowOnce
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn ws() -> PathBuf {
        PathBuf::from("/srv/latoile/projects/mon-app")
    }

    #[test]
    fn editing_inside_the_workspace_is_allowed_once() {
        let raw = json!({"file_path": "/srv/latoile/projects/mon-app/src/main.rs"});
        assert_eq!(
            decide(Some("Edit src/main.rs"), Some(&raw), &ws()),
            Decision::AllowOnce
        );
    }

    #[test]
    fn relative_paths_are_allowed() {
        let raw = json!({"command": "cargo test"});
        assert_eq!(decide(None, Some(&raw), &ws()), Decision::AllowOnce);
        assert_eq!(decide(None, None, &ws()), Decision::AllowOnce);
    }

    #[test]
    fn dotenv_is_rejected_even_inside_the_workspace() {
        let raw = json!({"file_path": "/srv/latoile/projects/mon-app/.env"});
        assert_eq!(decide(Some("Read .env"), Some(&raw), &ws()), Decision::Reject);
        let lowercase = json!({"file_path": "/srv/latoile/projects/mon-app/.ENV"});
        assert_eq!(decide(None, Some(&lowercase), &ws()), Decision::Reject);
    }

    #[test]
    fn docker_is_rejected() {
        let raw = json!({"command": "docker compose up -d"});
        assert_eq!(decide(Some("Bash"), Some(&raw), &ws()), Decision::Reject);
    }

    #[test]
    fn absolute_paths_outside_the_workspace_are_rejected() {
        let raw = json!({"command": "cat /etc/passwd"});
        assert_eq!(decide(None, Some(&raw), &ws()), Decision::Reject);
        let raw = json!({"file_path": "/srv/latoile/other-project/src/lib.rs"});
        assert_eq!(decide(None, Some(&raw), &ws()), Decision::Reject);
    }

    #[test]
    fn a_mixed_call_is_judged_by_its_worst_token() {
        let raw = json!({"command": "cargo build && cp target/app /usr/local/bin/app"});
        assert_eq!(decide(None, Some(&raw), &ws()), Decision::Reject);
    }
}
