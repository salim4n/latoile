//! The Manager's actions wire format. The Manager answers in prose; machine
//! intent travels in a fenced block inside that prose:
//!
//! ````text
//! Bien reçu, je crée la tâche et je la confie au Frontend.
//!
//! ```latoile-actions
//! [
//!   {"type": "create_tasks", "tasks": [
//!     {"title": "Login page", "role_id": "frontend", "description": "Email + password form"}
//!   ]},
//!   {"type": "dispatch_task", "title": "Login page", "role_id": "frontend",
//!    "prompt": "Build the login page per design/"},
//!   {"type": "propose_spec", "design_dir": "design/"}
//! ]
//! ```
//! ````
//!
//! The prose is what the thread shows (the block is stripped); the block is
//! what the executor runs. Everything malformed becomes a warning — the
//! reply itself is never lost over a bad action.
//!
//! Pure parsing, no I/O: exhaustively unit-tested.

/// One structured intent. Field names are the wire contract — the
/// project-manager skill documents them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerAction {
    /// Put tasks on the board without starting anything.
    CreateTasks { tasks: Vec<NewTask> },
    /// Create a task AND start its run (DispatchTask; refuses without an
    /// approved spec — the refusal is a card, not a crash).
    DispatchTask {
        title: String,
        role_id: String,
        prompt: String,
    },
    /// Register a new draft spec version for the project.
    ProposeSpec { design_dir: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTask {
    pub title: String,
    pub role_id: String,
    pub description: String,
}

/// What a reply parses into: the prose to display, the actions to run, and
/// everything that was wrong with the block(s).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReply {
    pub display_text: String,
    pub actions: Vec<ManagerAction>,
    pub warnings: Vec<String>,
}

const FENCE: &str = "```latoile-actions";

/// Split the prose from the fenced action blocks, parse each block, merge.
pub fn parse_reply(content: &str) -> ParsedReply {
    let mut display = String::new();
    let mut actions = Vec::new();
    let mut warnings = Vec::new();

    let mut rest = content;
    loop {
        let Some(start) = rest.find(FENCE) else {
            display.push_str(rest);
            break;
        };
        display.push_str(&rest[..start]);
        let after_fence = &rest[start + FENCE.len()..];
        // The block runs to the closing fence on its own line.
        let Some(end) = after_fence.find("```") else {
            warnings.push("an actions block was never closed".into());
            break;
        };
        let body = after_fence[..end].trim();
        parse_block(body, &mut actions, &mut warnings);
        rest = &after_fence[end + 3..];
    }

    ParsedReply {
        display_text: display.trim().to_string(),
        actions,
        warnings,
    }
}

fn parse_block(body: &str, actions: &mut Vec<ManagerAction>, warnings: &mut Vec<String>) {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(format!("malformed actions block: {e}"));
            return;
        }
    };
    let Some(entries) = value.as_array() else {
        warnings.push("an actions block must be a JSON array".into());
        return;
    };
    for entry in entries {
        match parse_action(entry) {
            Ok(action) => actions.push(action),
            Err(warning) => warnings.push(warning),
        }
    }
}

fn parse_action(value: &serde_json::Value) -> Result<ManagerAction, String> {
    let kind = value
        .get("type")
        .and_then(|t| t.as_str())
        .ok_or_else(|| format!("an action has no \"type\": {value}"))?;
    match kind {
        "create_tasks" => {
            let tasks = value
                .get("tasks")
                .and_then(|t| t.as_array())
                .ok_or("\"create_tasks\" needs a \"tasks\" array")?;
            let mut parsed = Vec::new();
            for task in tasks {
                parsed.push(NewTask {
                    title: required(task, "title")?,
                    role_id: required(task, "role_id")?,
                    description: optional(task, "description"),
                });
            }
            Ok(ManagerAction::CreateTasks { tasks: parsed })
        }
        "dispatch_task" => Ok(ManagerAction::DispatchTask {
            title: required(value, "title")?,
            role_id: required(value, "role_id")?,
            prompt: optional(value, "prompt"),
        }),
        "propose_spec" => Ok(ManagerAction::ProposeSpec {
            design_dir: {
                let dir = optional(value, "design_dir");
                if dir.is_empty() { "design/".into() } else { dir }
            },
        }),
        other => Err(format!("unknown action type {other:?}")),
    }
}

fn required(value: &serde_json::Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("an action is missing {field:?}: {value}"))
}

fn optional(value: &serde_json::Value, field: &str) -> String {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_without_a_block_is_just_prose() {
        let parsed = parse_reply("Bien reçu, je regarde ça.");
        assert_eq!(parsed.display_text, "Bien reçu, je regarde ça.");
        assert!(parsed.actions.is_empty());
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn a_full_block_parses_and_leaves_the_prose_clean() {
        let content = "Je crée la tâche.\n\n```latoile-actions\n[\n  {\"type\": \"create_tasks\", \"tasks\": [{\"title\": \"Login page\", \"role_id\": \"frontend\", \"description\": \"Form\"}]},\n  {\"type\": \"dispatch_task\", \"title\": \"Login page\", \"role_id\": \"frontend\", \"prompt\": \"Build it\"},\n  {\"type\": \"propose_spec\", \"design_dir\": \"design/\"}\n]\n```\n\nSuite bientôt.";
        let parsed = parse_reply(content);
        assert_eq!(parsed.display_text, "Je crée la tâche.\n\n\n\nSuite bientôt.");
        assert_eq!(parsed.actions.len(), 3);
        assert!(parsed.warnings.is_empty());
        assert!(matches!(
            &parsed.actions[0],
            ManagerAction::CreateTasks { tasks } if tasks[0].title == "Login page"
        ));
        assert!(matches!(
            &parsed.actions[1],
            ManagerAction::DispatchTask { prompt, .. } if prompt == "Build it"
        ));
        assert!(matches!(
            &parsed.actions[2],
            ManagerAction::ProposeSpec { design_dir } if design_dir == "design/"
        ));
    }

    #[test]
    fn multiple_blocks_merge() {
        let content = "```latoile-actions\n[{\"type\": \"propose_spec\"}]\n```\ntexte\n```latoile-actions\n[{\"type\": \"dispatch_task\", \"title\": \"T\", \"role_id\": \"backend\"}]\n```";
        let parsed = parse_reply(content);
        assert_eq!(parsed.actions.len(), 2);
        assert_eq!(parsed.display_text, "texte");
        // Defaults: design_dir, and prompt falls back to empty.
        assert_eq!(
            parsed.actions[0],
            ManagerAction::ProposeSpec {
                design_dir: "design/".into()
            }
        );
    }

    #[test]
    fn malformed_json_is_a_warning_not_a_crash() {
        let parsed = parse_reply("```latoile-actions\n[{oops]\n```");
        assert!(parsed.actions.is_empty());
        assert_eq!(parsed.warnings.len(), 1);
        assert!(parsed.warnings[0].contains("malformed"));
    }

    #[test]
    fn a_non_array_block_is_a_warning() {
        let parsed = parse_reply("```latoile-actions\n{\"type\": \"propose_spec\"}\n```");
        assert!(parsed.warnings[0].contains("array"));
    }

    #[test]
    fn an_unclosed_block_is_a_warning_and_keeps_the_prose() {
        let parsed = parse_reply("Voici.\n```latoile-actions\n[{\"type\": \"propose_spec\"}]");
        assert_eq!(parsed.display_text, "Voici.");
        assert!(parsed.warnings[0].contains("never closed"));
    }

    #[test]
    fn unknown_types_and_missing_fields_warn_but_keep_the_rest() {
        let parsed = parse_reply(
            "```latoile-actions\n[{\"type\": \"nuke\"}, {\"type\": \"dispatch_task\", \"role_id\": \"backend\"}, {\"type\": \"propose_spec\"}]\n```",
        );
        assert_eq!(parsed.warnings.len(), 2);
        assert!(parsed.warnings[0].contains("unknown action type"));
        assert!(parsed.warnings[1].contains("title"));
        assert_eq!(parsed.actions.len(), 1, "the valid action survives");
    }

    #[test]
    fn create_tasks_requires_the_tasks_array() {
        let parsed = parse_reply("```latoile-actions\n[{\"type\": \"create_tasks\"}]\n```");
        assert!(parsed.warnings[0].contains("tasks"));
    }
}
