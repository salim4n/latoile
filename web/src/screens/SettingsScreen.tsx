// Settings implements design/screen-settings.html as the visual contract:
// one page-level loading/error state, provider connection cards, an actionable
// no-provider state, then the five fixed role assignments and sticky save bar.

import { useState } from "react";
import { api, type AgentProvider, type Role, type Routing } from "../api";
import { useAsync } from "../hooks";
import { type Key, useT } from "../i18n";
import { Shell } from "../components/Shell";
import { ProviderAuthCard } from "../components/ProviderAuthCard";
import { ErrorState } from "../components/states";

const PROVIDERS: { id: AgentProvider; label: string }[] = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
];

const ROLE_IDS = ["manager", "architect", "backend", "frontend", "reviewer"] as const;
type RoleId = (typeof ROLE_IDS)[number];

const ROLE_MISSIONS: Record<RoleId, Key> = {
  manager: "settings.role.manager",
  architect: "settings.role.architect",
  backend: "settings.role.backend",
  frontend: "settings.role.frontend",
  reviewer: "settings.role.reviewer",
};

function SettingsSkeleton() {
  const { t } = useT();
  return (
    <div role="status" aria-label={t("settings.loading")} aria-busy="true">
      <section className="settings-section" aria-hidden="true">
        <div className="skel settings-skel-title" />
        {[0, 1].map((index) => (
          <div className="skel settings-skel-provider" data-testid="provider-skeleton" key={index} />
        ))}
      </section>
      <section className="settings-section" aria-hidden="true">
        <div className="skel settings-skel-title" />
        {[0, 1, 2, 3, 4].map((index) => (
          <div className="skel settings-skel-role" data-testid="role-skeleton" key={index} />
        ))}
      </section>
    </div>
  );
}

function SectionHeader({ id, title, body }: { id: string; title: string; body: string }) {
  return (
    <div className="settings-section-head">
      <h2 id={id}>{title}</h2>
      <p>{body}</p>
    </div>
  );
}

function RoleRoutingForm({
  roles,
  draft,
  onChange,
  onSave,
}: {
  roles: Role[];
  draft: Routing;
  onChange: (routing: Routing) => void;
  onSave: (routing: Routing) => Promise<void>;
}) {
  const { t } = useT();
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [saveError, setSaveError] = useState(false);

  const labelOf = (id: RoleId) => roles.find((role) => role.id === id)?.label ?? id;

  async function save() {
    setSaving(true);
    setSaved(false);
    setSaveError(false);
    try {
      await onSave(draft);
      setSaved(true);
    } catch {
      setSaveError(true);
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="settings-section" aria-labelledby="settings-team-title">
      <SectionHeader id="settings-team-title" title={t("settings.team")} body={t("settings.team.body")} />
      <div className="settings-role-list">
        {ROLE_IDS.map((role) => {
          const label = labelOf(role);
          return (
            <article className="card settings-role" data-testid="role-routing" key={role}>
              <div className="settings-role-copy">
                <h3>{label}</h3>
                <p>{t(ROLE_MISSIONS[role])}</p>
              </div>
              <div>
                <div
                  className="seg settings-provider-seg"
                  role="group"
                  aria-label={`${t("settings.provider.for")} ${label}`}
                >
                  {PROVIDERS.map((provider) => (
                    <button
                      key={provider.id}
                      type="button"
                      aria-pressed={draft[role] === provider.id}
                      onClick={() => {
                        setSaved(false);
                        setSaveError(false);
                        onChange({ ...draft, [role]: provider.id });
                      }}
                    >
                      {provider.label}
                    </button>
                  ))}
                </div>
                <span className="settings-cost">
                  {t(draft[role] === "claude" ? "cost.claude" : "cost.codex")}
                </span>
              </div>
            </article>
          );
        })}
      </div>

      <div className="settings-savebar">
        <button
          className="btn btn--primary btn--block"
          type="button"
          disabled={saving}
          onClick={save}
        >
          {t(saving ? "settings.saving" : "settings.save")}
        </button>
        <div className="settings-save-feedback" aria-live="polite">
          {saved && <p className="settings-saved">{t("settings.saved")}</p>}
          {saveError && <p className="settings-save-error" role="alert">{t("settings.save.error")}</p>}
        </div>
        <p className="settings-save-note">{t("settings.team.note")}</p>
      </div>
    </section>
  );
}

export function SettingsScreen() {
  const { t } = useT();
  const statuses = useAsync(api.agentAuthStatusAll, []);
  const roles = useAsync(api.roles, []);
  const routing = useAsync(api.getRouting, []);
  const [draft, setDraft] = useState<Routing | null>(null);

  const currentDraft = draft ?? routing.data;
  const loading = statuses.loading || roles.loading || routing.loading;
  const failed = statuses.error || roles.error || routing.error;

  function reload() {
    statuses.reload();
    roles.reload();
    routing.reload();
  }

  async function save(next: Routing) {
    const persisted = await api.putRouting(next);
    setDraft(persisted);
  }

  let content;
  if (loading) {
    content = <SettingsSkeleton />;
  } else if (failed || !statuses.data || !roles.data || !currentDraft) {
    content = (
      <ErrorState
        title={t("settings.error.title")}
        body={t("settings.error.body")}
        onRetry={reload}
      />
    );
  } else {
    const noneConnected = !statuses.data.claude.authenticated && !statuses.data.codex.authenticated;
    content = (
      <>
        <section className="settings-section" aria-labelledby="settings-connections-title">
          <SectionHeader
            id="settings-connections-title"
            title={t("settings.connections")}
            body={t("settings.connections.body")}
          />
          {noneConnected && (
            <div className="settings-empty-copy">
              <h2>{t("settings.empty.title")}</h2>
              <p>{t("settings.empty.body")}</p>
            </div>
          )}
          <div className="settings-provider-list">
            {PROVIDERS.map((provider) => (
              <ProviderAuthCard
                key={provider.id}
                provider={provider.id}
                label={provider.label}
                status={statuses.data?.[provider.id] ?? null}
                onChanged={statuses.reload}
              />
            ))}
          </div>
        </section>

        {!noneConnected && (
          <RoleRoutingForm
            roles={roles.data}
            draft={currentDraft}
            onChange={setDraft}
            onSave={save}
          />
        )}
      </>
    );
  }

  return (
    <Shell back={t("shell.back.inbox")} title={t("settings.title")}>
      <div className="settings-page">{content}</div>
    </Shell>
  );
}
