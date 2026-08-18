// Runtime validation for reviewer output. The Reviewer is an external agent:
// malformed or legacy JSON must degrade to a summary-only review, never crash
// the human decision screen.

export interface Finding {
  severity?: "blocking" | "reservation";
  text: string;
  location?: string;
}

export interface DiffPayload {
  file: string;
  additions: number;
  deletions: number;
  lines: string[];
}

export interface ReviewFramePayload {
  title?: string;
  subtitle?: string;
  fields: string[];
  cta?: string;
}

export interface ComparisonPayload {
  spec_version: string;
  target: ReviewFramePayload;
  render: ReviewFramePayload;
  expected_spacing_px?: number;
  actual_spacing_px?: number;
  gap?: string;
}

export interface TrustedEvidenceReference {
  evidence_id: string;
  comparison_id: string;
  status: "invalid" | "blocking" | "reservation" | "passed";
  manifest_digest: string;
  baseline_png_digest: string;
  render_png_digest?: string;
  pixel_diff_digest?: string;
  heatmap_png_digest?: string;
  geometry_diff_digest?: string;
  accessibility_diff_digest?: string;
  environment_digest?: string;
  changed_pixels: number;
  total_pixels: number;
  pixel_ratio_micros: number;
  max_geometry_delta_milli: number;
  accessibility_changes: number;
}

export interface TrustedVisualEvidence {
  applicability: "required" | "not_applicable";
  references: TrustedEvidenceReference[];
}

export interface ReviewGate {
  trusted_v2: boolean;
  approvable: boolean;
  code: string;
  message: string;
}

export interface VerdictPayload {
  schema_version?: number;
  verdict?: string;
  summary?: string;
  findings: Finding[];
  suggested_follow_ups: string[];
  diff?: DiffPayload;
  comparison?: ComparisonPayload;
  reviewed_run_id?: string;
  visual_evidence?: TrustedVisualEvidence;
  gate?: ReviewGate;
}

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : null;
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function finiteNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function parseEvidenceReference(value: unknown): TrustedEvidenceReference | undefined {
  const reference = record(value);
  if (!reference) return undefined;
  const evidenceId = text(reference.evidence_id);
  const comparisonId = text(reference.comparison_id);
  const manifestDigest = text(reference.manifest_digest);
  const baselineDigest = text(reference.baseline_png_digest);
  const status = reference.status;
  if (
    !evidenceId || !comparisonId || !manifestDigest || !baselineDigest ||
    !["invalid", "blocking", "reservation", "passed"].includes(String(status))
  ) return undefined;
  return {
    evidence_id: evidenceId,
    comparison_id: comparisonId,
    status: status as TrustedEvidenceReference["status"],
    manifest_digest: manifestDigest,
    baseline_png_digest: baselineDigest,
    render_png_digest: text(reference.render_png_digest),
    pixel_diff_digest: text(reference.pixel_diff_digest),
    heatmap_png_digest: text(reference.heatmap_png_digest),
    geometry_diff_digest: text(reference.geometry_diff_digest),
    accessibility_diff_digest: text(reference.accessibility_diff_digest),
    environment_digest: text(reference.environment_digest),
    changed_pixels: finiteNumber(reference.changed_pixels) ?? 0,
    total_pixels: finiteNumber(reference.total_pixels) ?? 0,
    pixel_ratio_micros: finiteNumber(reference.pixel_ratio_micros) ?? 0,
    max_geometry_delta_milli: finiteNumber(reference.max_geometry_delta_milli) ?? 0,
    accessibility_changes: finiteNumber(reference.accessibility_changes) ?? 0,
  };
}

function parseFrame(value: unknown): ReviewFramePayload | undefined {
  const frame = record(value);
  if (!frame) return undefined;
  const fields = Array.isArray(frame.fields)
    ? frame.fields.filter((field): field is string => typeof field === "string")
    : [];
  return {
    title: text(frame.title),
    subtitle: text(frame.subtitle),
    fields,
    cta: text(frame.cta),
  };
}

export function parseReviewPayload(raw: string): VerdictPayload {
  try {
    const payload = record(JSON.parse(raw));
    if (!payload) return { findings: [], suggested_follow_ups: [] };

    const findings: Finding[] = Array.isArray(payload.findings)
      ? payload.findings.flatMap((item) => {
          const finding = record(item);
          const findingText = finding && text(finding.text);
          if (!finding || !findingText) return [];
          const severity =
            finding.severity === "blocking" || finding.severity === "reservation"
              ? finding.severity
              : undefined;
          return [{
            text: findingText,
            severity,
            location: text(finding.location) ?? text(finding.loc),
          }];
        })
      : [];

    const rawDiff = record(payload.diff);
    const diffLines = rawDiff && Array.isArray(rawDiff.lines)
      ? rawDiff.lines.filter((line): line is string => typeof line === "string")
      : [];
    const file = rawDiff && text(rawDiff.file);
    const diff = rawDiff && file && diffLines.length > 0
      ? {
          file,
          additions: finiteNumber(rawDiff.additions) ?? 0,
          deletions: finiteNumber(rawDiff.deletions) ?? 0,
          lines: diffLines,
        }
      : undefined;

    const rawComparison = record(payload.comparison);
    const target = rawComparison && parseFrame(rawComparison.target);
    const render = rawComparison && parseFrame(rawComparison.render);
    const specVersion = rawComparison?.spec_version;
    const comparison = rawComparison && target && render &&
      (typeof specVersion === "string" || typeof specVersion === "number")
      ? {
          spec_version: String(specVersion),
          target,
          render,
          expected_spacing_px: finiteNumber(rawComparison.expected_spacing_px),
          actual_spacing_px: finiteNumber(rawComparison.actual_spacing_px),
          gap: text(rawComparison.gap),
        }
      : undefined;

    const rawEvidence = record(payload.visual_evidence);
    const applicability = rawEvidence?.applicability;
    const evidenceReferences = rawEvidence && Array.isArray(rawEvidence.references)
      ? rawEvidence.references.flatMap((item) => {
          const parsed = parseEvidenceReference(item);
          return parsed ? [parsed] : [];
        })
      : [];
    const visualEvidence: TrustedVisualEvidence | undefined =
      applicability === "required" || applicability === "not_applicable"
      ? { applicability, references: evidenceReferences }
      : undefined;

    const rawGate = record(payload.gate);
    const gateCode = rawGate && text(rawGate.code);
    const gateMessage = rawGate && text(rawGate.message);
    const gate = rawGate && gateCode && gateMessage &&
      typeof rawGate.trusted_v2 === "boolean" && typeof rawGate.approvable === "boolean"
      ? {
          trusted_v2: rawGate.trusted_v2,
          approvable: rawGate.approvable,
          code: gateCode,
          message: gateMessage,
        }
      : undefined;

    return {
      schema_version: finiteNumber(payload.schema_version),
      verdict: text(payload.verdict),
      summary: text(payload.summary),
      findings,
      suggested_follow_ups: Array.isArray(payload.suggested_follow_ups)
        ? payload.suggested_follow_ups.filter(
            (item): item is string => typeof item === "string" && item.trim().length > 0,
          )
        : [],
      diff,
      comparison,
      reviewed_run_id: text(payload.reviewed_run_id),
      visual_evidence: visualEvidence,
      gate,
    };
  } catch {
    return { findings: [], suggested_follow_ups: [] };
  }
}
