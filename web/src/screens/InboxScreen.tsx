// Inbox — « qu'est-ce qui attend MA décision ? » in under 3 seconds:
// pending approvals, blocked runs (permission approvals), active projects.
// Refreshes itself on approval/run events (D10).

import { useState } from "react";
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

interface InboxPayload {
  title?: string;
  summary?: string;
  verdict?: string;
  command?: string;
}

function parsePayload(raw: string): InboxPayload {
  try {
    const parsed: unknown = JSON.parse(raw);
    return typeof parsed === "object" && parsed !== null ? parsed : {};
  } catch {
    // Early permission requests used the exact command as a plain string.
    return { command: raw };
  }
}

function roleLabel(role?: string) {
  if (!role) return "Agent";
  return role.charAt(0).toUpperCase() + role.slice(1);
}

function contextLine(approval: Approval) {
  const parts = [
    approval.project_name,
    roleLabel(approval.role_id),
    `run ${approval.run_id}`,
  ];
  return parts.filter(Boolean).join(" · ");
}

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

function ReviewCard({ approval }: { approval: Approval }) {
  const { t, lang } = useT();
  const payload = parsePayload(approval.payload);
  const title = payload.title ?? payload.summary ?? approval.task_title ?? t("review.title");
  const time = relativeTime(approval.created_at, lang);
  const badge =
    payload.verdict === "changes_requested"
      ? { className: "badge badge--danger", label: t("inbox.review.changes") }
      : payload.verdict === "approve_with_reservations"
        ? { className: "badge badge--warning", label: t("inbox.review.reserve") }
        : { className: "badge badge--warning", label: t("inbox.review.badge") };

  return (
    <article className="card item" aria-label={title}>
      <div className="item-head">
        <span className={badge.className}>{badge.label}</span>
        {time && <time className="time" dateTime={approval.created_at}>{time}</time>}
      </div>
      <h3 className="item-title">{title}</h3>
      <p className="item-sub">{contextLine(approval)}</p>
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
  const payload = parsePayload(approval.payload);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);

  async function decide(granted: boolean) {
    if (busy) return;
    setBusy(true);
    setError(false);
    try {
      await api.decide(approval.id, granted);
      onDecided();
    } catch {
      setError(true);
    } finally {
      setBusy(false);
    }
  }

  const title = approval.task_title ?? t("inbox.permission.blocked");
  return (
    <article className="card item" aria-label={title}>
      <div className="item-head">
        <span className="badge badge--warning">{t("inbox.permission.badge")}</span>
      </div>
      <h3 className="item-title">{title}</h3>
      <p className="item-sub">{contextLine(approval)}</p>
      <code className="cmd">{payload.command ?? approval.payload}</code>
      <div className="item-actions">
        <button
          className="btn btn--primary btn--sm"
          type="button"
          disabled={busy}
          onClick={() => decide(true)}
        >
          {t("inbox.permission.allow")}
        </button>
        <button
          className="btn btn--ghost btn--sm"
          type="button"
          disabled={busy}
          onClick={() => decide(false)}
        >
          {t("inbox.permission.deny")}
        </button>
      </div>
      {error && <p className="decision-error" role="alert">{t("inbox.decision.error")}</p>}
    </article>
  );
}

function SpecCard({
  approval,
  onDecided,
}: {
  approval: Approval;
  onDecided: () => void;
}) {
  const { t, lang } = useT();
  const payload = parsePayload(approval.payload);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const title = approval.task_title ?? payload.title ?? t("inbox.spec.title");
  const summary = payload.summary ?? t("inbox.spec.body");
  const time = relativeTime(approval.created_at, lang);

  async function decide(granted: boolean) {
    if (busy) return;
    setBusy(true);
    setError(false);
    try {
      await api.decide(approval.id, granted);
      onDecided();
    } catch {
      setError(true);
    } finally {
      setBusy(false);
    }
  }

  return (
    <article className="card item" aria-label={title}>
      <div className="item-head">
        <span className="badge badge--accent">{t("inbox.spec.badge")}</span>
        {time && <time className="time" dateTime={approval.created_at}>{time}</time>}
      </div>
      <h3 className="item-title">{title}</h3>
      <p className="item-sub">{summary}</p>
      <p className="item-sub">{contextLine(approval)}</p>
      <div className="item-actions">
        <button
          className="btn btn--primary btn--sm"
          type="button"
          disabled={busy}
          onClick={() => decide(true)}
        >
          {t("inbox.spec.approve")}
        </button>
        <button
          className="btn btn--ghost btn--sm"
          type="button"
          disabled={busy}
          onClick={() => decide(false)}
        >
          {t("inbox.spec.reject")}
        </button>
      </div>
      {error && <p className="decision-error" role="alert">{t("inbox.decision.error")}</p>}
    </article>
  );
}

export function InboxScreen() {
  const { t, lang } = useT();
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
  const specs = list.filter((a) => a.kind === "spec");
  const permissions = list.filter((a) => a.kind === "permission");
  const active = projects.data ?? [];
  const allClear = !loading && list.length === 0 && active.length === 0;

  return (
    <Shell title="Inbox">
      {auth.data && !auth.data.claude.authenticated && !auth.data.codex.authenticated && (
        <Link
          className="card row inbox-auth-banner"
          to="/settings"
        >
          <div className="row-main">
            <p>{t("inbox.auth.banner")}</p>
          </div>
          <span className="badge badge--warning">{t("inbox.auth.cta")}</span>
        </Link>
      )}
      {loading && <Skeletons label={t("inbox.loading")} />}
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
          {(reviews.length > 0 || specs.length > 0) && (
            <div className="sec">
              <h2 className="sec-title">
                {t("inbox.approvals")} <span className="count">({reviews.length + specs.length})</span>
              </h2>
              {specs.map((a) => (
                <SpecCard key={a.id} approval={a} onDecided={approvals.reload} />
              ))}
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
                    <p>
                      {p.last_activity_at
                        ? `${t("inbox.project.activity")} ${relativeTime(p.last_activity_at, lang)}`
                        : p.github_repo}
                    </p>
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
