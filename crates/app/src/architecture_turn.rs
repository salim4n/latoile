//! Strict output contract for the Socratic Architect. Discovery output is
//! untrusted provider text: only a validated question or ready signal may
//! advance the persisted state machine.

use latoile_core::ArchitecturePhase;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchitectureTurnKind {
    Question,
    ReadyToDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchitectureTurn {
    pub kind: ArchitectureTurnKind,
    pub phase: ArchitecturePhase,
    pub message: String,
}

#[derive(Deserialize)]
struct WireTurn {
    schema_version: u8,
    kind: String,
    phase: String,
    message: String,
}

pub fn parse_architecture_turn(output: &str) -> Result<ArchitectureTurn, &'static str> {
    let raw = extract(output).ok_or("missing latoile-architecture contract")?;
    let wire: WireTurn =
        serde_json::from_str(raw).map_err(|_| "invalid latoile-architecture JSON")?;
    if wire.schema_version != 1 || wire.message.trim().is_empty() || wire.message.len() > 8 * 1024 {
        return Err("invalid latoile-architecture fields");
    }
    let kind = match wire.kind.as_str() {
        "question" => ArchitectureTurnKind::Question,
        "ready_to_draft" => ArchitectureTurnKind::ReadyToDraft,
        _ => return Err("unknown latoile-architecture kind"),
    };
    let phase = match wire.phase.as_str() {
        "domain_discovery" => ArchitecturePhase::DomainDiscovery,
        "requirements" => ArchitecturePhase::Requirements,
        "ux_discovery" => ArchitecturePhase::UxDiscovery,
        "ready_to_draft" => ArchitecturePhase::ReadyToDraft,
        _ => return Err("unknown latoile-architecture phase"),
    };
    if matches!(kind, ArchitectureTurnKind::Question) == (phase == ArchitecturePhase::ReadyToDraft)
    {
        return Err("question/ready phase mismatch");
    }
    Ok(ArchitectureTurn {
        kind,
        phase,
        message: wire.message,
    })
}

fn extract(output: &str) -> Option<&str> {
    const OPEN: &str = "```latoile-architecture";
    let after = output
        .find(OPEN)
        .map(|index| &output[index + OPEN.len()..])?;
    let body = after
        .strip_prefix("\r\n")
        .or_else(|| after.strip_prefix('\n'))?;
    body.find("```").map(|end| body[..end].trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_questions_and_ready_signals_only() {
        let question = parse_architecture_turn(
            "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"question\",\"phase\":\"domain_discovery\",\"message\":\"Qui utilise le produit ?\"}\n```",
        )
        .unwrap();
        assert_eq!(question.kind, ArchitectureTurnKind::Question);
        assert_eq!(question.phase, ArchitecturePhase::DomainDiscovery);

        let ready = parse_architecture_turn(
            "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"ready_to_draft\",\"phase\":\"ready_to_draft\",\"message\":\"Décisions suffisantes.\"}\n```",
        )
        .unwrap();
        assert_eq!(ready.kind, ArchitectureTurnKind::ReadyToDraft);

        for invalid in [
            "plain text",
            "```latoile-architecture\n{}\n```",
            "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"question\",\"phase\":\"ready_to_draft\",\"message\":\"x\"}\n```",
        ] {
            assert!(parse_architecture_turn(invalid).is_err());
        }
    }
}
