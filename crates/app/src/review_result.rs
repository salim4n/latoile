//! Versioned Reviewer output contract. Agent text is untrusted: only a
//! validated `latoile-review` JSON document reaches the human approval
//! surface. Legacy or malformed output becomes an explicit, actionable
//! fallback rather than crashing supervision or pretending the review ran.

use serde::{Deserialize, Serialize};

pub const REVIEW_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Approve,
    ApproveWithReservations,
    ChangesRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Blocking,
    Reservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub severity: FindingSeverity,
    pub text: String,
    pub location: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewDiff {
    pub file: String,
    pub additions: u32,
    pub deletions: u32,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFrame {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cta: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComparison {
    pub spec_version: u32,
    pub target: ReviewFrame,
    pub render: ReviewFrame,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_spacing_px: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_spacing_px: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewResultV1 {
    pub schema_version: u8,
    pub verdict: ReviewVerdict,
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    #[serde(default)]
    pub suggested_follow_ups: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<ReviewDiff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<ReviewComparison>,
}

impl ReviewResultV1 {
    fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != REVIEW_SCHEMA_VERSION {
            return Err("unsupported schema version");
        }
        if self.summary.trim().is_empty() || self.summary.len() > 8 * 1024 {
            return Err("summary is empty or too long");
        }
        if self.findings.len() > 50 || self.suggested_follow_ups.len() > 20 {
            return Err("too many review items");
        }
        for finding in &self.findings {
            if finding.text.trim().is_empty()
                || finding.text.len() > 2 * 1024
                || finding.location.trim().is_empty()
                || finding.location.len() > 1024
                || finding.fix.as_ref().is_some_and(|fix| fix.len() > 2 * 1024)
            {
                return Err("invalid finding");
            }
        }
        if self
            .suggested_follow_ups
            .iter()
            .any(|item| item.trim().is_empty() || item.len() > 2 * 1024)
        {
            return Err("invalid follow-up");
        }
        match self.verdict {
            ReviewVerdict::Approve if !self.findings.is_empty() => {
                return Err("approve cannot carry findings");
            }
            ReviewVerdict::ApproveWithReservations
                if !self
                    .findings
                    .iter()
                    .any(|finding| finding.severity == FindingSeverity::Reservation) =>
            {
                return Err("reservation verdict needs a reservation");
            }
            ReviewVerdict::ChangesRequested
                if !self
                    .findings
                    .iter()
                    .any(|finding| finding.severity == FindingSeverity::Blocking) =>
            {
                return Err("changes requested needs a blocking finding");
            }
            _ => {}
        }
        if let Some(diff) = &self.diff {
            if diff.file.trim().is_empty()
                || diff.file.len() > 2 * 1024
                || diff.lines.is_empty()
                || diff.lines.len() > 400
                || diff.lines.iter().any(|line| line.len() > 4 * 1024)
            {
                return Err("invalid diff excerpt");
            }
        }
        Ok(())
    }
}

/// Parse a pure JSON response or the first fenced `latoile-review` block.
/// The returned string is always valid V1 JSON for the existing UI.
pub fn review_payload(output: &str) -> String {
    let parsed = extract_contract(output)
        .and_then(|raw| serde_json::from_str::<ReviewResultV1>(raw).ok())
        .filter(|result| result.validate().is_ok());
    serialize(parsed.unwrap_or_else(|| {
        fallback(
            "Le Reviewer a terminé, mais sa réponse structurée est invalide ou absente.",
            "Relancer le Reviewer avec le contrat de sortie V1.",
        )
    }))
}

/// A Reviewer process that cannot start or finish is terminal evidence too:
/// make the failure visible to the owner and keep the decision recoverable.
pub fn review_failure_payload(reason: &str) -> String {
    let reason = truncate(reason.trim(), 1024);
    serialize(fallback(
        &format!("La review automatique est indisponible : {reason}"),
        "Corriger la cause puis relancer le Reviewer avant approbation.",
    ))
}

fn fallback(summary: &str, follow_up: &str) -> ReviewResultV1 {
    ReviewResultV1 {
        schema_version: REVIEW_SCHEMA_VERSION,
        verdict: ReviewVerdict::ChangesRequested,
        summary: summary.into(),
        findings: vec![ReviewFinding {
            severity: FindingSeverity::Blocking,
            text: "Aucun verdict Reviewer fiable n'est disponible pour cette exécution.".into(),
            location: "reviewer-output".into(),
            fix: Some(follow_up.into()),
        }],
        suggested_follow_ups: vec![follow_up.into()],
        diff: None,
        comparison: None,
    }
}

fn serialize(result: ReviewResultV1) -> String {
    serde_json::to_string(&result).expect("the fixed reviewer schema serializes")
}

fn extract_contract(output: &str) -> Option<&str> {
    const OPEN: &str = "```latoile-review";
    if let Some(after_open) = output.find(OPEN).map(|index| &output[index + OPEN.len()..]) {
        let body = after_open
            .strip_prefix("\r\n")
            .or_else(|| after_open.strip_prefix('\n'))?;
        return body.find("```").map(|end| body[..end].trim());
    }
    let trimmed = output.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn truncate(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut boundary = limit;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}…", &value[..boundary])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_json() -> String {
        serde_json::json!({
            "schema_version": 1,
            "verdict": "approve_with_reservations",
            "summary": "Conforme avec une amélioration non bloquante.",
            "findings": [{
                "severity": "reservation",
                "text": "Ajouter un état de chargement.",
                "location": "web/src/Login.tsx:42",
                "fix": "Désactiver le bouton pendant la requête."
            }],
            "suggested_follow_ups": ["Ajouter le test de double clic."],
            "diff": {
                "file": "web/src/Login.tsx",
                "additions": 4,
                "deletions": 1,
                "lines": ["-disabled={false}", "+disabled={busy}"]
            }
        })
        .to_string()
    }

    #[test]
    fn a_fenced_v1_result_is_validated_and_preserved() {
        let payload = review_payload(&format!(
            "Review complete.\n```latoile-review\n{}\n```",
            valid_json()
        ));
        let parsed: ReviewResultV1 = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.verdict, ReviewVerdict::ApproveWithReservations);
        assert_eq!(parsed.findings[0].location, "web/src/Login.tsx:42");
        assert!(parsed.diff.is_some());
    }

    #[test]
    fn malformed_and_legacy_results_become_an_honest_fallback() {
        for output in [
            "not json",
            r#"{"summary":"legacy"}"#,
            r#"{"schema_version":99}"#,
        ] {
            let payload: ReviewResultV1 = serde_json::from_str(&review_payload(output)).unwrap();
            assert_eq!(payload.verdict, ReviewVerdict::ChangesRequested);
            assert!(payload.summary.contains("invalide"));
            assert_eq!(payload.findings[0].severity, FindingSeverity::Blocking);
        }
    }

    #[test]
    fn a_failure_payload_is_bounded_and_actionable() {
        let payload: ReviewResultV1 =
            serde_json::from_str(&review_failure_payload(&"é".repeat(1000))).unwrap();
        assert!(payload.summary.len() < 1400);
        assert_eq!(payload.suggested_follow_ups.len(), 1);
    }
}
