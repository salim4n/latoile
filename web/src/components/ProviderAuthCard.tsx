// Provider connection card from the Settings visual contract. Credentials stay
// owned by the provider CLI; this component only supervises its login challenge.

import { useEffect, useState } from "react";
import { api, type AgentAuthSession, type AgentProvider, type ProviderStatus } from "../api";
import { useT } from "../i18n";

const TERMINAL = ["authenticated", "failed", "expired"];

export function ProviderAuthCard({
  provider,
  label,
  status,
  onChanged,
}: {
  provider: AgentProvider;
  label: string;
  status: ProviderStatus | null;
  onChanged: () => void;
}) {
  const { t } = useT();
  const [session, setSession] = useState<AgentAuthSession | null>(null);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [actionError, setActionError] = useState(false);
  const [locallyDisconnected, setLocallyDisconnected] = useState(false);

  useEffect(() => {
    if (status?.authenticated) setLocallyDisconnected(false);
  }, [status?.authenticated]);

  const active = session && !TERMINAL.includes(session.status);
  useEffect(() => {
    if (!active || !session) return;
    const timer = setInterval(async () => {
      try {
        const next = await api.agentAuthStatus(session.session_id);
        setSession(next);
        if (TERMINAL.includes(next.status)) onChanged();
      } catch {
        // The challenge remains visible and the next interval retries.
      }
    }, 2000);
    return () => clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, session?.session_id]);

  async function start() {
    setBusy(true);
    setActionError(false);
    setLocallyDisconnected(false);
    try {
      setSession(await api.agentAuthStart(provider));
    } catch {
      setActionError(true);
    } finally {
      setBusy(false);
    }
  }

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!session || !code.trim() || busy) return;
    setBusy(true);
    setActionError(false);
    try {
      setSession(await api.agentAuthCode(session.session_id, code.trim()));
      setCode("");
    } catch {
      setActionError(true);
    } finally {
      setBusy(false);
    }
  }

  async function disconnect() {
    setBusy(true);
    setActionError(false);
    try {
      const next = await api.agentAuthDisconnect(provider);
      if (next.authenticated) {
        setActionError(true);
        return;
      }
      setConfirming(false);
      setSession(null);
      setLocallyDisconnected(true);
      onChanged();
    } catch {
      setActionError(true);
    } finally {
      setBusy(false);
    }
  }

  const mark = provider === "claude" ? "CL" : "CX";
  const connected =
    !locallyDisconnected &&
    (status?.authenticated || session?.status === "authenticated") &&
    !active;

  if (connected) {
    return (
      <article className="card settings-provider-card">
        <div className="settings-provider-head">
          <div className="settings-provider-id">
            <span className="settings-provider-mark" aria-hidden="true">{mark}</span>
            <div>
              <div className="settings-provider-name">
                <strong>{label}</strong>
                <span className="badge badge--success">{t("auth.connected.detail")}</span>
              </div>
              {status?.detail && <p className="settings-provider-detail">{status.detail}</p>}
            </div>
          </div>
        </div>

        <div className="settings-provider-actions">
          {confirming ? (
            <div className="settings-confirm" role="group" aria-label={t("auth.disconnect.confirm")}>
              <span>{t("auth.disconnect.confirm")}</span>
              <button className="btn btn--danger btn--sm" type="button" disabled={busy} onClick={disconnect}>
                {t("auth.disconnect")}
              </button>
              <button className="btn btn--ghost btn--sm" type="button" onClick={() => setConfirming(false)}>
                {t("auth.cancel")}
              </button>
            </div>
          ) : (
            <button className="btn btn--danger btn--sm" type="button" onClick={() => setConfirming(true)}>
              {t("auth.disconnect")}
            </button>
          )}
        </div>
        {actionError && <p className="settings-action-error" role="alert">{t("auth.action.error")}</p>}
      </article>
    );
  }

  return (
    <article className={`card settings-provider-card${active ? " settings-provider-card--active" : ""}`}>
      <div className="settings-provider-head">
        <div className="settings-provider-id">
          <span className="settings-provider-mark" aria-hidden="true">{mark}</span>
          <div>
            <div className="settings-provider-name">
              <strong>{label}</strong>
              {!session && <span className="badge badge--neutral">{t("auth.disconnected")}</span>}
              {session?.status === "failed" && <span className="badge badge--danger">{t("auth.failed")}</span>}
              {session?.status === "expired" && <span className="badge badge--danger">{t("auth.expired")}</span>}
            </div>
            {!session && <p className="settings-provider-detail">{t(`auth.${provider}.detail` as const)}</p>}
          </div>
        </div>
      </div>

      {actionError && <p className="settings-action-error" role="alert">{t("auth.action.error")}</p>}

      {!session && (
        <div className="settings-provider-actions">
          <button
            className={`btn ${provider === "claude" ? "btn--primary" : "btn--ghost"} btn--sm`}
            type="button"
            disabled={busy}
            onClick={start}
          >
            {busy ? t("state.loading") : t(provider === "claude" ? "auth.connect" : "auth.connect.codex")}
          </button>
        </div>
      )}

      {session && (session.status === "starting" || session.status === "validating") && (
        <p className="settings-waiting" aria-live="polite">
          <span className="spin" aria-hidden="true" /> {t("state.loading")}
        </p>
      )}

      {session?.status === "waiting_for_input" && (
        <div className="settings-device-flow">
          {session.input_required ? (
            <>
              {session.url && (
                <a className="btn btn--primary btn--block" href={session.url} target="_blank" rel="noreferrer">
                  {t("auth.open")}
                </a>
              )}
              <form className="composer settings-code-form" onSubmit={submit}>
                <label htmlFor={`auth-code-${provider}`} className="sr-only">{t("auth.paste")}</label>
                <input
                  id={`auth-code-${provider}`}
                  type="text"
                  autoComplete="off"
                  placeholder={t("auth.paste")}
                  value={code}
                  onChange={(event) => setCode(event.target.value)}
                />
                <button className="send" type="submit" aria-label={t("auth.validate")} disabled={busy}>➤</button>
              </form>
            </>
          ) : (
            <>
              <p className="settings-device-instruction">{t("auth.enter.code")}</p>
              {session.user_code && <code className="settings-device-code">{session.user_code}</code>}
              {!session.user_code && session.hint && <code className="cmd">{session.hint}</code>}
              {session.url && (
                <a className="btn btn--primary btn--block" href={session.url} target="_blank" rel="noreferrer">
                  {t("auth.open")}
                </a>
              )}
              <p className="settings-waiting" aria-live="polite">
                <span className="settings-pulse" aria-hidden="true" /> {t("auth.waiting")}
              </p>
            </>
          )}
        </div>
      )}

      {session && TERMINAL.includes(session.status) && session.status !== "authenticated" && (
        <div className="settings-device-flow">
          {session.error && <code className="cmd">{session.error}</code>}
          <div className="settings-provider-actions">
            <button className="btn btn--ghost btn--sm" type="button" disabled={busy} onClick={start}>
              {t("auth.retry")}
            </button>
          </div>
        </div>
      )}
    </article>
  );
}
