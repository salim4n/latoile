// LaToile's human signature point. The visual blocks and the untrusted agent
// payload parser are split out so this screen keeps the decision flow obvious.

import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api, ApiError, type Approval } from "../api";
import { useAsync } from "../hooks";
import { useT } from "../i18n";
import { Shell } from "../components/Shell";
import { EmptyState, ErrorState } from "../components/states";
import {
  CompareBlock,
  DiffBlock,
  ReviewLoading,
  runLabel,
  type ReviewDecision,
  VerdictCard,
} from "./ReviewBlocks";
import { parseReviewPayload } from "./reviewPayload";
import { VisualEvidencePanel } from "./VisualEvidencePanel";

export function ReviewScreen() {
  const { t } = useT();
  const { approvalId = "" } = useParams();
  const review = useAsync(() => api.approval(approvalId), [approvalId]);
  const [decidedApproval, setDecidedApproval] = useState<Approval | null>(null);
  const [decision, setDecision] = useState<ReviewDecision>(null);
  const [comment, setComment] = useState("");
  const [busy, setBusy] = useState(false);
  const [decisionError, setDecisionError] = useState(false);

  const approval = decidedApproval ?? review.data;
  const persistedDecision: ReviewDecision = approval?.status === "granted"
    ? "approved"
    : approval?.status === "rejected"
      ? "rejected"
      : null;
  const effectiveDecision = decision ?? persistedDecision;
  const payload = approval ? parseReviewPayload(approval.payload) : null;
  const screenTitle = approval?.task_title
    ? `${t("review.screen.prefix")}${approval.task_title}`
    : t("review.title");
  const crumb = approval
    ? `${approval.project_name ?? "LaToile"} / Reviews / run ${runLabel(approval.run_id)}`
    : t("review.title");

  async function decide(granted: boolean) {
    if (busy || !approval) return;
    if (granted && !canApprove) return;
    if (!granted && !comment.trim()) return;
    setBusy(true);
    setDecisionError(false);
    try {
      const decided = await api.decide(
        approvalId,
        granted,
        comment.trim() || undefined,
      );
      setDecidedApproval({ ...approval, ...decided });
      setDecision(granted ? "approved" : "rejected");
    } catch {
      setDecisionError(true);
    } finally {
      setBusy(false);
    }
  }

  const decided = effectiveDecision !== null;
  const canApprove = payload?.schema_version === 2 &&
    payload.gate?.trusted_v2 === true && payload.gate.approvable === true;
  const notFound = review.error instanceof ApiError && review.error.status === 404;

  return (
    <Shell back={t("shell.back.inbox")} title={screenTitle} crumb={crumb} wide>
      {review.loading && <ReviewLoading label={t("review.loading")} />}
      {review.error && !notFound && (
        <ErrorState
          title={t("review.error.title")}
          body={t("review.error.body")}
          onRetry={review.reload}
        />
      )}
      {notFound && (
        <EmptyState
          title={t("review.gone.title")}
          body={t("review.gone")}
          action={<Link className="btn btn--primary" to="/">{t("review.back")}</Link>}
        />
      )}
      {approval && payload && (
        <section
          className="review-state"
          aria-label={decided ? t("review.state.decided") : t("review.state.pending")}
        >
          <span className="state-label">
            {t(effectiveDecision === "approved"
              ? "review.state.approved"
              : effectiveDecision === "rejected"
                ? "review.state.rejected"
                : "review.state.pending")}
          </span>
          <VerdictCard approval={approval} payload={payload} decision={effectiveDecision} />
          {payload.gate && (
            <section
              className={canApprove ? "review-gate review-gate--open" : "review-gate review-gate--closed"}
              aria-label={t("review.gate.title")}
            >
              <div>
                <strong>{canApprove ? t("review.gate.open") : t("review.gate.closed")}</strong>
                <span>{payload.gate.message}</span>
              </div>
              <code>{payload.gate.code}</code>
            </section>
          )}
          {!payload.gate && (
            <section className="review-gate review-gate--closed" aria-label={t("review.gate.title")}>
              <div>
                <strong>{t("review.gate.legacy")}</strong>
                <span>{t("review.gate.legacy.body")}</span>
              </div>
              <code>legacy_untrusted</code>
            </section>
          )}
          {payload.diff && <DiffBlock diff={payload.diff} runId={approval.run_id} />}
          {payload.visual_evidence && <VisualEvidencePanel payload={payload} />}
          {!payload.visual_evidence && payload.comparison && (
            <CompareBlock comparison={payload.comparison} runId={approval.run_id} />
          )}
          {!payload.diff && !payload.comparison && !payload.visual_evidence && (
            <p className="review-details-note">{t("review.details.missing")}</p>
          )}

          {decided && (approval.decision_comment || approval.corrective_run_id) && (
            <section className="decision-history" aria-label={t("review.history")}>
              <h2>{t("review.history")}</h2>
              {approval.decision_comment && (
                <p><strong>{t("review.comment.saved")}</strong> {approval.decision_comment}</p>
              )}
              {approval.corrective_run_id && (
                <p>{t("review.correction.started")} #{approval.corrective_run_id}</p>
              )}
            </section>
          )}

          <div className="actionbar">
            {!decided && (
              <div className="review-comment">
                <label htmlFor="review-comment">{t("review.comment.label")}</label>
                <textarea
                  id="review-comment"
                  rows={3}
                  value={comment}
                  onChange={(event) => setComment(event.target.value)}
                  placeholder={t("review.comment.placeholder")}
                  disabled={busy}
                />
                <span>{t("review.comment.help")}</span>
              </div>
            )}
            {decided && (
              <p className="merged-note">
                {effectiveDecision === "approved"
                  ? `${t("review.run")} ${runLabel(approval.run_id)} ${t("review.decided.approved")}`
                  : t("review.decided.rejected")}
              </p>
            )}
            <button
              className="btn btn--primary btn--block"
              type="button"
              disabled={busy || decided || !canApprove}
              aria-describedby={!canApprove && !decided ? "review-approval-blocked" : undefined}
              onClick={() => decide(true)}
            >
              {busy && <span className="spin" aria-hidden="true" />}
              {busy ? t("review.deciding") : t("review.approve")}
            </button>
            {!canApprove && !decided && (
              <p id="review-approval-blocked" className="approval-blocked-note">
                {payload.gate?.message ?? t("review.gate.legacy.body")}
              </p>
            )}
            <div className="divider" aria-hidden="true" />
            <button
              className="btn btn--danger btn--block"
              type="button"
              disabled={busy || decided || !comment.trim()}
              onClick={() => decide(false)}
            >
              {t("review.changes")}
            </button>
          </div>
          {decisionError && (
            <p className="review-decision-error" role="alert">
              {t("review.decision.error")}
            </p>
          )}
        </section>
      )}
    </Shell>
  );
}
