// Settings — Connexions (agent providers) + Équipe IA (role → provider,
// the cost control). Mobile-first, in the mockups' design language.

import { useEffect, useState } from "react";
import { api, type AgentProvider, type Role, type Routing } from "../api";
import { useAsync } from "../hooks";
import { useT } from "../i18n";
import { Shell } from "../components/Shell";
import { ProviderAuthCard } from "../components/ProviderAuthCard";
import { ErrorState, Skeletons } from "../components/states";

const PROVIDERS: { id: AgentProvider; label: string; cost: "cost.claude" | "cost.codex" }[] = [
  { id: "claude", label: "Claude", cost: "cost.claude" },
  { id: "codex", label: "Codex", cost: "cost.codex" },
];

const ROLE_IDS = ["manager", "architect", "backend", "frontend", "reviewer"] as const;

function ConnectionsSection() {
  const { t } = useT();
  const status = useAsync(api.agentAuthStatusAll, []);

  if (status.loading) return <Skeletons />;
  const data = status.data;
  if (status.error || !data) {
    return (
      <ErrorState
        title={t("inbox.error.title")}
        body={t("inbox.error.body")}
        onRetry={status.reload}
      />
    );
  }
  return (
    <>
      {PROVIDERS.map((p) => (
        <ProviderAuthCard
          key={p.id}
          provider={p.id}
          label={p.label}
          status={data[p.id]}
          onChanged={status.reload}
        />
      ))}
    </>
  );
}

function TeamSection() {
  const { t } = useT();
  const roles = useAsync(api.roles, []);
  const routing = useAsync(api.getRouting, []);
  const [draft, setDraft] = useState<Routing | null>(null);
  const [saved, setSaved] = useState(false);
  const [saveError, setSaveError] = useState(false);

  // The draft tracks the server state until the user touches a selector.
  useEffect(() => {
    if (routing.data && !draft) setDraft(routing.data);
  }, [routing.data, draft]);

  if (roles.loading || routing.loading) return <Skeletons />;
  if (roles.error || routing.error || !draft) {
    return (
      <ErrorState
        title={t("inbox.error.title")}
        body={t("inbox.error.body")}
        onRetry={() => {
          roles.reload();
          routing.reload();
        }}
      />
    );
  }

  const labelOf = (id: string) =>
    (roles.data ?? []).find((r: Role) => r.id === id)?.label ?? id;

  async function save() {
    if (!draft) return;
    setSaved(false);
    setSaveError(false);
    try {
      await api.putRouting(draft);
      setSaved(true);
      routing.reload();
    } catch {
      setSaveError(true);
    }
  }

  return (
    <>
      {ROLE_IDS.map((role) => (
        <div className="card" style={{ marginBottom: "var(--space-2)", padding: "var(--space-3)" }} key={role}>
          <div className="item-head" style={{ marginBottom: 0 }}>
            <strong style={{ textTransform: "capitalize" }}>{labelOf(role)}</strong>
            <span className="seg" aria-label={role}>
              {PROVIDERS.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  aria-pressed={draft[role] === p.id}
                  title={t(p.cost)}
                  onClick={() => {
                    setSaved(false);
                    setDraft({ ...draft, [role]: p.id });
                  }}
                >
                  {p.label}
                </button>
              ))}
            </span>
          </div>
          <p className="hint" style={{ marginTop: "var(--space-1)" }}>
            {t(draft[role] === "claude" ? "cost.claude" : "cost.codex")}
          </p>
        </div>
      ))}
      <div className="submitbar">
        <button className="btn btn--primary btn--block" type="button" onClick={save}>
          {t("settings.save")}
        </button>
        {saved && <p className="hint" style={{ textAlign: "center" }}>{t("settings.saved")}</p>}
        {saveError && (
          <p className="hint" style={{ textAlign: "center", color: "var(--danger)" }}>
            {t("settings.save.error")}
          </p>
        )}
        <p className="hint" style={{ textAlign: "center" }}>{t("settings.team.note")}</p>
      </div>
    </>
  );
}

export function SettingsScreen() {
  const { t } = useT();
  return (
    <Shell back={t("shell.back.inbox")} title={t("settings.title")}>
      <div className="sec">
        <h2 className="sec-title">{t("settings.connections")}</h2>
        <ConnectionsSection />
      </div>
      <div className="sec">
        <h2 className="sec-title">{t("settings.team")}</h2>
        <TeamSection />
      </div>
    </Shell>
  );
}
