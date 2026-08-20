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
    write_root: Option<&Path>,
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
        ("manager" | "architect_discovery", true) => Decision::Reject,
        ("architect_package", true) => {
            if has_command || !package_paths_are_confined(raw_input, workspace, write_root) {
                Decision::Reject
            } else {
                Decision::AllowOnce
            }
        }
        (_, true) => Decision::Ask,
        _ => Decision::AllowOnce,
    }
}

fn package_paths_are_confined(
    raw_input: Option<&serde_json::Value>,
    workspace: &Path,
    write_root: Option<&Path>,
) -> bool {
    let Some(write_root) = write_root else {
        return false;
    };
    let mut candidates = Vec::new();
    collect_path_values(raw_input, None, &mut candidates);
    !candidates.is_empty()
        && candidates.into_iter().all(|candidate| {
            let path = Path::new(candidate);
            if path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            }) && !path.is_absolute()
            {
                return false;
            }
            let resolved = if path.is_absolute() {
                path.to_path_buf()
            } else {
                workspace.join(path)
            };
            resolved.starts_with(write_root)
                && matches!(
                    resolved.extension().and_then(|value| value.to_str()),
                    Some("md" | "html")
                )
        })
}

fn collect_path_values<'a>(
    value: Option<&'a serde_json::Value>,
    key: Option<&str>,
    output: &mut Vec<&'a str>,
) {
    let Some(value) = value else {
        return;
    };
    match value {
        serde_json::Value::Object(object) => {
            for (name, value) in object {
                collect_path_values(Some(value), Some(name), output);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_path_values(Some(value), key, output);
            }
        }
        serde_json::Value::String(value)
            if key.is_some_and(|name| {
                let name = name.to_ascii_lowercase();
                name.contains("path") || name == "file" || name == "filename"
            }) =>
        {
            output.push(value)
        }
        _ => {}
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
            decide("backend", Some("Read src/main.rs"), Some(&raw), &ws(), None),
            Decision::AllowOnce
        );
        assert_eq!(
            decide("backend", None, None, &ws(), None),
            Decision::AllowOnce
        );
    }

    #[test]
    fn commands_and_mutations_ask_the_owner() {
        let edit = json!({"file_path": "/srv/latoile/projects/mon-app/src/main.rs"});
        assert_eq!(
            decide(
                "backend",
                Some("Edit src/main.rs"),
                Some(&edit),
                &ws(),
                None,
            ),
            Decision::Ask
        );
        let command = json!({"command": "cargo test"});
        assert_eq!(
            decide("backend", Some("Bash"), Some(&command), &ws(), None),
            Decision::Ask
        );
    }

    #[test]
    fn the_manager_cannot_obtain_execution_permissions() {
        let command = json!({"command": "cargo test"});
        assert_eq!(
            decide("manager", Some("Bash"), Some(&command), &ws(), None),
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
                decide("backend", Some("Bash"), Some(&raw), &ws(), None),
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

    #[test]
    fn architect_package_writes_are_confined_and_commands_never_allowed() {
        let root = ws().join("design/v1-as1");
        let allowed = json!({"file_path": "design/v1-as1/mockups/home.html"});
        assert_eq!(
            decide(
                "architect_package",
                Some("Write file"),
                Some(&allowed),
                &ws(),
                Some(&root),
            ),
            Decision::AllowOnce
        );
        for denied in [
            json!({"file_path": "src/main.rs"}),
            json!({"file_path": "design/v1-as1/app.ts"}),
            json!({"command": "printf x > design/v1-as1/spec.md"}),
        ] {
            assert_eq!(
                decide(
                    "architect_package",
                    Some("Write file"),
                    Some(&denied),
                    &ws(),
                    Some(&root),
                ),
                Decision::Reject
            );
        }
    }
}
