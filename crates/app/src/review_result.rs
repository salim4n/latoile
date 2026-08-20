//! Reviewer output contracts and the server-owned V2 trust gate.
//!
//! V1 remains deserializable so historic approvals stay visible, but it can
//! never be granted as trusted evidence. V2 accepts only Reviewer judgement;
//! capture facts and their complete binding are reconstructed from immutable
//! server rows selected by the Reviewer's immutable subject run.
//! Schemas and gate construction deliberately stay in one module so no caller
//! can serialize a trusted envelope without traversing the same validation.

use latoile_core::ids::{ProjectId, RunId, SpecVersionId};
use latoile_core::{VisualComparison, VisualComparisonStatus};
use serde::{Deserialize, Serialize};

pub const REVIEW_SCHEMA_VERSION: u8 = 2;
pub const LEGACY_REVIEW_SCHEMA_VERSION: u8 = 1;

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
#[serde(deny_unknown_fields)]
pub struct ReviewFinding {
    pub severity: FindingSeverity,
    pub text: String,
    pub location: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewDiff {
    pub file: String,
    pub additions: u32,
    pub deletions: u32,
    pub lines: Vec<String>,
}

/// Legacy self-reported frames. Kept solely to decode existing records.
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

/// Legacy synthetic comparison. It is never promoted to trusted V2 proof.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualApplicability {
    Required,
    NotApplicable,
}

/// Deprecated untrusted echo accepted for wire compatibility. These values
/// never select or override evidence; the server binds the complete set from
/// the immutable reviewed run. New Reviewer prompts submit an empty list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewEvidenceReference {
    pub evidence_id: String,
    pub manifest_digest: String,
    pub baseline_png_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_png_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_diff_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heatmap_png_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry_diff_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility_diff_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewEvidenceDecision {
    pub applicability: VisualApplicability,
    /// Accepted for compatibility but never used as a trust input.
    #[serde(default)]
    pub references: Vec<ReviewEvidenceReference>,
}

/// Untrusted Reviewer V2 output. There is intentionally no `gate`, status or
/// metric field for the model to fill in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewResultV2 {
    pub schema_version: u8,
    pub verdict: ReviewVerdict,
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    #[serde(default)]
    pub suggested_follow_ups: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<ReviewDiff>,
    pub visual_evidence: ReviewEvidenceDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedEvidenceReference {
    pub evidence_id: String,
    pub comparison_id: String,
    pub status: String,
    pub manifest_digest: String,
    pub baseline_png_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_png_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_diff_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heatmap_png_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry_diff_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility_diff_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_digest: Option<String>,
    pub changed_pixels: u64,
    pub total_pixels: u64,
    pub pixel_ratio_micros: u32,
    pub max_geometry_delta_milli: u32,
    pub accessibility_changes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedEvidenceDecision {
    pub applicability: VisualApplicability,
    pub references: Vec<TrustedEvidenceReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTrustGate {
    pub trusted_v2: bool,
    pub approvable: bool,
    pub code: String,
    pub message: String,
}

/// The only review shape written by the V2 supervision path. Evidence status
/// and metrics always come from the server-side comparison aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedReviewResultV2 {
    pub schema_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_run_id: Option<String>,
    pub verdict: ReviewVerdict,
    pub summary: String,
    pub findings: Vec<ReviewFinding>,
    pub suggested_follow_ups: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<ReviewDiff>,
    pub visual_evidence: TrustedEvidenceDecision,
    pub gate: ReviewTrustGate,
}

pub struct ReviewTrustContext<'a> {
    pub project_id: &'a ProjectId,
    pub spec_version_id: Option<&'a SpecVersionId>,
    pub reviewed_run_id: &'a RunId,
    pub visual_required: bool,
    pub evidence: &'a [VisualComparison],
}

impl ReviewResultV2 {
    fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != REVIEW_SCHEMA_VERSION {
            return Err("unsupported schema version");
        }
        validate_common(
            &self.verdict,
            &self.summary,
            &self.findings,
            &self.suggested_follow_ups,
            self.diff.as_ref(),
        )?;
        if self.visual_evidence.references.len() > 50 {
            return Err("too many evidence references");
        }
        if self.visual_evidence.references.iter().any(|reference| {
            reference.evidence_id.len() > 256
                || reference.manifest_digest.len() > 256
                || reference.baseline_png_digest.len() > 256
                || [
                    &reference.render_png_digest,
                    &reference.pixel_diff_digest,
                    &reference.heatmap_png_digest,
                    &reference.geometry_diff_digest,
                    &reference.accessibility_diff_digest,
                    &reference.environment_digest,
                ]
                .into_iter()
                .flatten()
                .any(|value| value.len() > 256)
        }) {
            return Err("oversized legacy evidence reference");
        }
        Ok(())
    }
}

fn validate_common(
    verdict: &ReviewVerdict,
    summary: &str,
    findings: &[ReviewFinding],
    follow_ups: &[String],
    diff: Option<&ReviewDiff>,
) -> Result<(), &'static str> {
    if summary.trim().is_empty() || summary.len() > 8 * 1024 {
        return Err("summary is empty or too long");
    }
    if findings.len() > 50 || follow_ups.len() > 20 {
        return Err("too many review items");
    }
    for finding in findings {
        if finding.text.trim().is_empty()
            || finding.text.len() > 2 * 1024
            || finding.location.trim().is_empty()
            || finding.location.len() > 1024
            || finding.fix.as_ref().is_some_and(|fix| fix.len() > 2 * 1024)
        {
            return Err("invalid finding");
        }
    }
    if follow_ups
        .iter()
        .any(|item| item.trim().is_empty() || item.len() > 2 * 1024)
    {
        return Err("invalid follow-up");
    }
    match verdict {
        ReviewVerdict::Approve if !findings.is_empty() => {
            return Err("approve cannot carry findings");
        }
        ReviewVerdict::ApproveWithReservations
            if findings
                .iter()
                .any(|finding| finding.severity == FindingSeverity::Blocking)
                || !findings
                    .iter()
                    .any(|finding| finding.severity == FindingSeverity::Reservation) =>
        {
            return Err("reservation verdict needs reservations and no blocking finding");
        }
        ReviewVerdict::ChangesRequested
            if !findings
                .iter()
                .any(|finding| finding.severity == FindingSeverity::Blocking) =>
        {
            return Err("changes requested needs a blocking finding");
        }
        _ => {}
    }
    if let Some(diff) = diff {
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

/// Parse and gate one Reviewer response against the exact server context.
/// The returned JSON is always V2 and always carries an explicit trust result.
pub fn trusted_review_payload(output: &str, context: &ReviewTrustContext<'_>) -> String {
    let parsed = extract_contract(output)
        .and_then(|raw| serde_json::from_str::<ReviewResultV2>(raw).ok())
        .filter(|result| result.validate().is_ok());
    let Some(result) = parsed else {
        return serialize(fallback(
            Some(context.reviewed_run_id),
            "invalid_reviewer_output",
            "Le Reviewer a terminé, mais sa réponse V2 est invalide ou absente.",
            "Relancer le Reviewer avec le contrat de sortie V2.",
            context,
        ));
    };

    let gate = evaluate_gate(&result, context);
    let mut verdict = result.verdict;
    let mut findings = result.findings;
    if !gate.approvable {
        verdict = ReviewVerdict::ChangesRequested;
        if !findings
            .iter()
            .any(|finding| finding.severity == FindingSeverity::Blocking)
        {
            findings.push(blocking_finding(&gate.message, &gate.message));
        }
    }
    serialize(TrustedReviewResultV2 {
        schema_version: REVIEW_SCHEMA_VERSION,
        reviewed_run_id: Some(context.reviewed_run_id.as_str().to_string()),
        verdict,
        summary: result.summary,
        findings,
        suggested_follow_ups: result.suggested_follow_ups,
        diff: result.diff,
        visual_evidence: trusted_evidence(context),
        gate,
    })
}

/// A Reviewer process failure remains a V2-shaped, explicitly untrusted and
/// non-approvable record. Existing V1 records are untouched in storage.
pub fn review_failure_payload(reason: &str) -> String {
    let reason = truncate(reason.trim(), 1024);
    serialize(TrustedReviewResultV2 {
        schema_version: REVIEW_SCHEMA_VERSION,
        reviewed_run_id: None,
        verdict: ReviewVerdict::ChangesRequested,
        summary: format!("La review automatique est indisponible : {reason}"),
        findings: vec![blocking_finding(
            "Aucun verdict Reviewer fiable n'est disponible pour cette exécution.",
            "Corriger la cause puis relancer le Reviewer avant approbation.",
        )],
        suggested_follow_ups: vec![
            "Corriger la cause puis relancer le Reviewer avant approbation.".into(),
        ],
        diff: None,
        visual_evidence: TrustedEvidenceDecision {
            applicability: VisualApplicability::NotApplicable,
            references: vec![],
        },
        gate: ReviewTrustGate {
            trusted_v2: false,
            approvable: false,
            code: "reviewer_failed".into(),
            message: "Le run Reviewer a échoué ; aucune approbation n'est autorisée.".into(),
        },
    })
}

/// Granting is fail-closed. Historic V1 and malformed payloads return false.
pub fn review_payload_is_approvable(payload: &str) -> bool {
    serde_json::from_str::<TrustedReviewResultV2>(payload)
        .ok()
        .is_some_and(|result| {
            result.schema_version == REVIEW_SCHEMA_VERSION
                && result.reviewed_run_id.is_some()
                && result.gate.trusted_v2
                && result.gate.approvable
                && matches!(
                    result.verdict,
                    ReviewVerdict::Approve | ReviewVerdict::ApproveWithReservations
                )
        })
}

fn evaluate_gate(result: &ReviewResultV2, context: &ReviewTrustContext<'_>) -> ReviewTrustGate {
    let deny = |code: &str, message: &str| ReviewTrustGate {
        trusted_v2: false,
        approvable: false,
        code: code.into(),
        message: message.into(),
    };
    let trusted_block = |code: &str, message: &str| ReviewTrustGate {
        trusted_v2: true,
        approvable: false,
        code: code.into(),
        message: message.into(),
    };
    let Some(spec_version_id) = context.spec_version_id else {
        return deny(
            "missing_approved_spec",
            "Le run revu n'est pas lié à la spec actuellement approuvée.",
        );
    };
    if context.evidence.iter().any(|evidence| {
        &evidence.project_id != context.project_id
            || &evidence.run_id != context.reviewed_run_id
            || &evidence.spec_version_id != spec_version_id
    }) {
        return deny(
            "evidence_provenance_mismatch",
            "Une preuve appartient à un autre projet, run ou version de spec.",
        );
    }

    if context.visual_required {
        if result.visual_evidence.applicability != VisualApplicability::Required {
            return deny(
                "visual_evidence_required",
                "Ce run frontend exige une décision visual_evidence=required.",
            );
        }
        if context.evidence.is_empty() {
            return deny(
                "visual_evidence_missing",
                "Aucune comparaison visuelle serveur n'est disponible pour ce run frontend.",
            );
        }
        if context
            .evidence
            .iter()
            .any(|evidence| evidence.status == VisualComparisonStatus::Invalid)
        {
            return deny(
                "visual_evidence_invalid",
                "Au moins une capture est invalide et doit être régénérée.",
            );
        }
        if context
            .evidence
            .iter()
            .any(|evidence| evidence.status == VisualComparisonStatus::Blocking)
        {
            return trusted_block(
                "visual_evidence_blocking",
                "Au moins une comparaison visuelle dépasse un seuil bloquant.",
            );
        }
        if context
            .evidence
            .iter()
            .any(|evidence| evidence.status == VisualComparisonStatus::Reservation)
            && result.verdict == ReviewVerdict::Approve
        {
            return trusted_block(
                "visual_reservation_unacknowledged",
                "Le verdict doit expliciter les réserves détectées par le serveur.",
            );
        }
    } else {
        if result.visual_evidence.applicability != VisualApplicability::NotApplicable
            || !result.visual_evidence.references.is_empty()
        {
            return deny(
                "visual_evidence_not_applicable",
                "Ce run non visuel doit déclarer explicitement not_applicable sans référence.",
            );
        }
        if !context.evidence.is_empty() {
            return deny(
                "unexpected_visual_evidence",
                "Des preuves visuelles inattendues sont liées à un run non visuel.",
            );
        }
    }

    if result.verdict == ReviewVerdict::ChangesRequested {
        return trusted_block(
            "changes_requested",
            "Le Reviewer demande des corrections avant approbation.",
        );
    }
    ReviewTrustGate {
        trusted_v2: true,
        approvable: true,
        code: "trusted".into(),
        message: "Le verdict V2 est lié côté serveur aux preuves exactes du run et peut être décidé.".into(),
    }
}

fn trusted_evidence(context: &ReviewTrustContext<'_>) -> TrustedEvidenceDecision {
    TrustedEvidenceDecision {
        applicability: if context.visual_required {
            VisualApplicability::Required
        } else {
            VisualApplicability::NotApplicable
        },
        references: context
            .evidence
            .iter()
            .map(|evidence| TrustedEvidenceReference {
                evidence_id: evidence.id.as_str().to_string(),
                comparison_id: evidence.comparison_id.clone(),
                status: evidence.status.as_str().into(),
                manifest_digest: evidence.manifest_digest.clone(),
                baseline_png_digest: evidence.baseline_png_digest.clone(),
                render_png_digest: evidence.render_png_digest.clone(),
                pixel_diff_digest: evidence.pixel_diff_digest.clone(),
                heatmap_png_digest: evidence.heatmap_png_digest.clone(),
                geometry_diff_digest: evidence.geometry_diff_digest.clone(),
                accessibility_diff_digest: evidence.accessibility_diff_digest.clone(),
                environment_digest: evidence.environment_digest.clone(),
                changed_pixels: evidence.changed_pixels,
                total_pixels: evidence.total_pixels,
                pixel_ratio_micros: evidence.pixel_ratio_micros,
                max_geometry_delta_milli: evidence.max_geometry_delta_milli,
                accessibility_changes: evidence.accessibility_changes,
            })
            .collect(),
    }
}

fn fallback(
    reviewed_run_id: Option<&RunId>,
    code: &str,
    summary: &str,
    follow_up: &str,
    context: &ReviewTrustContext<'_>,
) -> TrustedReviewResultV2 {
    TrustedReviewResultV2 {
        schema_version: REVIEW_SCHEMA_VERSION,
        reviewed_run_id: reviewed_run_id.map(|id| id.as_str().to_string()),
        verdict: ReviewVerdict::ChangesRequested,
        summary: summary.into(),
        findings: vec![blocking_finding(
            "Aucun verdict Reviewer V2 fiable n'est disponible pour cette exécution.",
            follow_up,
        )],
        suggested_follow_ups: vec![follow_up.into()],
        diff: None,
        visual_evidence: trusted_evidence(context),
        gate: ReviewTrustGate {
            trusted_v2: false,
            approvable: false,
            code: code.into(),
            message: follow_up.into(),
        },
    }
}

fn blocking_finding(text: &str, fix: &str) -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Blocking,
        text: text.into(),
        location: "reviewer-output".into(),
        fix: Some(fix.into()),
    }
}

fn serialize(result: TrustedReviewResultV2) -> String {
    serde_json::to_string(&result).expect("the fixed reviewer V2 schema serializes")
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
    use latoile_core::ids::VisualComparisonId;

    fn evidence(status: VisualComparisonStatus) -> VisualComparison {
        let complete = status != VisualComparisonStatus::Invalid;
        VisualComparison {
            id: VisualComparisonId::new("visual:executor:home").unwrap(),
            spec_version_id: SpecVersionId::new("spec-2").unwrap(),
            project_id: ProjectId::new("project-1").unwrap(),
            run_id: RunId::new("executor-1").unwrap(),
            comparison_id: "home-default".into(),
            manifest_digest: "a".repeat(64),
            package_commit_sha: "1".repeat(40),
            baseline_png_digest: "b".repeat(64),
            status,
            changed_pixels: u64::from(complete),
            total_pixels: if complete { 100 } else { 0 },
            pixel_ratio_micros: if complete { 10_000 } else { 0 },
            max_geometry_delta_milli: 0,
            accessibility_changes: 0,
            render_png_digest: complete.then(|| "c".repeat(64)),
            pixel_diff_digest: complete.then(|| "d".repeat(64)),
            heatmap_png_digest: complete.then(|| "e".repeat(64)),
            geometry_diff_digest: complete.then(|| "f".repeat(64)),
            accessibility_diff_digest: complete.then(|| "1".repeat(64)),
            environment_digest: complete.then(|| "2".repeat(64)),
            browser_version: complete.then(|| "Chrome/151".into()),
            font_fingerprint: complete.then(|| "3".repeat(64)),
            failure_code: (!complete).then(|| "timeout".into()),
            failure_message: (!complete).then(|| "not ready".into()),
            recovery_action: (!complete).then(|| "fix readiness".into()),
        }
    }

    fn reference(evidence: &VisualComparison) -> serde_json::Value {
        serde_json::json!({
            "evidence_id": evidence.id.as_str(),
            "manifest_digest": evidence.manifest_digest,
            "baseline_png_digest": evidence.baseline_png_digest,
            "render_png_digest": evidence.render_png_digest,
            "pixel_diff_digest": evidence.pixel_diff_digest,
            "heatmap_png_digest": evidence.heatmap_png_digest,
            "geometry_diff_digest": evidence.geometry_diff_digest,
            "accessibility_diff_digest": evidence.accessibility_diff_digest,
            "environment_digest": evidence.environment_digest,
        })
    }

    fn output(_evidence: &VisualComparison, verdict: &str) -> String {
        let (findings, follow_ups) = if verdict == "approve" {
            (serde_json::json!([]), serde_json::json!([]))
        } else {
            (
                serde_json::json!([{
                    "severity": "reservation",
                    "text": "Écart mineur accepté.",
                    "location": "visual:home",
                }]),
                serde_json::json!(["Suivre l'écart."]),
            )
        };
        serde_json::json!({
            "schema_version": 2,
            "verdict": verdict,
            "summary": "Preuves contrôlées.",
            "findings": findings,
            "suggested_follow_ups": follow_ups,
            "visual_evidence": {
                "applicability": "required",
                "references": [],
            }
        })
        .to_string()
    }

    fn context<'a>(evidence: &'a [VisualComparison]) -> ReviewTrustContext<'a> {
        ReviewTrustContext {
            project_id: &evidence[0].project_id,
            spec_version_id: Some(&evidence[0].spec_version_id),
            reviewed_run_id: &evidence[0].run_id,
            visual_required: true,
            evidence,
        }
    }

    #[test]
    fn exact_passed_server_evidence_produces_an_approvable_v2_payload() {
        let evidence = vec![evidence(VisualComparisonStatus::Passed)];
        let payload = trusted_review_payload(&output(&evidence[0], "approve"), &context(&evidence));
        let parsed: TrustedReviewResultV2 = serde_json::from_str(&payload).unwrap();
        assert!(parsed.gate.trusted_v2);
        assert!(parsed.gate.approvable);
        assert_eq!(parsed.visual_evidence.references[0].status, "passed");
        assert!(review_payload_is_approvable(&payload));
    }

    #[test]
    fn server_side_project_spec_and_run_provenance_fail_closed() {
        let original = evidence(VisualComparisonStatus::Passed);
        for mutation in ["project", "spec", "run"] {
            let mut evidence = vec![original.clone()];
            match mutation {
                "project" => evidence[0].project_id = ProjectId::new("other-project").unwrap(),
                "spec" => evidence[0].spec_version_id = SpecVersionId::new("old-spec").unwrap(),
                "run" => evidence[0].run_id = RunId::new("other-run").unwrap(),
                _ => unreachable!(),
            }
            let project = ProjectId::new("project-1").unwrap();
            let spec = SpecVersionId::new("spec-2").unwrap();
            let run = RunId::new("executor-1").unwrap();
            let context = ReviewTrustContext {
                project_id: &project,
                spec_version_id: Some(&spec),
                reviewed_run_id: &run,
                visual_required: true,
                evidence: &evidence,
            };
            let payload = trusted_review_payload(&output(&original, "approve"), &context);
            assert!(
                !review_payload_is_approvable(&payload),
                "mutation={mutation}"
            );
        }
    }

    #[test]
    fn model_echoes_cannot_select_or_override_server_evidence() {
        let evidence = vec![evidence(VisualComparisonStatus::Passed)];
        let expected_digest = evidence[0].render_png_digest.clone();
        let mut invented = evidence[0].clone();
        invented.render_png_digest = Some("9".repeat(64));
        let mut untrusted: serde_json::Value =
            serde_json::from_str(&output(&evidence[0], "approve")).unwrap();
        untrusted["visual_evidence"]["references"] = serde_json::json!([reference(&invented)]);

        let payload = trusted_review_payload(&untrusted.to_string(), &context(&evidence));
        let trusted: TrustedReviewResultV2 = serde_json::from_str(&payload).unwrap();

        assert!(trusted.gate.trusted_v2);
        assert!(trusted.gate.approvable);
        assert_eq!(
            trusted.visual_evidence.references[0].render_png_digest,
            expected_digest
        );
        assert!(review_payload_is_approvable(&payload));
    }

    #[test]
    fn ignored_legacy_echoes_remain_size_bounded() {
        let evidence = vec![evidence(VisualComparisonStatus::Passed)];
        let mut oversized = reference(&evidence[0]);
        oversized["evidence_id"] = serde_json::Value::String("x".repeat(257));
        let mut untrusted: serde_json::Value =
            serde_json::from_str(&output(&evidence[0], "approve")).unwrap();
        untrusted["visual_evidence"]["references"] = serde_json::json!([oversized]);

        let payload = trusted_review_payload(&untrusted.to_string(), &context(&evidence));
        let trusted: TrustedReviewResultV2 = serde_json::from_str(&payload).unwrap();

        assert_eq!(trusted.gate.code, "invalid_reviewer_output");
        assert!(!review_payload_is_approvable(&payload));
    }

    #[test]
    fn missing_invalid_and_blocking_visual_evidence_are_not_approvable() {
        let project = ProjectId::new("project-1").unwrap();
        let spec = SpecVersionId::new("spec-2").unwrap();
        let run = RunId::new("executor-1").unwrap();
        let missing = ReviewTrustContext {
            project_id: &project,
            spec_version_id: Some(&spec),
            reviewed_run_id: &run,
            visual_required: true,
            evidence: &[],
        };
        let missing_output = serde_json::json!({
            "schema_version": 2,
            "verdict": "approve",
            "summary": "OK",
            "visual_evidence": {"applicability": "required", "references": []}
        })
        .to_string();
        assert!(!review_payload_is_approvable(&trusted_review_payload(
            &missing_output,
            &missing
        )));

        for status in [
            VisualComparisonStatus::Invalid,
            VisualComparisonStatus::Blocking,
        ] {
            let evidence = vec![evidence(status)];
            let payload =
                trusted_review_payload(&output(&evidence[0], "approve"), &context(&evidence));
            assert!(!review_payload_is_approvable(&payload));
        }
    }

    #[test]
    fn non_visual_review_requires_an_explicit_not_applicable_decision() {
        let project = ProjectId::new("project-1").unwrap();
        let spec = SpecVersionId::new("spec-2").unwrap();
        let run = RunId::new("executor-1").unwrap();
        let context = ReviewTrustContext {
            project_id: &project,
            spec_version_id: Some(&spec),
            reviewed_run_id: &run,
            visual_required: false,
            evidence: &[],
        };
        let output = serde_json::json!({
            "schema_version": 2,
            "verdict": "approve",
            "summary": "Code et tests conformes.",
            "visual_evidence": {"applicability": "not_applicable", "references": []}
        })
        .to_string();
        assert!(review_payload_is_approvable(&trusted_review_payload(
            &output, &context
        )));
    }

    #[test]
    fn malformed_failed_and_legacy_v1_outputs_are_preserved_but_untrusted() {
        let evidence = vec![evidence(VisualComparisonStatus::Passed)];
        for output in [
            "not json",
            r#"{"schema_version":1,"verdict":"approve","summary":"legacy"}"#,
            r#"{"schema_version":2,"verdict":"approve","summary":"missing evidence"}"#,
        ] {
            let payload = trusted_review_payload(output, &context(&evidence));
            let parsed: TrustedReviewResultV2 = serde_json::from_str(&payload).unwrap();
            assert_eq!(parsed.verdict, ReviewVerdict::ChangesRequested);
            assert_eq!(parsed.gate.code, "invalid_reviewer_output");
            assert!(!review_payload_is_approvable(&payload));
        }

        let legacy = serde_json::json!({
            "schema_version": LEGACY_REVIEW_SCHEMA_VERSION,
            "verdict": "approve",
            "summary": "Ancien résultat",
            "findings": [],
            "suggested_follow_ups": []
        })
        .to_string();
        let parsed: ReviewResultV1 = serde_json::from_str(&legacy).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert!(!review_payload_is_approvable(&legacy));

        let failed = review_failure_payload(&"é".repeat(1000));
        assert!(!review_payload_is_approvable(&failed));
        assert!(failed.len() < 3_000);
    }
}
