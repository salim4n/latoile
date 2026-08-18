// LaToile's human signature point. The visual blocks and the untrusted agent
// payload parser are split out so this screen keeps the decision flow obvious.

import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api } from "../api";
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

export function ReviewScreen() {
  const { t } = useT();
  const { approvalId = "" } = useParams();
  const approvals = useAsync(api.approvals, []);
  const [decision, setDecision] = useState<ReviewDecision>(null);
  const [busy, setBusy] = useState(false);
  const [decisionError, setDecisionError] = useState(false);

  const approval = (approvals.data ?? []).find((item) => item.id === approvalId);
  const payload = approval ? parseReviewPayload(approval.payload) : null;
  const screenTitle = approval?.task_title
    ? `${t("review.screen.prefix")}${approval.task_title}`
    : t("review.title");
  const crumb = approval
    ? `${approval.project_name ?? "LaToile"} / Reviews / run ${runLabel(approval.run_id)}`
    : t("review.title");

  async function decide(granted: boolean) {
    if (busy || !approval) return;
    setBusy(true);
    setDecisionError(false);
    try {
      await api.decide(approvalId, granted);
      setDecision(granted ? "approved" : "rejected");
    } catch {
      setDecisionError(true);
    } finally {
      setBusy(false);
    }
  }

  const decided = decision !== null;

  return (
    <Shell back={t("shell.back.inbox")} title={screenTitle} crumb={crumb} wide>
      {approvals.loading && <ReviewLoading label={t("review.loading")} />}
      {approvals.error && (
        <ErrorState
          title={t("review.error.title")}
          body={t("review.error.body")}
          onRetry={approvals.reload}
        />
      )}
      {approvals.data && !approval && (
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
            {t(decision === "approved"
              ? "review.state.approved"
              : decision === "rejected"
                ? "review.state.rejected"
                : "review.state.pending")}
          </span>
          <VerdictCard approval={approval} payload={payload} decision={decision} />
          {payload.diff && <DiffBlock diff={payload.diff} runId={approval.run_id} />}
          {payload.comparison && (
            <CompareBlock comparison={payload.comparison} runId={approval.run_id} />
          )}
          {!payload.diff && !payload.comparison && (
            <p className="review-details-note">{t("review.details.missing")}</p>
          )}

          <div className="actionbar">
            {decided && (
              <p className="merged-note">
                {decision === "approved"
                  ? `${t("review.run")} ${runLabel(approval.run_id)} ${t("review.decided.approved")}`
                  : t("review.decided.rejected")}
              </p>
            )}
            <button
              className="btn btn--primary btn--block"
              type="button"
              disabled={busy || decided}
              onClick={() => decide(true)}
            >
              {busy && <span className="spin" aria-hidden="true" />}
              {busy ? t("review.deciding") : t("review.approve")}
            </button>
            <div className="divider" aria-hidden="true" />
            <button
              className="btn btn--danger btn--block"
              type="button"
              disabled={busy || decided}
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
