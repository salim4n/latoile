// The project workspace: Chat with the Manager, the task board, the live
// preview — one tab visible at a time (the mockup annotates exactly this).
// All three refresh from the SSE journal (D10).

import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api, getToken, type Message, type Task } from "../api";
import { useAsync, useEventReload } from "../hooks";
import { useT, type Key } from "../i18n";
import { Shell } from "../components/Shell";
import { EmptyState, ErrorState, Skeletons } from "../components/states";
import { CheckIcon, PlayIcon, SendIcon, WarningIcon } from "../components/icons";

type Tab = "chat" | "board" | "preview";

// ── Chat ─────────────────────────────────────────────────────────────────────

/// The Manager's structured actions, when they parse. The format is the
/// orchestrator pass's to define; V1 renders title-ish fields as cards and
/// ignores the rest.
function ActionCard({ actions }: { actions: string }) {
  let parsed: unknown;
  try {
    parsed = JSON.parse(actions);
  } catch {
    return null;
  }
  if (!Array.isArray(parsed)) return null;
  const rows = parsed.filter(
    (a): a is { title?: string; sub?: string } => typeof a === "object" && a !== null,
  );
  if (rows.length === 0) return null;
  return (
    <div className="action-card">
      {rows.map((row, i) => (
        <div className="action-row" key={i}>
          {i % 2 === 0 ? <CheckIcon /> : <PlayIcon />}
          <div>
            <strong>{row.title ?? JSON.stringify(row)}</strong>
            {row.sub && <p>{row.sub}</p>}
          </div>
        </div>
      ))}
    </div>
  );
}

function ChatTab({ project }: { project: string }) {
  const { t } = useT();
  const messages = useAsync(() => api.messages(project), [project]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  useEventReload(["message_posted"], messages.reload);

  async function send(e: React.FormEvent) {
    e.preventDefault();
    const content = draft.trim();
    if (!content || sending) return;
    setSending(true);
    try {
      await api.sendMessage(project, content);
      setDraft("");
      messages.reload();
    } finally {
      setSending(false);
    }
  }

  const list = messages.data ?? [];
  return (
    <>
      {messages.loading && <Skeletons />}
      {messages.error && (
        <ErrorState title={t("inbox.error.title")} body={t("inbox.error.body")} onRetry={messages.reload} />
      )}
      {messages.data && list.length === 0 && (
        <EmptyState title={t("tabs.chat")} body={t("chat.empty")} />
      )}
      {list.map((m: Message) => (
        <div className={m.author === "user" ? "msg msg--user" : "msg"} key={m.id}>
          <div className="msg-meta">{m.author === "user" ? t("chat.you") : t("chat.manager")}</div>
          <div className="msg-body">
            {m.content}
            {m.actions && <ActionCard actions={m.actions} />}
          </div>
        </div>
      ))}

      <form className="composer" onSubmit={send} aria-label={t("chat.placeholder")}>
        <label htmlFor="composer-input" className="sr-only">
          {t("chat.placeholder")}
        </label>
        <input
          id="composer-input"
          type="text"
          placeholder={t("chat.placeholder")}
          autoComplete="off"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
        />
        <button className="send" type="submit" aria-label={t("chat.send")} disabled={sending}>
          <SendIcon />
        </button>
      </form>
    </>
  );
}

// ── Board ────────────────────────────────────────────────────────────────────

const COLUMNS: { key: Key; statuses: Task["status"][] }[] = [
  { key: "board.ready", statuses: ["ready", "changes_requested"] },
  { key: "board.progress", statuses: ["in_progress"] },
  { key: "board.review", statuses: ["review"] },
  { key: "board.done", statuses: ["done"] },
];

function BoardTab({ project }: { project: string }) {
  const { t } = useT();
  const tasks = useAsync(() => api.tasks(project), [project]);
  useEventReload(["task_ready", "run_started", "run_finished", "approval_granted"], tasks.reload);

  if (tasks.loading) return <Skeletons />;
  if (tasks.error) {
    return <ErrorState title={t("inbox.error.title")} body={t("inbox.error.body")} onRetry={tasks.reload} />;
  }
  const list = tasks.data ?? [];
  return (
    <div className="board" aria-label={t("tabs.board")}>
      {COLUMNS.map((col) => {
        const inColumn = list.filter((task) => col.statuses.includes(task.status));
        return (
          <div className="col" key={col.key}>
            <div className="col-head">
              <span>{t(col.key)}</span> <span>{inColumn.length}</span>
            </div>
            {inColumn.map((task) => (
              <article className="task" key={task.id}>
                <h4>{task.title}</h4>
                <div className="task-foot">
                  <span className="agent">{task.role_id}</span>
                  <span>{task.id.slice(0, 6)}</span>
                </div>
              </article>
            ))}
          </div>
        );
      })}
    </div>
  );
}

// ── Preview ──────────────────────────────────────────────────────────────────

function PreviewTab({ project }: { project: string }) {
  const { t } = useT();
  const preview = useAsync(
    () => api.preview(project).catch(() => null), // 404 = nothing running
    [project],
  );
  const [desktop, setDesktop] = useState(false);
  useEventReload(["preview_ready", "preview_stale", "preview_error"], preview.reload);

  const current = preview.data;
  const url = `/api/projects/${project}/preview/?token=${getToken() ?? ""}`;

  async function ensure() {
    await api.ensurePreview(project);
    preview.reload();
  }

  return (
    <>
      <div className="preview-toolbar">
        <span className="url">{url.replace(/\?token=.*/, "")}</span>
        {current?.status === "ready" && (
          <span className="badge badge--success">
            <span className="pulse-dot" aria-hidden="true" />
            {t("preview.live")}
          </span>
        )}
        {(current?.status === "error" || current?.status === "stale") && (
          <span className="badge badge--danger">{t("preview.error.badge")}</span>
        )}
        <span className="seg" aria-label="Format">
          <button type="button" aria-pressed={!desktop} onClick={() => setDesktop(false)}>
            {t("preview.mobile")}
          </button>
          <button type="button" aria-pressed={desktop} onClick={() => setDesktop(true)}>
            {t("preview.desktop")}
          </button>
        </span>
      </div>

      {preview.loading && <Skeletons />}
      {!preview.loading && !current && (
        <EmptyState
          title={t("preview.off.title")}
          body={t("preview.off.body")}
          action={
            <button className="btn btn--primary" type="button" onClick={ensure}>
              {t("preview.start")}
            </button>
          }
        />
      )}
      {current && (current.status === "ready" || current.status === "starting") && (
        <div className={desktop ? "phone phone--desktop" : "phone"}>
          <div className="phone-screen">
            {current.status === "ready" && <iframe title="preview" src={url} />}
            {current.status === "starting" && (
              <div className="build-error">
                <span className="spin" aria-hidden="true" />
                <p>{t("state.loading")}</p>
              </div>
            )}
          </div>
        </div>
      )}
      {current && (current.status === "error" || current.status === "stale") && (
        <div className="phone">
          <div className="phone-screen">
            <div className="build-error">
              <WarningIcon size={28} />
              <h5>{t("preview.error.title")}</h5>
              <p>{t("preview.error.body")}</p>
              <button className="btn btn--ghost btn--sm" type="button" onClick={ensure}>
                {t("preview.retry")}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

// ── The screen ───────────────────────────────────────────────────────────────

export function ProjectScreen() {
  const { t } = useT();
  const { id = "" } = useParams();
  const project = useAsync(() => api.project(id), [id]);
  const [tab, setTab] = useState<Tab>("chat");

  const tabs: { key: Tab; label: string }[] = [
    { key: "chat", label: t("tabs.chat") },
    { key: "board", label: t("tabs.board") },
    { key: "preview", label: t("tabs.preview") },
  ];

  return (
    <Shell
      back={t("shell.back.projects")}
      title={project.data?.name ?? "…"}
      crumb={
        <>
          <Link to="/projects">{t("nav.projects")}</Link>
          {" / "}
          {project.data?.name ?? "…"}
        </>
      }
      wide
    >
      <div className="tabs" role="tablist" aria-label="Project tabs">
        {tabs.map((item) => (
          <button
            key={item.key}
            type="button"
            role="tab"
            aria-selected={tab === item.key}
            onClick={() => setTab(item.key)}
          >
            {item.label}
          </button>
        ))}
      </div>
      {tab === "chat" && <ChatTab project={id} />}
      {tab === "board" && <BoardTab project={id} />}
      {tab === "preview" && <PreviewTab project={id} />}
    </Shell>
  );
}
