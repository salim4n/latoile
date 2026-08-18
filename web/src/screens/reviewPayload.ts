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

export interface VerdictPayload {
  schema_version?: number;
  verdict?: string;
  summary?: string;
  findings: Finding[];
  suggested_follow_ups: string[];
  diff?: DiffPayload;
  comparison?: ComparisonPayload;
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
    };
  } catch {
    return { findings: [], suggested_follow_ups: [] };
  }
}
