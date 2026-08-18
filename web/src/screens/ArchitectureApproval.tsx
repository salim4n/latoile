import { useState } from "react";
import { api, type SpecVersion } from "../api";
import { useAsync, type Async } from "../hooks";
import { useT } from "../i18n";

export function ArchitectureApproval({
  draft,
  specs,
}: {
  draft: SpecVersion | undefined;
  specs: Async<SpecVersion[]>;
}) {
  const { t } = useT();
  const validation = useAsync(
    () => (draft ? api.validateSpec(draft.id) : Promise.resolve(null)),
    [draft?.id],
  );
  const [artifact, setArtifact] = useState<string | null>(null);
  const [artifactLabel, setArtifactLabel] = useState("");
  const [busyArtifact, setBusyArtifact] = useState(false);
  const [approving, setApproving] = useState(false);
  const [actionError, setActionError] = useState(false);

  if (!draft) return null;
  const proof = validation.data;

  async function showArtifact(path: string, label: string) {
    if (!draft || busyArtifact) return;
    setBusyArtifact(true);
    setActionError(false);
    try {
      setArtifact(await api.specArtifact(draft.id, path));
      setArtifactLabel(label);
    } catch {
      setActionError(true);
    } finally {
      setBusyArtifact(false);
    }
  }

  async function approve() {
    if (!draft || !proof?.valid || approving) return;
    setApproving(true);
    setActionError(false);
    try {
      await api.approveSpec(draft.id);
      setArtifact(null);
      specs.reload();
    } catch {
      setActionError(true);
      validation.reload();
    } finally {
      setApproving(false);
    }
  }

  return (
    <section className="architecture-approval" aria-label={t("architecture.validation.aria")}>
      <div className="architecture-validation-head">
        <strong>{t("architecture.validation.title")}</strong>
        {validation.loading ? (
          <span className="badge badge--neutral">{t("architecture.validation.checking")}</span>
        ) : (
          <span className={`badge ${proof?.valid ? "badge--success" : "badge--danger"}`}>
            {t(proof?.valid ? "architecture.validation.valid" : "architecture.validation.invalid")}
          </span>
        )}
      </div>
      {proof && (
        <>
          <p className="architecture-validation-proof">
            {proof.file_count} {t("architecture.package.files")} · {proof.scenarios.length}{" "}
            {t("architecture.validation.scenarios")} · commit {proof.commit_sha.slice(0, 12)} · manifest{" "}
            {proof.manifest_digest.slice(0, 12)}
          </p>
          <ul className="architecture-findings">
            {proof.findings.map((finding) => (
              <li key={finding.code} className={proof.valid ? "is-valid" : "is-invalid"}>
                <code>{finding.code}</code>
                <span>{finding.message}</span>
              </li>
            ))}
          </ul>
          <div className="architecture-scenarios">
            <button
              className="btn btn--ghost btn--sm"
              type="button"
              disabled={!proof.valid || busyArtifact}
              onClick={() => showArtifact(proof.gallery_path, t("architecture.gallery.title"))}
            >
              {t("architecture.gallery.open")}
            </button>
            {proof.scenarios.map((scenario) => (
              <button
                className="btn btn--ghost btn--sm"
                type="button"
                key={scenario.comparison_id}
                disabled={!proof.valid || busyArtifact}
                onClick={() => showArtifact(scenario.mockup, scenario.comparison_id)}
              >
                {scenario.comparison_id} · {scenario.locale} · {scenario.viewport_width}×
                {scenario.viewport_height}
              </button>
            ))}
          </div>
          {artifact && (
            <div className="architecture-gallery">
              <div className="architecture-gallery-head">
                <strong>{artifactLabel}</strong>
                <button className="btn btn--ghost btn--sm" type="button" onClick={() => setArtifact(null)}>
                  {t("architecture.gallery.close")}
                </button>
              </div>
              <iframe
                title={artifactLabel}
                srcDoc={artifact}
                sandbox=""
                referrerPolicy="no-referrer"
              />
            </div>
          )}
          <button
            className="btn btn--primary btn--sm architecture-approve"
            type="button"
            disabled={!proof.valid || approving}
            onClick={approve}
          >
            {approving ? t("architecture.approving") : t("architecture.approve")}
          </button>
        </>
      )}
      {(validation.error || actionError) && (
        <p className="decision-error" role="alert">
          {t("architecture.validation.error")}
        </p>
      )}
    </section>
  );
}
