// The review screen (P0 signature): the verdict card and the sticky
// Approve / Request-changes bar. The mockup's diff and mockup-vs-render
// frames need reviewer output that only the orchestrator pass produces —
// V1 renders them when the payload carries them, and says so when it
// doesn't.

import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync } from "../hooks";
import { useT } from "../i18n";
import { Shell } from "../components/Shell";
import { Skeletons } from "../components/states";

type Decision = "approved" | "rejected" | null;

interface VerdictPayload {
  summary?: string;
  findings?: { text: string; loc?: string }[];
}

function parsePayload(raw: string): VerdictPayload {
  try {
    const parsed = JSON.parse(raw);
    return typeof parsed === "object" && parsed !== null ? parsed : {};
  } catch {
    return {};
  }
}

export function ReviewScreen() {
  const { t } = useT();
  const { approvalId = "" } = useParams();
  // The port exposes the pending list only; a decided approval is gone from
  // it, which is exactly how the "already decided" state is detected.
  const approvals = useAsync(api.approvals, []);
  const [decision, setDecision] = useState<Decision>(null);
  const [busy, setBusy] = useState(false);

  const approval = (approvals.data ?? []).find((a) => a.id === approvalId);
  const payload = approval ? parsePayload(approval.payload) : {};

  async function decide(granted: boolean) {
    if (busy) return;
    setBusy(true);
    try {
      await api.decide(approvalId, granted);
      setDecision(granted ? "approved" : "rejected");
    } finally {
      setBusy(false);
    }
  }

  const decided = decision !== null;
  const badge = decided ? (
    decision === "approved" ? (
      <span className="badge badge--success">{t("review.badge.approved")}</span>
    ) : (
      <span className="badge badge--danger">{t("review.badge.rejected")}</span>
    )
  ) : (
    <span className="badge badge--warning">{t("review.badge.pending")}</span>
  );
  const verdictClass = decided
    ? decision === "approved"
      ? "verdict verdict--ok"
      : "verdict verdict--ko"
    : "verdict";

  return (
    <Shell back={t("shell.back.inbox")} title={t("review.title")}>
      {approvals.loading && <Skeletons />}

      {approvals.data && (
        <>
          <div className={verdictClass}>
            <div className="verdict-head">
              {badge}
              <span className="badge badge--neutral">
                Reviewer · run {approval?.run_id ?? "?"}
              </span>
            </div>
            <h2>{t("review.title")}</h2>
            <p className="summary">{payload.summary ?? t("review.no.details")}</p>
            {payload.findings && payload.findings.length > 0 && (
              <div className="findings">
                <h3>Findings ({payload.findings.length})</h3>
                {payload.findings.map((finding, i) => (
                  <div className="finding" key={i}>
                    <span className="sev" aria-hidden="true" />
                    <div>
                      {finding.text}
                      {finding.loc && <span className="loc">{finding.loc}</span>}
                    </div>
                  </div>
                ))}
              </div>
            )}
            {!approval && (
              <p className="summary" style={{ marginTop: "var(--space-2)" }}>
                {t("review.gone")}
              </p>
            )}
          </div>

          <div className="actionbar">
            {decided && (
              <p className="merged-note">
                {t(
                  decision === "approved"
                    ? "review.decided.approved"
                    : "review.decided.rejected",
                )}
              </p>
            )}
            {!decided && approval && (
              <>
                <button
                  className="btn btn--primary btn--block"
                  type="button"
                  disabled={busy}
                  onClick={() => decide(true)}
                >
                  {t("review.approve")}
                </button>
                <div className="divider" aria-hidden="true" />
                <button
                  className="btn btn--danger btn--block"
                  type="button"
                  disabled={busy}
                  onClick={() => decide(false)}
                >
                  {t("review.changes")}
                </button>
              </>
            )}
            {(decided || !approval) && (
              <Link className="btn btn--ghost btn--block" to="/">
                {t("review.back")}
              </Link>
            )}
          </div>
        </>
      )}
    </Shell>
  );
}
