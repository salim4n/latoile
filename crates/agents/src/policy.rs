//! Pure, fail-closed ACP permission policy.
//!
//! Hard-denied requests never become owner-grantable. Read-only operations
//! inside the workspace are allowed once. Commands and mutations become an
//! explicit human decision for executor roles; the Manager cannot acquire
//! those execution rights.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    AllowOnce,
    Ask,
    Reject,
}

const FORBIDDEN_NEEDLES: &[&str] = &[".env", "docker"];
const MUTATION_HINTS: &[&str] = &[
    "bash", "command", "create", "delete", "edit", "execute", "move", "patch", "rename", "shell",
    "terminal", "write",
];

pub fn decide(
    role_id: &str,
    title: Option<&str>,
    raw_input: Option<&serde_json::Value>,
    workspace: &Path,
) -> Decision {
    let mut haystack = title.unwrap_or_default().to_string();
    if let Some(raw) = raw_input {
        haystack.push_str(&raw.to_string());
    }
    let lowered = haystack.to_lowercase();

    if FORBIDDEN_NEEDLES
        .iter()
        .any(|needle| lowered.contains(needle))
        || contains_workspace_escape(&haystack, workspace)
    {
        return Decision::Reject;
    }

    let has_command = raw_input
        .and_then(serde_json::Value::as_object)
        .is_some_and(|object| object.contains_key("command"));
    let title_lowered = title.unwrap_or_default().to_lowercase();
    let mutating = has_command
        || MUTATION_HINTS
            .iter()
            .any(|hint| title_lowered.contains(hint));

    match (role_id, mutating) {
        ("manager", true) => Decision::Reject,
        (_, true) => Decision::Ask,
        _ => Decision::AllowOnce,
    }
}

/// Owner-visible text intentionally contains no raw command, path or title:
/// ACP metadata is agent-controlled and may contain a secret. The operation
/// class plus task/run context is enough to make a safe V1 decision.
pub fn sanitized_summary(title: Option<&str>, raw_input: Option<&serde_json::Value>) -> String {
    let has_command = raw_input
        .and_then(serde_json::Value::as_object)
        .is_some_and(|object| object.contains_key("command"));
    if has_command {
        "Execute a command inside the project workspace".into()
    } else {
        let title_lowered = title.unwrap_or_default().to_lowercase();
        if MUTATION_HINTS
            .iter()
            .any(|hint| title_lowered.contains(hint))
        {
            "Modify files inside the project workspace".into()
        } else {
            "Use a read-only tool inside the project workspace".into()
        }
    }
}

fn contains_workspace_escape(haystack: &str, workspace: &Path) -> bool {
    haystack
        .split(|c: char| !(c.is_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | '~')))
        .filter(|token| token.starts_with('/'))
        .any(|token| !Path::new(token).starts_with(workspace))
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
    fn read_only_workspace_operations_are_allowed_once() {
        let raw = json!({"file_path": "/srv/latoile/projects/mon-app/src/main.rs"});
        assert_eq!(
            decide("backend", Some("Read src/main.rs"), Some(&raw), &ws()),
            Decision::AllowOnce
        );
        assert_eq!(decide("backend", None, None, &ws()), Decision::AllowOnce);
    }

    #[test]
    fn commands_and_mutations_ask_the_owner() {
        let edit = json!({"file_path": "/srv/latoile/projects/mon-app/src/main.rs"});
        assert_eq!(
            decide("backend", Some("Edit src/main.rs"), Some(&edit), &ws()),
            Decision::Ask
        );
        let command = json!({"command": "cargo test"});
        assert_eq!(
            decide("backend", Some("Bash"), Some(&command), &ws()),
            Decision::Ask
        );
    }

    #[test]
    fn the_manager_cannot_obtain_execution_permissions() {
        let command = json!({"command": "cargo test"});
        assert_eq!(
            decide("manager", Some("Bash"), Some(&command), &ws()),
            Decision::Reject
        );
    }

    #[test]
    fn hard_denials_never_become_ask_decisions() {
        for raw in [
            json!({"file_path": "/srv/latoile/projects/mon-app/.env"}),
            json!({"command": "docker compose up -d"}),
            json!({"command": "cat /etc/passwd"}),
            json!({"file_path": "/srv/latoile/other-project/src/lib.rs"}),
        ] {
            assert_eq!(
                decide("backend", Some("Bash"), Some(&raw), &ws()),
                Decision::Reject
            );
        }
    }

    #[test]
    fn summaries_do_not_echo_agent_controlled_input() {
        let raw = json!({"command": "TOKEN=super-secret deploy --password hunter2"});
        let summary = sanitized_summary(Some("Bash super-secret"), Some(&raw));
        assert_eq!(summary, "Execute a command inside the project workspace");
        assert!(!summary.contains("secret"));
        assert!(!summary.contains("hunter2"));
    }
}
