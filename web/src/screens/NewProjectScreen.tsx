// New project — GitHub repo picker, multi-line brief, and bottom-sticky
// primary action. The repository name becomes the project name; the brief is
// posted as the first durable Manager message after creation.

import { useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { api } from "../api";
import { useAsync } from "../hooks";
import { useT } from "../i18n";
import { Shell } from "../components/Shell";
import { EmptyState, ErrorState } from "../components/states";
import { PlusIcon } from "../components/icons";

const CONNECT_GITHUB_URL = "https://github.com/settings/tokens";

function RepositoryLoading({ label }: { label: string }) {
  return (
    <div className="repo-loading" role="status" aria-label={label} aria-busy="true">
      <div className="skel skel-line skel-line--short" />
      {[72, 58, 66, 82].map((width) => (
        <div className="card repo repo--skeleton" data-testid="repo-skeleton" aria-hidden="true" key={width}>
          <div className="skel repo-radio-skeleton" />
          <div className="repo-main">
            <div className="skel skel-title" style={{ width: `${width}%` }} />
            <div className="skel skel-line" />
          </div>
          <div className="skel skel-badge" />
        </div>
      ))}
    </div>
  );
}

function ConnectRepositoryLink({ compact = false }: { compact?: boolean }) {
  const { t } = useT();
  return (
    <a
      className={compact ? "btn btn--primary" : "repo-more"}
      href={CONNECT_GITHUB_URL}
      target="_blank"
      rel="noreferrer"
    >
      <PlusIcon />
      {t(compact ? "new.repo.empty.cta" : "new.repo.more")}
    </a>
  );
}

export function NewProjectScreen() {
  const { t } = useT();
  const navigate = useNavigate();
  const repos = useAsync(api.repos, []);
  const [selected, setSelected] = useState<string | null>(null);
  const [brief, setBrief] = useState("");
  const [devCommand, setDevCommand] = useState("");
  const [briefError, setBriefError] = useState(false);
  const [sending, setSending] = useState(false);
  const [failed, setFailed] = useState(false);

  const effectiveSelected = selected ?? repos.data?.[0]?.full_name ?? null;

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (sending) return;
    if (!brief.trim()) {
      setBriefError(true);
      return;
    }
    if (!effectiveSelected) return;

    setSending(true);
    setFailed(false);
    setBriefError(false);
    try {
      const name = effectiveSelected.split("/").pop() ?? effectiveSelected;
      const body: Record<string, string> = {
        name,
        slug: name.toLowerCase().replace(/[^a-z0-9]+/g, "-"),
        github_repo: effectiveSelected,
        work_branch: "work",
      };
      if (devCommand.trim()) body.dev_command = devCommand.trim();
      const project = await api.createProject(body);
      await api.sendMessage(project.id, brief.trim());
      navigate(`/projects/${project.id}`);
    } catch {
      setFailed(true);
      setSending(false);
    }
  }

  return (
    <Shell
      back={t("shell.back.projects")}
      title={t("new.title")}
      crumb={
        <>
          <Link to="/projects">{t("nav.projects")}</Link>
          {" / "}
          {t("new.crumb")}
        </>
      }
    >
      <div className="new-project-page">
        {repos.loading && <RepositoryLoading label={t("new.repo.loading")} />}
        {repos.error && (
          <ErrorState
            title={t("new.repo.error")}
            body={t("new.repo.error.body")}
            onRetry={repos.reload}
          />
        )}
        {repos.data && repos.data.length === 0 && (
          <EmptyState
            title={t("new.repo.empty.title")}
            body={t("new.repo.empty.body")}
            action={<ConnectRepositoryLink compact />}
          />
        )}
        {repos.data && repos.data.length > 0 && (
          <form aria-label={t("new.form.aria")} aria-busy={sending} onSubmit={submit} noValidate>
            <fieldset className="fieldset" disabled={sending}>
              <legend className="legend">{t("new.repo.legend")}</legend>
              <div className="repo-list" role="radiogroup" aria-label={t("new.repo.legend")}>
                {repos.data.map((repo) => (
                  <label className="card repo" key={repo.full_name}>
                    <input
                      type="radio"
                      name="repo"
                      value={repo.full_name}
                      disabled={sending}
                      checked={effectiveSelected === repo.full_name}
                      onChange={() => setSelected(repo.full_name)}
                    />
                    <span className="repo-main">
                      <span className="repo-name">{repo.full_name}</span>
                      <span className="repo-desc">{repo.description ?? t("new.repo.no.description")}</span>
                    </span>
                    <span className="badge badge--neutral">
                      {t(repo.private ? "new.repo.private" : "new.repo.public")}
                    </span>
                  </label>
                ))}
                <ConnectRepositoryLink />
              </div>
            </fieldset>

            <div className="fieldset">
              <label className="field-label" htmlFor="brief">
                {t("new.brief.label")}
              </label>
              <textarea
                id="brief"
                name="brief"
                disabled={sending}
                aria-invalid={briefError}
                aria-describedby={briefError ? "brief-error" : "brief-hint"}
                value={brief}
                onChange={(event) => {
                  setBrief(event.target.value);
                  if (event.target.value.trim()) setBriefError(false);
                }}
              />
              {briefError && (
                <p className="field-error" id="brief-error" role="alert">
                  {t("new.brief.required")}
                </p>
              )}
              <p className="hint" id="brief-hint">{t("new.brief.hint")}</p>
            </div>

            <div className="fieldset">
              <label className="field-label" htmlFor="dev-command">
                {t("new.dev.label")}
              </label>
              <input
                className="text-field"
                id="dev-command"
                name="dev-command"
                disabled={sending}
                value={devCommand}
                placeholder={t("new.dev.placeholder")}
                onChange={(event) => setDevCommand(event.target.value)}
              />
              <p className="hint">{t("new.dev.hint")}</p>
            </div>

            <div className="submitbar new-project-submitbar">
              <button className="btn btn--primary btn--block" type="submit" disabled={sending}>
                {sending && <span className="spin" aria-hidden="true" />}
                {sending ? t("new.sending") : t("new.submit")}
              </button>
              {sending && <p className="submit-note">{t("new.sending.note")}</p>}
              {failed && <p className="submit-error" role="alert">{t("new.failed")}</p>}
            </div>
          </form>
        )}
      </div>
    </Shell>
  );
}
