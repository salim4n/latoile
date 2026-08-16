// Onboarding (D9): paste the token `latoile serve` printed. Probed against
// the API before being stored; a refusal shows the error state inline.

import { useState } from "react";
import { api, setToken } from "../api";
import { LangToggle, useT } from "../i18n";

export function TokenScreen({ onAccepted }: { onAccepted: () => void }) {
  const { t } = useT();
  const [value, setValue] = useState("");
  const [error, setError] = useState(false);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!value.trim() || busy) return;
    setBusy(true);
    setError(false);
    setToken(value.trim());
    try {
      await api.projects(); // the probe: 401 empties the token again
      onAccepted();
    } catch {
      setError(true);
      setBusy(false);
    }
  }

  return (
    <div className="token-screen">
      <div className="card token-card">
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <h1>{t("token.title")}</h1>
          <LangToggle />
        </div>
        <p className="lead">{t("token.lead")}</p>
        <form onSubmit={submit}>
          <label className="sr-only" htmlFor="token-input">
            {t("token.label")}
          </label>
          <input
            id="token-input"
            type="password"
            autoComplete="off"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder={t("token.label")}
          />
          {error && <p className="token-error">{t("token.error")}</p>}
          <button className="btn btn--primary btn--block" type="submit" disabled={busy}>
            {t("token.submit")}
          </button>
        </form>
      </div>
    </div>
  );
}
