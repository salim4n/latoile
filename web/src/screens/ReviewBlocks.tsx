import type { Approval } from "../api";
import { useT } from "../i18n";
import { WarningIcon } from "../components/icons";
import type {
  ComparisonPayload,
  DiffPayload,
  ReviewFramePayload,
  VerdictPayload,
} from "./reviewPayload";

export type ReviewDecision = "approved" | "rejected" | null;

function relativeTime(iso: string | undefined, lang: "fr" | "en") {
  if (!iso) return null;
  const timestamp = Date.parse(iso);
  if (!Number.isFinite(timestamp)) return null;
  const seconds = Math.round((timestamp - Date.now()) / 1000);
  const formatter = new Intl.RelativeTimeFormat(lang, { numeric: "auto" });
  if (Math.abs(seconds) < 60) return formatter.format(seconds, "second");
  const minutes = Math.round(seconds / 60);
  if (Math.abs(minutes) < 60) return formatter.format(minutes, "minute");
  const hours = Math.round(minutes / 60);
  if (Math.abs(hours) < 24) return formatter.format(hours, "hour");
  return formatter.format(Math.round(hours / 24), "day");
}

export function runLabel(runId: string | undefined) {
  return `#${runId ?? "?"}`;
}

export function ReviewLoading({ label }: { label: string }) {
  return (
    <div className="review-loading" role="status" aria-label={label} aria-busy="true">
      <div className="skel review-state-skeleton" />
      {[124, 230, 360].map((height) => (
        <div
          className="skel review-block-skeleton"
          data-testid="review-block-skeleton"
          aria-hidden="true"
          key={height}
        >
          <span className="skel review-block-line" />
        </div>
      ))}
    </div>
  );
}

export function VerdictCard({
  approval,
  payload,
  decision,
}: {
  approval: Approval;
  payload: VerdictPayload;
  decision: ReviewDecision;
}) {
  const { t, lang } = useT();
  const time = relativeTime(approval.created_at, lang);
  const recommendedChanges = payload.verdict === "changes_requested";
  const badge = decision === "approved"
    ? { className: "badge badge--success", label: t("review.badge.approved") }
    : decision === "rejected"
      ? { className: "badge badge--danger", label: t("review.badge.rejected") }
      : recommendedChanges
        ? { className: "badge badge--danger", label: t("review.badge.rejected") }
        : payload.verdict === "approve_with_reservations"
          ? { className: "badge badge--warning", label: t("review.badge.reserve") }
          : { className: "badge badge--warning", label: t("review.badge.pending") };
  const verdictClass = decision === "approved"
    ? "verdict verdict--ok"
    : decision === "rejected" || recommendedChanges
      ? "verdict verdict--ko"
      : "verdict";

  return (
    <div className={verdictClass}>
      <div className="verdict-head">
        <span className={badge.className}>{badge.label}</span>
        <span className="badge badge--neutral review-meta">
          Reviewer · run {runLabel(approval.run_id)}
          {time && (
            <>
              {" · "}
              <time dateTime={approval.created_at}>{time}</time>
            </>
          )}
        </span>
      </div>
      <h2>{t("review.title")}</h2>
      <p className="summary">{payload.summary ?? t("review.no.details")}</p>
      {payload.findings.length > 0 && (
        <div className="findings">
          <h3>{t("review.findings")} ({payload.findings.length})</h3>
          {payload.findings.map((finding, index) => (
            <div
              className={`finding finding--${finding.severity ?? "reservation"}`}
              key={`${finding.location ?? "finding"}-${index}`}
            >
              <span className="sev" aria-hidden="true" />
              <div>
                {finding.text}
                {finding.location && <span className="loc">{finding.location}</span>}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function DiffBlock({ diff, runId }: { diff: DiffPayload; runId: string }) {
  const { t } = useT();
  return (
    <section>
      <h2 className="sec-title">{t("review.diff")} — run {runLabel(runId)}</h2>
      <div className="diff" role="region" aria-label={`${t("review.diff.aria")} ${diff.file}`}>
        <div className="diff-head">
          <span className="file">{diff.file}</span>
          <span className="counts">
            <span className="add-n">+{diff.additions}</span>{" "}
            <span className="del-n">−{diff.deletions}</span>
          </span>
        </div>
        <pre>
          {diff.lines.map((line, index) => {
            const prefix = line.startsWith("+") ? "+" : line.startsWith("-") ? "−" : " ";
            const className = prefix === "+" ? "l add" : prefix === "−" ? "l del" : "l";
            const content = line.startsWith("+") || line.startsWith("-")
              ? line.slice(1)
              : line.slice(1) || line;
            return (
              <span className={className} key={`${index}-${line}`}>
                <span className="g">{prefix}</span>{content}
              </span>
            );
          })}
        </pre>
      </div>
    </section>
  );
}

function ReviewFrame({
  label,
  frame,
  mismatch,
}: {
  label: string;
  frame: ReviewFramePayload;
  mismatch?: boolean;
}) {
  return (
    <figure className="review-frame" aria-label={label}>
      <figcaption className="frame-cap">{label}</figcaption>
      <div className={mismatch ? "review-phone review-phone--gap" : "review-phone"}>
        <div className="review-phone-screen">
          <div className="review-app-body">
            {frame.title && <h5>{frame.title}</h5>}
            {frame.subtitle && <p className="review-app-sub">{frame.subtitle}</p>}
            {frame.fields.map((field, index) => (
              <div className="review-app-field" key={`${field}-${index}`}>{field}</div>
            ))}
            {frame.cta && (
              <div className={mismatch
                ? "review-app-cta review-app-cta--render"
                : "review-app-cta review-app-cta--spec"}
              >
                {frame.cta}
              </div>
            )}
          </div>
        </div>
      </div>
    </figure>
  );
}

export function CompareBlock({
  comparison,
  runId,
}: {
  comparison: ComparisonPayload;
  runId: string;
}) {
  const { t } = useT();
  const hasMeasurement = comparison.actual_spacing_px !== undefined &&
    comparison.expected_spacing_px !== undefined;
  return (
    <section>
      <h2 className="sec-title">{t("review.compare")}</h2>
      <div className="compare">
        <ReviewFrame
          label={`${t("review.compare.spec")} (spec v${comparison.spec_version})`}
          frame={comparison.target}
        />
        <ReviewFrame
          label={`${t("review.compare.render")} (run ${runLabel(runId)})`}
          frame={comparison.render}
          mismatch
        />
      </div>
      {(comparison.gap || hasMeasurement) && (
        <div className="gap-note">
          <WarningIcon size={18} />
          <div>
            {comparison.gap && <strong>{comparison.gap}</strong>}
            {hasMeasurement && (
              <span>
                {comparison.gap ? " " : ""}
                {comparison.actual_spacing_px} px {t("review.compare.measured")} (run {runLabel(runId)}) {t("review.compare.against")} {comparison.expected_spacing_px} px (spec v{comparison.spec_version}).
              </span>
            )}
          </div>
        </div>
      )}
    </section>
  );
}
