// One provider's connection card (Settings → Connexions). The status comes
// from the provider's own CLI via the server: a connected provider shows
// its state and a Disconnect action — never a Connect button. A
// disconnected provider shows the connect flow (Claude: URL + paste; Codex:
// URL + device code).

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

  // A later successful status refresh (for example, login in another tab)
  // supersedes the optimistic local disconnect.
  useEffect(() => {
    if (status?.authenticated) setLocallyDisconnected(false);
  }, [status?.authenticated]);

  // Poll while a login session is in flight.
  const active = session && !TERMINAL.includes(session.status);
  useEffect(() => {
    if (!active || !session) return;
    const timer = setInterval(async () => {
      try {
        const next = await api.agentAuthStatus(session.session_id);
        setSession(next);
        if (TERMINAL.includes(next.status)) onChanged();
      } catch {
        // retried next tick
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

  async function submit(e: React.FormEvent) {
    e.preventDefault();
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

  // ── Connected: state first, disconnect behind an inline confirm ──
  if (
    !locallyDisconnected &&
    (status?.authenticated || session?.status === "authenticated") &&
    !active
  ) {
    return (
      <div className="card" style={{ marginBottom: "var(--space-3)" }}>
        <div className="item-head" style={{ marginBottom: 0 }}>
          <span style={{ display: "inline-flex", alignItems: "center", gap: "var(--space-2)" }}>
            <span className="conn">
              <span className="dot" aria-hidden="true" />
            </span>
            <strong>{label}</strong>
            <span className="badge badge--success">{t("auth.connected.detail")}</span>
            {status?.detail && <span className="item-sub">{status.detail}</span>}
          </span>
          {confirming ? (
            <span style={{ display: "inline-flex", gap: "var(--space-2)", alignItems: "center" }}>
              <span className="item-sub">{t("auth.disconnect.confirm")}</span>
              <button className="btn btn--danger btn--sm" type="button" disabled={busy} onClick={disconnect}>
                {t("auth.disconnect")}
              </button>
              <button className="btn btn--ghost btn--sm" type="button" onClick={() => setConfirming(false)}>
                {t("auth.cancel")}
              </button>
            </span>
          ) : (
            <button className="btn btn--ghost btn--sm" type="button" onClick={() => setConfirming(true)}>
              {t("auth.disconnect")}
            </button>
          )}
        </div>
        {actionError && (
          <p className="hint" role="alert" style={{ color: "var(--danger)" }}>
            {t("auth.action.error")}
          </p>
        )}
      </div>
    );
  }

  // ── Not connected: the connect flow ──
  return (
    <div className="card" style={{ marginBottom: "var(--space-3)" }}>
      <div className="item-head">
        <strong>{label}</strong>
        {session?.status === "failed" && <span className="badge badge--danger">{t("auth.failed")}</span>}
        {session?.status === "expired" && <span className="badge badge--danger">{t("auth.expired")}</span>}
      </div>

      {actionError && (
        <p className="hint" role="alert" style={{ color: "var(--danger)" }}>
          {t("auth.action.error")}
        </p>
      )}

      {!session && (
        <div className="item-actions">
          <button className="btn btn--primary btn--sm" type="button" disabled={busy} onClick={start}>
            {busy ? t("state.loading") : t(provider === "claude" ? "auth.connect" : "auth.connect.codex")}
          </button>
        </div>
      )}

      {session && (session.status === "starting" || session.status === "validating") && (
        <p className="item-sub">
          <span className="spin" aria-hidden="true" /> {t("state.loading")}
        </p>
      )}

      {session && session.status === "waiting_for_input" && (
        <>
          <div className="item-actions">
            {session.url && (
              <a className="btn btn--primary btn--sm" href={session.url} target="_blank" rel="noreferrer">
                {t("auth.open")}
              </a>
            )}
          </div>
          {session.input_required ? (
            <form className="composer" onSubmit={submit}>
              <label htmlFor={`auth-code-${provider}`} className="sr-only">
                {t("auth.paste")}
              </label>
              <input
                id={`auth-code-${provider}`}
                type="text"
                autoComplete="off"
                placeholder={t("auth.paste")}
                value={code}
                onChange={(e) => setCode(e.target.value)}
              />
              <button className="send" type="submit" aria-label={t("auth.validate")} disabled={busy}>
                ➤
              </button>
            </form>
          ) : (
            <>
              {session.user_code ? (
                <>
                  <p className="item-sub">{t("auth.enter.code")}</p>
                  <code className="cmd" style={{ fontSize: "var(--text-lg)", textAlign: "center" }}>
                    {session.user_code}
                  </code>
                </>
              ) : (
                session.hint && <code className="cmd">{session.hint}</code>
              )}
              <p className="item-sub">
                <span className="spin" aria-hidden="true" /> {t("auth.waiting")}
              </p>
            </>
          )}
        </>
      )}

      {session && TERMINAL.includes(session.status) && session.status !== "authenticated" && (
        <>
          {session.error && <code className="cmd">{session.error}</code>}
          <div className="item-actions">
            <button className="btn btn--ghost btn--sm" type="button" disabled={busy} onClick={start}>
              {t("auth.retry")}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
