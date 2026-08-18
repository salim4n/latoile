// The project workspace: Chat with the Manager, the task board, and the live
// preview. The HTML mockup is the visual contract; only one tab is mounted at
// a time and all live data refreshes from the SSE journal.

import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import {
  ApiError,
  api,
  getToken,
  type ArchitectureSession,
  type Delivery,
  type Message,
  type Preview,
  type SpecVersion,
  type Task,
} from "../api";
import { useAsync, useEventReload, type Async } from "../hooks";
import { useT, type Key, type Lang } from "../i18n";
import { Shell } from "../components/Shell";
import { EmptyState, ErrorState } from "../components/states";
import { CheckIcon, GearIcon, PlayIcon, SendIcon, WarningIcon } from "../components/icons";
import { ArchitectureApproval } from "./ArchitectureApproval";

type Tab = "chat" | "board" | "preview";

function DeliveryPanel({ project }: { project: string }) {
  const { t } = useT();
  const delivery = useAsync(() => api.delivery(project), [project]);
  const [latest, setLatest] = useState<Delivery | null>(null);
  const [delivering, setDelivering] = useState(false);
  const [actionError, setActionError] = useState(false);
  const current = latest ?? delivery.data;

  async function deliver() {
    if (delivering) return;
    setDelivering(true);
    setActionError(false);
    try {
      setLatest(await api.deliverProject(project));
    } catch {
      setActionError(true);
      delivery.reload();
    } finally {
      setDelivering(false);
    }
  }

  if (delivery.loading && !current) {
    return <div className="skel delivery-skeleton" role="status" aria-label={t("delivery.loading")} />;
  }
  if (delivery.error && !current) {
    return (
      <div className="delivery-card delivery-card--error">
        <p>{t("delivery.load.error")}</p>
        <button className="btn btn--ghost btn--sm" type="button" onClick={delivery.reload}>
          {t("inbox.error.retry")}
        </button>
      </div>
    );
  }
  if (!current) return null;

  const verified = current.local_sha && current.local_sha === current.remote_sha;
  return (
    <section className="delivery-card" aria-label={t("delivery.aria")}>
      <div className="delivery-copy">
        <div className="delivery-heading">
          <strong>{t("delivery.title")}</strong>
          <span className={current.status === "pull_request_open" ? "badge badge--success" : "badge badge--neutral"}>
            {t(`delivery.status.${current.status}` as Key)}
          </span>
        </div>
        <p>{current.work_branch}</p>
        {verified && (
          <code className="delivery-sha">
            {t("delivery.sha")} {current.local_sha?.slice(0, 12)}
          </code>
        )}
        {actionError && <p className="decision-error" role="alert">{t("delivery.action.error")}</p>}
      </div>
      {current.pull_request_url ? (
        <a className="btn btn--primary btn--sm" href={current.pull_request_url} target="_blank" rel="noreferrer">
          {t("delivery.open_pr")}
        </a>
      ) : (
        <button className="btn btn--primary btn--sm" type="button" onClick={deliver} disabled={delivering}>
          {delivering && <span className="spin" aria-hidden="true" />}
          {current.status === "pushed" ? t("delivery.retry_pr") : t("delivery.deliver")}
        </button>
      )}
    </section>
  );
}

function ProjectLoading({ label }: { label: string }) {
  return (
    <div className="project-loading" role="status" aria-label={label} aria-busy="true">
      <div className="skel project-tabs-skeleton" aria-hidden="true" />
      {[68, 84, 58].map((width) => (
        <div className="project-message-skeleton" data-testid="project-message-skeleton" key={width}>
          <div className="skel skel-line skel-line--short" />
          <div className="skel project-message-body" style={{ width: `${width}%` }} />
        </div>
      ))}
    </div>
  );
}

function BoardLoading({ label }: { label: string }) {
  return (
    <div className="board board--loading" role="status" aria-label={label} aria-busy="true">
      {[0, 1, 2, 3].map((column) => (
        <div className="col" aria-hidden="true" key={column}>
          <div className="skel skel-line skel-line--short" />
          <div className="skel board-task-skeleton" />
          <div className="skel board-task-skeleton" />
        </div>
      ))}
    </div>
  );
}

function PreviewLoading({ label }: { label: string }) {
  return (
    <div className="preview-loading" role="status" aria-label={label} aria-busy="true">
      <div className="skel preview-toolbar-skeleton" />
      <div className="phone" aria-hidden="true">
        <div className="phone-screen">
          <div className="skel preview-screen-skeleton" />
        </div>
      </div>
    </div>
  );
}

// ── Chat ─────────────────────────────────────────────────────────────────────

function ActionCard({ actions }: { actions: string }) {
  let parsed: unknown;
  try {
    parsed = JSON.parse(actions);
  } catch {
    return null;
  }
  if (!Array.isArray(parsed)) return null;
  const rows = parsed.filter(
    (action): action is { title?: string; sub?: string } =>
      typeof action === "object" && action !== null,
  );
  if (rows.length === 0) return null;
  return (
    <div className="action-card">
      {rows.map((row, index) => (
        <div className="action-row" key={index}>
          {index % 2 === 0 ? <CheckIcon /> : <PlayIcon />}
          <div>
            <strong>{row.title ?? JSON.stringify(row)}</strong>
            {row.sub && <p>{row.sub}</p>}
          </div>
        </div>
      ))}
    </div>
  );
}

function messageTime(createdAt: string | undefined, lang: Lang): string {
  if (!createdAt) return "";
  const date = new Date(createdAt);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(lang, { hour: "2-digit", minute: "2-digit" }).format(date);
}

const ARCHITECTURE_STATUS_KEYS: Record<ArchitectureSession["status"], Key> = {
  discovering: "architecture.status.discovering",
  awaiting_answer: "architecture.status.awaiting_answer",
  ready_to_draft: "architecture.status.ready_to_draft",
  failed: "architecture.status.failed",
  cancelled: "architecture.status.cancelled",
};

const ARCHITECTURE_PHASE_KEYS: Record<ArchitectureSession["phase"], Key> = {
  domain_discovery: "architecture.phase.domain_discovery",
  requirements: "architecture.phase.requirements",
  ux_discovery: "architecture.phase.ux_discovery",
  ready_to_draft: "architecture.phase.ready_to_draft",
};

function ArchitecturePanel({
  project,
  architecture,
  specs,
}: {
  project: string;
  architecture: Async<ArchitectureSession | null>;
  specs: Async<SpecVersion[]>;
}) {
  const { t } = useT();
  const [cancelling, setCancelling] = useState(false);
  const [cancelError, setCancelError] = useState(false);
  const session = architecture.data;

  async function cancel() {
    if (cancelling) return;
    setCancelling(true);
    setCancelError(false);
    try {
      await api.cancelArchitecture(project);
      architecture.reload();
    } catch {
      setCancelError(true);
    } finally {
      setCancelling(false);
    }
  }

  if (architecture.loading && !session) {
    return (
      <div
        className="skel architecture-skeleton"
        role="status"
        aria-label={t("architecture.loading")}
      />
    );
  }
  if (architecture.error && !session) {
    return (
      <section className="architecture-card architecture-card--error">
        <p>{t("architecture.load.error")}</p>
        <button className="btn btn--ghost btn--sm" type="button" onClick={architecture.reload}>
          {t("inbox.error.retry")}
        </button>
      </section>
    );
  }
  if (!session) return null;

  const openQuestion = session.questions.find((question) => question.status === "open");
  const active =
    !["failed", "cancelled"].includes(session.status) &&
    session.package_status !== "draft_ready";
  const statusLabel =
    session.package_status === "draft_ready"
      ? t("architecture.package.ready")
      : session.package_status === "generating"
        ? t("architecture.package.generating")
        : t(ARCHITECTURE_STATUS_KEYS[session.status]);
  return (
    <section className="architecture-card" aria-label={t("architecture.aria")}>
      <div className="architecture-heading">
        <div>
          <span className="architecture-eyebrow">{t("architecture.role")}</span>
          <h3>{t("architecture.title")}</h3>
        </div>
        <span
          className={`badge ${session.status === "failed" ? "badge--danger" : session.status === "ready_to_draft" ? "badge--success" : "badge--neutral"}`}
        >
          {statusLabel}
        </span>
      </div>
      <p className="architecture-phase">
        {t("architecture.phase.label")} {t(ARCHITECTURE_PHASE_KEYS[session.phase])}
      </p>
      {session.skill_digest && (
        <p className="architecture-provenance">
          {session.skill_name} · {t("architecture.skill.sha")} {session.skill_digest.slice(0, 12)} ·{" "}
          {session.operating_mode === "reverse_engineering"
            ? t("architecture.mode.reverse_engineering")
            : t("architecture.mode.greenfield")}
        </p>
      )}
      {session.package && (
        <div className="architecture-package">
          <strong>{t("architecture.package.evidence")}</strong>
          <code>{session.package.design_dir}</code>
          <span>
            {session.package.changed_files.length} {t("architecture.package.files")} · commit{" "}
            {session.package.head_sha.slice(0, 12)} · tree {session.package.tree_sha.slice(0, 12)}
          </span>
        </div>
      )}
      {session.package_status === "draft_ready" && (
        <ArchitectureApproval
          draft={(specs.data ?? []).find(
            (spec) =>
              spec.status === "draft" && spec.architecture_session_id === session.id,
          )}
          specs={specs}
        />
      )}
      {openQuestion && (
        <div className="architecture-current">
          <strong>{t("architecture.current_question")}</strong>
          <p>{openQuestion.prompt}</p>
        </div>
      )}
      {session.questions.length > 0 && (
        <ol className="architecture-history" aria-label={t("architecture.history")}>
          {session.questions.map((question) => (
            <li key={question.id}>
              <span>{question.sequence}</span>
              <div>
                <strong>{question.prompt}</strong>
                {question.answer && <p>{question.answer}</p>}
              </div>
            </li>
          ))}
        </ol>
      )}
      {session.failure_reason && (
        <p className="architecture-failure" role="alert">
          {session.failure_reason}
        </p>
      )}
      {cancelError && (
        <p className="decision-error" role="alert">
          {t("architecture.cancel.error")}
        </p>
      )}
      {active && (
        <button
          className="btn btn--ghost btn--sm architecture-cancel"
          type="button"
          onClick={cancel}
          disabled={cancelling}
        >
          {cancelling ? t("architecture.cancelling") : t("architecture.cancel")}
        </button>
      )}
    </section>
  );
}

function ChatTab({ project }: { project: string }) {
  const { t, lang } = useT();
  const messages = useAsync(() => api.messages(project), [project]);
  const architecture = useAsync(() => api.architecture(project), [project]);
  const specs = useAsync(() => api.specs(project), [project]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState(false);
  useEventReload(["message_posted"], messages.reload);
  useEventReload(["message_posted"], architecture.reload);
  useEventReload(["spec_version_created", "spec_approved"], specs.reload);

  async function send(event: React.FormEvent) {
    event.preventDefault();
    const content = draft.trim();
    if (!content || sending) return;
    setSending(true);
    setSendError(false);
    try {
      await api.sendMessage(project, content);
      setDraft("");
      messages.reload();
      architecture.reload();
    } catch {
      setSendError(true);
    } finally {
      setSending(false);
    }
  }

  const list = messages.data ?? [];
  const awaitingArchitect = architecture.data?.status === "awaiting_answer";
  return (
    <div className="chat-panel">
      <ArchitecturePanel project={project} architecture={architecture} specs={specs} />
      <div className="chat-thread">
        {messages.loading && <ProjectLoading label={t("chat.loading")} />}
        {messages.error && (
          <ErrorState title={t("chat.error.title")} body={t("chat.error.body")} onRetry={messages.reload} />
        )}
        {messages.data && list.length === 0 && (
          <EmptyState title={t("tabs.chat")} body={t("chat.empty")} />
        )}
        {list.map((message: Message) => {
          const time = messageTime(message.created_at, lang);
          return (
            <div className={message.author === "user" ? "msg msg--user" : "msg"} key={message.id}>
              <div className="msg-meta">
                {message.author === "user" ? t("chat.you") : t("chat.manager")}
                {time && ` · ${time}`}
              </div>
              <div className="msg-body">
                <p>{message.content}</p>
                {message.actions && <ActionCard actions={message.actions} />}
              </div>
            </div>
          );
        })}
      </div>

      <form className="composer" onSubmit={send} aria-label={t("chat.composer.aria")}>
        {sendError && <p className="composer-error" role="alert">{t("chat.send.error")}</p>}
        <label htmlFor="composer-input" className="sr-only">
          {t(awaitingArchitect ? "architecture.answer.placeholder" : "chat.placeholder")}
        </label>
        <input
          id="composer-input"
          type="text"
          placeholder={t(awaitingArchitect ? "architecture.answer.placeholder" : "chat.placeholder")}
          autoComplete="off"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
        />
        <button className="send" type="submit" aria-label={t("chat.send")} disabled={sending}>
          {sending ? <span className="spin" aria-hidden="true" /> : <SendIcon />}
        </button>
      </form>
    </div>
  );
}

// ── Board ────────────────────────────────────────────────────────────────────

const COLUMNS: { key: Key; statuses: Task["status"][] }[] = [
  { key: "board.ready", statuses: ["ready", "changes_requested"] },
  { key: "board.progress", statuses: ["in_progress"] },
  { key: "board.review", statuses: ["review"] },
  { key: "board.done", statuses: ["done"] },
];

const ROLE_KEYS: Partial<Record<string, Key>> = {
  manager: "role.manager",
  architect: "role.architect",
  backend: "role.backend",
  frontend: "role.frontend",
  reviewer: "role.reviewer",
};

const NEXT_ACTION_KEYS: Record<Task["next_action"], Key> = {
  ready_to_start: "board.next.ready_to_start",
  agent_working: "board.next.agent_working",
  reviewer_working: "board.next.reviewer_working",
  awaiting_owner_decision: "board.next.awaiting_owner_decision",
  changes_requested: "board.next.changes_requested",
  correction_ready: "board.next.correction_ready",
  corrective_run_in_progress: "board.next.corrective_run_in_progress",
  completed: "board.next.completed",
};

function taskReference(task: Task): string {
  return task.latest_run_id ? `${task.id} · run #${task.latest_run_id}` : task.id;
}

function BoardTab({ project }: { project: string }) {
  const { t } = useT();
  const tasks = useAsync(() => api.tasks(project), [project]);
  useEventReload(
    ["task_ready", "run_started", "run_finished", "approval_granted", "approval_rejected"],
    tasks.reload,
  );

  if (tasks.loading) return <BoardLoading label={t("board.loading")} />;
  if (tasks.error) {
    return <ErrorState title={t("board.error.title")} body={t("board.error.body")} onRetry={tasks.reload} />;
  }
  const list = tasks.data ?? [];
  return (
    <div className="board-panel" role="region" aria-label={t("board.aria")}>
      {list.length === 0 && <p className="board-empty">{t("board.empty")}</p>}
      <div className="board">
        {COLUMNS.map((column) => {
          const inColumn = list.filter((task) => column.statuses.includes(task.status));
          return (
            <div className="col" role="group" aria-label={t(column.key)} key={column.key}>
              <div className="col-head">
                <span>{t(column.key)}</span> <span>{inColumn.length}</span>
              </div>
              {inColumn.map((task) => {
                const roleKey = ROLE_KEYS[task.role_id];
                return (
                  <article className="task" key={task.id}>
                    <h4>{task.title}</h4>
                    <p className="task-next-action">{t(NEXT_ACTION_KEYS[task.next_action])}</p>
                    {task.latest_decision_comment && (
                      <p className="task-decision-comment">
                        <strong>{t("board.decision.comment")}</strong>{" "}
                        {task.latest_decision_comment}
                      </p>
                    )}
                    <div className="task-foot">
                      <span className="agent">{roleKey ? t(roleKey) : task.role_id}</span>
                      <span>{taskReference(task)}</span>
                    </div>
                  </article>
                );
              })}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ── Preview ──────────────────────────────────────────────────────────────────

async function loadPreview(project: string): Promise<Preview | null> {
  try {
    return await api.preview(project);
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) return null;
    throw error;
  }
}

function PreviewTab({
  project,
  preview,
}: {
  project: string;
  preview: Async<Preview | null>;
}) {
  const { t } = useT();
  const [desktop, setDesktop] = useState(false);
  const [actionError, setActionError] = useState(false);
  const [starting, setStarting] = useState(false);

  const current = preview.data;
  const url = `/api/projects/${project}/preview/?token=${getToken() ?? ""}`;
  const live = current?.status === "ready" && current.alive;
  const failed = current && (current.status === "error" || current.status === "stale" || !current.alive);

  async function ensure() {
    setStarting(true);
    setActionError(false);
    try {
      await api.ensurePreview(project);
      preview.reload();
    } catch {
      setActionError(true);
    } finally {
      setStarting(false);
    }
  }

  if (preview.loading) return <PreviewLoading label={t("preview.loading")} />;
  if (preview.error) {
    return (
      <ErrorState
        title={t("preview.load.error.title")}
        body={t("preview.load.error.body")}
        onRetry={preview.reload}
      />
    );
  }

  return (
    <div className="preview-panel">
      <div className="preview-toolbar">
        <span className="url">{url.replace(/\?token=.*/, "")}</span>
        {live && (
          <span className="badge badge--success">
            <span className="pulse-dot" aria-hidden="true" />
            {t("preview.live")}
          </span>
        )}
        {failed && <span className="badge badge--danger">{t("preview.error.badge")}</span>}
        <span className="seg" role="group" aria-label={t("preview.format")}>
          <button type="button" aria-pressed={!desktop} onClick={() => setDesktop(false)}>
            {t("preview.mobile")}
          </button>
          <button type="button" aria-pressed={desktop} onClick={() => setDesktop(true)}>
            {t("preview.desktop")}
          </button>
        </span>
      </div>

      {!current && (
        <EmptyState
          title={t("preview.off.title")}
          body={t("preview.off.body")}
          action={
            <button className="btn btn--primary" type="button" onClick={ensure} disabled={starting}>
              {starting && <span className="spin" aria-hidden="true" />}
              {t("preview.start")}
            </button>
          }
        />
      )}
      {actionError && <p className="preview-action-error" role="alert">{t("preview.action.error")}</p>}

      {current && !failed && (current.status === "ready" || current.status === "starting") && (
        <div className={desktop ? "phone phone--desktop" : "phone"} data-testid="preview-frame">
          <div className="phone-screen">
            {current.status === "ready" && (
              <iframe title={t("preview.iframe.title")} src={url} />
            )}
            {current.status === "starting" && (
              <div className="build-error">
                <span className="spin spin--accent" aria-hidden="true" />
                <p>{t("preview.starting")}</p>
              </div>
            )}
          </div>
        </div>
      )}
      {failed && (
        <div className={desktop ? "phone phone--desktop" : "phone"} data-testid="preview-frame">
          <div className="phone-screen">
            <div className="build-error">
              <WarningIcon size={28} />
              <h5>{t("preview.error.title")}</h5>
              <p>{t("preview.error.body")}</p>
              {current.logs.length > 0 && <pre className="build-log">{current.logs.slice(-5).join("\n")}</pre>}
              <button className="btn btn--ghost btn--sm" type="button" onClick={ensure} disabled={starting}>
                {t("preview.retry")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ── The screen ───────────────────────────────────────────────────────────────

export function ProjectScreen() {
  const { t } = useT();
  const { id = "" } = useParams();
  const project = useAsync(() => api.project(id), [id]);
  const preview = useAsync(() => loadPreview(id), [id]);
  const [tab, setTab] = useState<Tab>("chat");
  useEventReload(["preview_ready", "preview_stale", "preview_error"], preview.reload);

  const live = preview.data?.status === "ready" && preview.data.alive;
  const tabs: { key: Tab; label: string }[] = [
    { key: "chat", label: t("tabs.chat") },
    { key: "board", label: t("tabs.board") },
    { key: "preview", label: t("tabs.preview") },
  ];

  const title = project.data?.name ?? "…";
  return (
    <Shell
      back={t("shell.back.projects")}
      title={title}
      crumb={
        <>
          <Link to="/projects">{t("nav.projects")}</Link>
          {" / "}
          {title}
        </>
      }
      action={
        <Link className="icon-btn" to="/settings" aria-label={t("project.settings")}>
          <GearIcon />
        </Link>
      }
      wide
    >
      {project.loading && <ProjectLoading label={t("project.loading")} />}
      {project.error && (
        <ErrorState
          title={t("project.error.title")}
          body={t("project.error.body")}
          onRetry={project.reload}
        />
      )}
      {project.data && (
        <>
          <DeliveryPanel project={id} />
          <div className="tabs" role="tablist" aria-label={t("tabs.aria")}>
            {tabs.map((item) => (
              <button
                id={`project-tab-${item.key}`}
                key={item.key}
                type="button"
                role="tab"
                aria-controls={`project-panel-${item.key}`}
                aria-selected={tab === item.key}
                onClick={() => setTab(item.key)}
              >
                {item.label}
                {item.key === "preview" && live && (
                  <span className="badge badge--success">
                    <span className="pulse-dot" aria-hidden="true" />
                    {t("preview.live")}
                  </span>
                )}
              </button>
            ))}
          </div>
          <section
            id={`project-panel-${tab}`}
            className="project-tabpanel"
            role="tabpanel"
            aria-labelledby={`project-tab-${tab}`}
          >
            {tab === "chat" && <ChatTab project={id} />}
            {tab === "board" && <BoardTab project={id} />}
            {tab === "preview" && <PreviewTab project={id} preview={preview} />}
          </section>
        </>
      )}
    </Shell>
  );
}
