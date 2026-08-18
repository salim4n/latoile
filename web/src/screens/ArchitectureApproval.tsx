import { useCallback, useEffect, useRef, useState } from "react";
import { api, type SpecVersion, type VisualBaseline } from "../api";
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
  const [capturing, setCapturing] = useState(false);
  const [baselines, setBaselines] = useState<VisualBaseline[] | null>(null);
  const [baselineImages, setBaselineImages] = useState<Record<string, string>>({});
  const captureStarted = useRef<string | null>(null);
  const [actionError, setActionError] = useState(false);

  const draftId = draft?.id;
  const proof = validation.data;
  const proofValid = proof?.valid === true;
  const readyKey = (baselines ?? [])
    .filter((baseline) => baseline.status === "ready")
    .map((baseline) => `${baseline.comparison_id}:${baseline.png_digest}`)
    .join("|");
  const allBaselinesReady =
    proofValid &&
    proof.scenarios.length > 0 &&
    baselines?.length === proof.scenarios.length &&
    proof.scenarios.every((scenario) =>
      (baselines ?? []).some(
        (baseline) =>
          baseline.comparison_id === scenario.comparison_id && baseline.status === "ready",
      ),
    );

  const capture = useCallback(
    async (force = false) => {
      if (!draftId || !proofValid || capturing) return;
      if (!force && captureStarted.current === draftId) return;
      captureStarted.current = draftId;
      setCapturing(true);
      setActionError(false);
      try {
        setBaselines(await api.captureBaselines(draftId));
      } catch {
        setActionError(true);
      } finally {
        setCapturing(false);
      }
    },
    [capturing, draftId, proofValid],
  );

  useEffect(() => {
    if (proofValid) void capture(false);
  }, [capture, proofValid]);

  useEffect(() => {
    if (!readyKey) {
      setBaselineImages({});
      return;
    }
    let active = true;
    const urls: string[] = [];
    const ready = (baselines ?? []).filter((baseline) => baseline.status === "ready");
    Promise.all(
      ready.map(async (baseline) => {
        if (!draftId) throw new Error("missing draft");
        const url = await api.baselinePng(draftId, baseline.comparison_id);
        urls.push(url);
        return [baseline.comparison_id, url] as const;
      }),
    )
      .then((entries) => {
        if (active) setBaselineImages(Object.fromEntries(entries));
      })
      .catch(() => {
        if (active) setActionError(true);
      });
    return () => {
      active = false;
      if (typeof URL.revokeObjectURL === "function") {
        urls.forEach((url) => URL.revokeObjectURL(url));
      }
    };
  }, [baselines, draftId, readyKey]);

  if (!draft) return null;

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
    if (!draft || !allBaselinesReady || approving) return;
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
          <section className="baseline-capture" aria-label={t("architecture.baseline.aria")}>
            <div className="architecture-validation-head">
              <strong>{t("architecture.baseline.title")}</strong>
              <span
                className={`badge ${
                  capturing
                    ? "badge--neutral"
                    : allBaselinesReady
                      ? "badge--success"
                      : "badge--danger"
                }`}
              >
                {t(
                  capturing
                    ? "architecture.baseline.capturing"
                    : allBaselinesReady
                      ? "architecture.baseline.ready"
                      : "architecture.baseline.blocked",
                )}
              </span>
            </div>
            {capturing && (
              <p className="architecture-validation-proof" role="status">
                {t("architecture.baseline.progress")}
              </p>
            )}
            <div className="baseline-grid">
              {(baselines ?? []).map((baseline) => (
                <article className={`baseline-card baseline-card--${baseline.status}`} key={baseline.comparison_id}>
                  <div className="baseline-card-head">
                    <strong>{baseline.comparison_id}</strong>
                    <span className={`badge ${baseline.status === "ready" ? "badge--success" : "badge--danger"}`}>
                      {t(
                        baseline.status === "ready"
                          ? "architecture.baseline.ready_one"
                          : "architecture.baseline.failed_one",
                      )}
                    </span>
                  </div>
                  {baseline.status === "ready" ? (
                    <>
                      {baselineImages[baseline.comparison_id] && (
                        <img
                          src={baselineImages[baseline.comparison_id]}
                          alt={`${t("architecture.baseline.image_alt")} ${baseline.comparison_id}`}
                        />
                      )}
                      <p>{baseline.browser_version}</p>
                      <code>PNG {baseline.png_digest?.slice(0, 12)}</code>
                      <code>DOM {baseline.geometry_digest?.slice(0, 12)}</code>
                      <code>AX {baseline.accessibility_digest?.slice(0, 12)}</code>
                    </>
                  ) : (
                    <>
                      <code>{baseline.failure_code}</code>
                      <p>{baseline.failure_message}</p>
                      <strong>{baseline.recovery_action}</strong>
                    </>
                  )}
                </article>
              ))}
            </div>
            {!capturing && !allBaselinesReady && proofValid && (
              <button className="btn btn--ghost btn--sm" type="button" onClick={() => void capture(true)}>
                {t("architecture.baseline.retry")}
              </button>
            )}
          </section>
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
            disabled={!allBaselinesReady || approving}
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
