// New project — GitHub repo picker (radio cards) + multi-line brief +
// bottom-sticky primary action, per the mockup's default/sending states.
// The brief becomes the first message to the project's Manager.

import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api";
import { useAsync } from "../hooks";
import { useT } from "../i18n";
import { Shell } from "../components/Shell";
import { ErrorState, Skeletons } from "../components/states";

export function NewProjectScreen() {
  const { t } = useT();
  const navigate = useNavigate();
  const repos = useAsync(api.repos, []);
  const [selected, setSelected] = useState<string | null>(null);
  const [brief, setBrief] = useState("");
  const [sending, setSending] = useState(false);
  const [failed, setFailed] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!selected || !brief.trim() || sending) return;
    setSending(true);
    setFailed(false);
    try {
      const name = selected.split("/").pop() ?? selected;
      const project = await api.createProject({
        name,
        slug: name.toLowerCase().replace(/[^a-z0-9]+/g, "-"),
        github_repo: selected,
        work_branch: "work",
        // The workspace layout is the orchestrator pass's business; the
        // repo identity is what V1 records here.
        local_path: selected,
        dev_command: "pnpm dev --port $PORT",
      });
      await api.sendMessage(project.id, brief.trim());
      navigate(`/projects/${project.id}`);
    } catch {
      setFailed(true);
      setSending(false);
    }
  }

  return (
    <Shell back={t("shell.back.projects")} title={t("new.title")}>
      {repos.loading && <Skeletons />}
      {repos.error && (
        <ErrorState
          title={t("new.repo.error")}
          body={t("inbox.error.body")}
          onRetry={repos.reload}
        />
      )}
      {repos.data && (
        <form aria-label={t("new.title")} onSubmit={submit}>
          <fieldset className="fieldset" disabled={sending}>
            <legend className="legend">{t("new.repo.legend")}</legend>
            <div className="repo-list" role="radiogroup" aria-label={t("new.repo.legend")}>
              {repos.data.map((repo) => (
                <label className="card repo" key={repo.full_name}>
                  <input
                    type="radio"
                    name="repo"
                    value={repo.full_name}
                    checked={selected === repo.full_name}
                    onChange={() => setSelected(repo.full_name)}
                  />
                  <span className="repo-main">
                    <span className="repo-name">{repo.full_name}</span>
                    <span className="repo-desc">{repo.description ?? ""}</span>
                  </span>
                </label>
              ))}
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
              value={brief}
              onChange={(e) => setBrief(e.target.value)}
            />
            <p className="hint">{t("new.brief.hint")}</p>
          </div>

          <div className="submitbar">
            <button
              className="btn btn--primary btn--block"
              type="submit"
              disabled={!selected || !brief.trim() || sending}
            >
              {sending && <span className="spin" aria-hidden="true" />}
              {sending ? t("new.sending") : t("new.submit")}
            </button>
            {sending && (
              <p className="hint" style={{ textAlign: "center" }}>
                {t("new.sending.note")}
              </p>
            )}
            {failed && (
              <p className="hint" style={{ textAlign: "center", color: "var(--danger)" }}>
                {t("new.failed")}
              </p>
            )}
          </div>
        </form>
      )}
    </Shell>
  );
}
