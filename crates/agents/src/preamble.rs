//! Role → skill preamble. Each LaToile role runs with a system preamble read
//! from `<skills_dir>/<skill>/SKILL.md`; a role with no skill file degrades
//! to its name only, so a missing skill is a thin agent, never a crash.
//!
//! The directory is injected: production points it at the deployed skills
//! tree, tests point it at a tempdir. Nothing here reads a hardcoded path.

use latoile_core::ids::RoleId;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub(crate) const ARCHITECT_SKILL_FILES: &[&str] = &[
    "SKILL.md",
    "references/brainstorming-method.md",
    "references/reverse-architecture.md",
    "references/database-modeling-uml.md",
    "references/archetype-patterns.md",
    "references/stack-selection-trees.md",
    "references/ui-ux-design.md",
    "references/ui-patterns.md",
    "references/development-guardian.md",
    "assets/ARCHITECTURE_CONTRACT.md",
    "assets/VALIDATION-CHECKLIST.md",
    "assets/templates/architecture-audit-template.md",
    "assets/templates/arch-spec-template.md",
    "assets/templates/adr-template.md",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDocument {
    pub path: String,
    pub contents: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchitectSkillBundle {
    pub name: String,
    pub digest: String,
    pub documents: Vec<SkillDocument>,
}

impl ArchitectSkillBundle {
    pub fn render(&self) -> String {
        let mut rendered = format!(
            "PINNED SKILL BUNDLE\nname: {}\nsha256: {}\nfiles: {}\n",
            self.name,
            self.digest,
            self.documents.len()
        );
        for document in &self.documents {
            rendered.push_str(&format!(
                "\n<skill-document path=\"{}\">\n{}\n</skill-document>\n",
                document.path, document.contents
            ));
        }
        rendered
    }
}

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

    /// The Architect is the exception to thin-role fallback: architecture
    /// decisions and mockups are only trusted when every mandatory skill
    /// reference is present and content-addressed as one bundle.
    pub fn architect_bundle(&self) -> std::io::Result<ArchitectSkillBundle> {
        let root = self.skills_dir.join("app-architect-brainstorm");
        let mut documents = Vec::with_capacity(ARCHITECT_SKILL_FILES.len());
        let mut hasher = Sha256::new();
        for relative in ARCHITECT_SKILL_FILES {
            let contents = std::fs::read_to_string(root.join(relative))?;
            if contents.trim().is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("mandatory Architect skill document is empty: {relative}"),
                ));
            }
            hasher.update(relative.as_bytes());
            hasher.update([0]);
            hasher.update(contents.as_bytes());
            hasher.update([0]);
            documents.push(SkillDocument {
                path: relative.to_string(),
                contents,
            });
        }
        Ok(ArchitectSkillBundle {
            name: "app-architect-brainstorm".into(),
            digest: format!("{:x}", hasher.finalize()),
            documents,
        })
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

    #[test]
    fn the_architect_bundle_is_complete_and_content_addressed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("app-architect-brainstorm");
        for relative in ARCHITECT_SKILL_FILES {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!("material for {relative}")).unwrap();
        }
        let preambles = Preambles::new(dir.path().to_path_buf());
        let first = preambles.architect_bundle().unwrap();
        let second = preambles.architect_bundle().unwrap();
        assert_eq!(first, second);
        assert_eq!(first.documents.len(), ARCHITECT_SKILL_FILES.len());
        assert_eq!(first.digest.len(), 64);
        assert!(first.render().contains("references/ui-ux-design.md"));

        std::fs::write(root.join("references/ui-ux-design.md"), "changed").unwrap();
        assert_ne!(preambles.architect_bundle().unwrap().digest, first.digest);
    }
}
