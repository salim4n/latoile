// Inbox — « qu'est-ce qui attend MA décision ? » in under 3 seconds:
// pending approvals, blocked runs (permission approvals), active projects.
// Refreshes itself on approval/run events (D10).

import { Link } from "react-router-dom";
import { api, type Approval } from "../api";
import { useAsync, useEventReload } from "../hooks";
import { useT, type Key } from "../i18n";
import { Shell } from "../components/Shell";
import { EmptyState, ErrorState, Skeletons } from "../components/states";

function statusBadge(status: string, t: (k: Key) => string) {
  switch (status) {
    case "building":
      return <span className="badge badge--accent">{t("status.building")}</span>;
    case "live":
      return <span className="badge badge--success">{t("status.live")}</span>;
    case "specced":
      return <span className="badge badge--neutral">{t("status.specced")}</span>;
    default:
      return <span className="badge badge--neutral">{t("status.draft")}</span>;
  }
}

function ReviewCard({ approval }: { approval: Approval }) {
  const { t } = useT();
  return (
    <article className="card item">
      <div className="item-head">
        <span className="badge badge--warning">{t("inbox.review.badge")}</span>
      </div>
      <h3 className="item-title">
        {t("review.title")} · run {approval.run_id}
      </h3>
      <Link className="item-link" to={`/reviews/${approval.id}`}>
        {t("inbox.review.link")}
      </Link>
    </article>
  );
}

function PermissionCard({
  approval,
  onDecided,
}: {
  approval: Approval;
  onDecided: () => void;
}) {
  const { t } = useT();
  async function decide(granted: boolean) {
    await api.decide(approval.id, granted);
    onDecided();
  }
  return (
    <article className="card item">
      <div className="item-head">
        <span className="badge badge--warning">{t("inbox.permission.badge")}</span>
      </div>
      <h3 className="item-title">
        {t("inbox.permission.badge")} · run {approval.run_id}
      </h3>
      <code className="cmd">{approval.payload}</code>
      <div className="item-actions">
        <button className="btn btn--primary btn--sm" type="button" onClick={() => decide(true)}>
          {t("inbox.permission.allow")}
        </button>
        <button className="btn btn--ghost btn--sm" type="button" onClick={() => decide(false)}>
          {t("inbox.permission.deny")}
        </button>
      </div>
    </article>
  );
}

export function InboxScreen() {
  const { t } = useT();
  const approvals = useAsync(api.approvals, []);
  const projects = useAsync(api.projects, []);
  const auth = useAsync(api.agentAuthStatusAll, []);
  useEventReload(
    ["approval_requested", "approval_granted", "approval_rejected", "run_blocked"],
    approvals.reload,
  );
  // Anything happening anywhere can change this screen.
  useEventReload(
    ["task_ready", "run_started", "run_finished", "message_posted", "preview_ready"],
    projects.reload,
  );

  const loading = approvals.loading || projects.loading;
  const failed = approvals.error || projects.error;
  const list = approvals.data ?? [];
  const reviews = list.filter((a) => a.kind === "review");
  const permissions = list.filter((a) => a.kind === "permission");
  const active = projects.data ?? [];
  const allClear = !loading && list.length === 0 && active.length === 0;

  return (
    <Shell title="Inbox">
      {auth.data && !auth.data.claude.authenticated && !auth.data.codex.authenticated && (
        <Link
          className="card row"
          to="/settings"
          style={{ marginBottom: "var(--space-4)", borderColor: "rgba(245, 176, 66, 0.4)" }}
        >
          <div className="row-main">
            <p>{t("inbox.auth.banner")}</p>
          </div>
          <span className="badge badge--warning">{t("inbox.auth.cta")}</span>
        </Link>
      )}
      {loading && <Skeletons />}
      {failed && (
        <ErrorState
          title={t("inbox.error.title")}
          body={t("inbox.error.body")}
          onRetry={() => {
            approvals.reload();
            projects.reload();
          }}
        />
      )}
      {!loading && !failed && allClear && (
        <EmptyState
          title={t("inbox.empty.title")}
          body={t("inbox.empty.body")}
          action={
            <Link className="btn btn--ghost" to="/projects">
              {t("inbox.empty.cta")}
            </Link>
          }
        />
      )}
      {!loading && !failed && !allClear && (
        <>
          {reviews.length > 0 && (
            <div className="sec">
              <h2 className="sec-title">
                {t("inbox.approvals")} <span className="count">({reviews.length})</span>
              </h2>
              {reviews.map((a) => (
                <ReviewCard key={a.id} approval={a} />
              ))}
            </div>
          )}
          {permissions.length > 0 && (
            <div className="sec">
              <h2 className="sec-title">
                {t("inbox.blocked")} <span className="count">({permissions.length})</span>
              </h2>
              {permissions.map((a) => (
                <PermissionCard key={a.id} approval={a} onDecided={approvals.reload} />
              ))}
            </div>
          )}
          {active.length > 0 && (
            <div className="sec">
              <h2 className="sec-title">{t("inbox.projects")}</h2>
              {active.map((p) => (
                <Link className="card row" to={`/projects/${p.id}`} key={p.id}>
                  <div className="row-main">
                    <h3>{p.name}</h3>
                    <p>{p.github_repo}</p>
                  </div>
                  {statusBadge(p.status, t)}
                </Link>
              ))}
            </div>
          )}
        </>
      )}
    </Shell>
  );
}
