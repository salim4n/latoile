//! Role → skill preamble. Each LaToile role runs with a system preamble read
//! from `<skills_dir>/<skill>/SKILL.md`; a role with no skill file degrades
//! to its name only, so a missing skill is a thin agent, never a crash.
//!
//! The directory is injected: production points it at the deployed skills
//! tree, tests point it at a tempdir. Nothing here reads a hardcoded path.

use latoile_core::ids::RoleId;
use std::path::PathBuf;

/// The fixed mapping from role id to skill directory
/// (architecture-spec.md §3.3, and the skills tree convention).
fn skill_dir_for(role: &str) -> &str {
    match role {
        "manager" => "project-manager",
        "architect" => "app-architect-brainstorm",
        "backend" => "backend-engineer",
        "frontend" => "frontend-engineer",
        "reviewer" => "code-reviewer",
        // A role LaToile doesn't know gets a directory named after itself —
        // which usually doesn't exist, and the fallback below takes over.
        other => other,
    }
}

#[derive(Debug, Clone)]
pub struct Preambles {
    skills_dir: PathBuf,
}

impl Preambles {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self { skills_dir }
    }

    /// The preamble a session starts with. Missing or unreadable skill file
    /// → the role name alone, which is still a usable (if thin) instruction.
    pub fn for_role(&self, role: &RoleId) -> String {
        let path = self
            .skills_dir
            .join(skill_dir_for(role.as_str()))
            .join("SKILL.md");
        match std::fs::read_to_string(&path) {
            Ok(contents) if !contents.trim().is_empty() => contents,
            _ => role.as_str().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixed_roles_map_to_their_skills() {
        assert_eq!(skill_dir_for("manager"), "project-manager");
        assert_eq!(skill_dir_for("architect"), "app-architect-brainstorm");
        assert_eq!(skill_dir_for("backend"), "backend-engineer");
        assert_eq!(skill_dir_for("frontend"), "frontend-engineer");
        assert_eq!(skill_dir_for("reviewer"), "code-reviewer");
    }

    #[test]
    fn a_skill_file_becomes_the_preamble() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("project-manager");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "Tu es le Manager du projet.").unwrap();

        let preambles = Preambles::new(dir.path().to_path_buf());
        assert_eq!(
            preambles.for_role(&RoleId::new("manager").unwrap()),
            "Tu es le Manager du projet."
        );
    }

    #[test]
    fn a_missing_skill_degrades_to_the_role_name() {
        let dir = tempfile::tempdir().unwrap();
        let preambles = Preambles::new(dir.path().to_path_buf());
        assert_eq!(
            preambles.for_role(&RoleId::new("frontend").unwrap()),
            "frontend"
        );
    }

    #[test]
    fn an_empty_skill_file_is_like_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("code-reviewer");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "  \n").unwrap();

        let preambles = Preambles::new(dir.path().to_path_buf());
        assert_eq!(
            preambles.for_role(&RoleId::new("reviewer").unwrap()),
            "reviewer"
        );
    }
}
