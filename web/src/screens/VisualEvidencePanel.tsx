import { useEffect, useMemo, useState } from "react";
import {
  api,
  type ArchitectureVisualScenario,
  type VisualComparison,
} from "../api";
import { useAsync } from "../hooks";
import { useT } from "../i18n";
import type { VerdictPayload } from "./reviewPayload";

type ViewMode = "side_by_side" | "overlay" | "diff";

interface ArtifactUrls {
  baseline: string;
  render: string;
  heatmap: string;
}

function shortDigest(value: string | undefined) {
  return value ? `${value.slice(0, 12)}…${value.slice(-8)}` : "—";
}

function scenarioFor(
  comparison: VisualComparison,
  scenarios: ArchitectureVisualScenario[],
) {
  return scenarios.find((scenario) => scenario.comparison_id === comparison.comparison_id);
}

function unique(values: Array<string | undefined>) {
  return [...new Set(values.filter((value): value is string => Boolean(value)))];
}

function revokeOwnedUrls(urls: string[]) {
  if (typeof URL.revokeObjectURL !== "function") return;
  urls
    .filter((url) => url.startsWith("blob:"))
    .forEach((url) => URL.revokeObjectURL(url));
}

export function VisualEvidencePanel({ payload }: { payload: VerdictPayload }) {
  const { t } = useT();
  const reviewedRun = payload.reviewed_run_id ?? "";
  const applicability = payload.visual_evidence?.applicability;
  const expectedIds = useMemo(
    () => new Set(payload.visual_evidence?.references.map((item) => item.evidence_id) ?? []),
    [payload.visual_evidence],
  );
  const comparisons = useAsync(
    () => reviewedRun ? api.visualComparisons(reviewedRun) : Promise.resolve([]),
    [reviewedRun],
  );
  const relevant = useMemo(
    () => (comparisons.data ?? []).filter((item) => expectedIds.has(item.id)),
    [comparisons.data, expectedIds],
  );
  const specId = relevant[0]?.spec_version_id ?? "";
  const validation = useAsync(
    () => specId ? api.validateSpec(specId) : Promise.resolve(null),
    [specId],
  );
  const scenarios = validation.data?.scenarios ?? [];
  const [selectedId, setSelectedId] = useState("");
  const selected = relevant.find((item) => item.id === selectedId) ?? relevant[0];
  const scenario = selected ? scenarioFor(selected, scenarios) : undefined;
  const [mode, setMode] = useState<ViewMode>("side_by_side");
  const [opacity, setOpacity] = useState(50);
  const [artifacts, setArtifacts] = useState<ArtifactUrls | null>(null);
  const [artifactsLoading, setArtifactsLoading] = useState(false);
  const [artifactsError, setArtifactsError] = useState(false);

  useEffect(() => {
    if (relevant.length > 0 && !relevant.some((item) => item.id === selectedId)) {
      setSelectedId(relevant[0].id);
    }
  }, [relevant, selectedId]);

  useEffect(() => {
    let active = true;
    let owned: string[] = [];
    setArtifacts(null);
    setArtifactsError(false);
    if (!selected || selected.status === "invalid") {
      setArtifactsLoading(false);
      return () => {};
    }
    setArtifactsLoading(true);
    Promise.all([
      api.baselinePng(selected.spec_version_id, selected.comparison_id),
      api.visualRender(selected.id),
      api.visualHeatmap(selected.id),
    ])
      .then(([baseline, render, heatmap]) => {
        owned = [baseline, render, heatmap];
        if (!active) {
          revokeOwnedUrls(owned);
          return;
        }
        setArtifacts({ baseline, render, heatmap });
        setArtifactsLoading(false);
      })
      .catch(() => {
        if (active) {
          setArtifactsError(true);
          setArtifactsLoading(false);
        }
      });
    return () => {
      active = false;
      revokeOwnedUrls(owned);
    };
  }, [selected]);

  if (applicability === "not_applicable") {
    return (
      <section className="visual-evidence visual-evidence--na" aria-label={t("review.evidence.title")}>
        <h2 className="sec-title">{t("review.evidence.title")}</h2>
        <p>{t("review.evidence.not_applicable")}</p>
      </section>
    );
  }
  if (applicability !== "required" || !reviewedRun) return null;

  const locales = unique(relevant.map((item) => scenarioFor(item, scenarios)?.locale));
  const viewports = unique(relevant.map((item) => {
    const itemScenario = scenarioFor(item, scenarios);
    return itemScenario
      ? `${itemScenario.viewport_width}×${itemScenario.viewport_height}`
      : undefined;
  }));
  const chooseByScenario = (comparisonId: string) => {
    const match = relevant.find((item) => item.comparison_id === comparisonId);
    if (match) setSelectedId(match.id);
  };
  const chooseByLocale = (locale: string) => {
    const match = relevant.find((item) => scenarioFor(item, scenarios)?.locale === locale);
    if (match) setSelectedId(match.id);
  };
  const chooseByViewport = (viewport: string) => {
    const match = relevant.find((item) => {
      const candidate = scenarioFor(item, scenarios);
      return candidate && `${candidate.viewport_width}×${candidate.viewport_height}` === viewport;
    });
    if (match) setSelectedId(match.id);
  };
  const expectedCount = payload.visual_evidence?.references.length ?? 0;
  const metadataComplete = expectedCount === relevant.length;

  return (
    <section className="visual-evidence" aria-label={t("review.evidence.title")}>
      <div className="visual-evidence-head">
        <div>
          <h2 className="sec-title">{t("review.evidence.title")}</h2>
          <p>{t("review.evidence.server_owned")}</p>
        </div>
        {selected && (
          <span className={`badge visual-status visual-status--${selected.status}`}>
            {t(`review.evidence.status.${selected.status}`)}
          </span>
        )}
      </div>

      {comparisons.loading && <p role="status">{t("review.evidence.loading")}</p>}
      {(comparisons.error || validation.error || !metadataComplete) && (
        <p className="visual-evidence-error" role="alert">{t("review.evidence.error")}</p>
      )}
      {selected && (
        <>
          <div className="evidence-selectors">
            <label>
              <span>{t("review.evidence.scenario")}</span>
              <select
                value={selected.comparison_id}
                onChange={(event) => chooseByScenario(event.target.value)}
              >
                {relevant.map((item) => {
                  const itemScenario = scenarioFor(item, scenarios);
                  return (
                    <option value={item.comparison_id} key={item.id}>
                      {itemScenario
                        ? `${itemScenario.screen} · ${itemScenario.state}`
                        : item.comparison_id}
                    </option>
                  );
                })}
              </select>
            </label>
            <label>
              <span>{t("review.evidence.viewport")}</span>
              <select
                value={scenario ? `${scenario.viewport_width}×${scenario.viewport_height}` : ""}
                onChange={(event) => chooseByViewport(event.target.value)}
              >
                {viewports.map((viewport) => <option key={viewport}>{viewport}</option>)}
              </select>
            </label>
            <label>
              <span>{t("review.evidence.locale")}</span>
              <select
                value={scenario?.locale ?? ""}
                onChange={(event) => chooseByLocale(event.target.value)}
              >
                {locales.map((locale) => <option key={locale}>{locale}</option>)}
              </select>
            </label>
          </div>

          {scenario && (
            <p className="scenario-provenance">
              <strong>{scenario.screen}</strong> · {scenario.state} · {scenario.locale} · {scenario.theme} · {scenario.viewport_width}×{scenario.viewport_height} @ {(scenario.device_scale_factor_milli / 1000).toFixed(1)}x · <code>{scenario.route}</code>
            </p>
          )}

          {selected.status === "invalid" ? (
            <div className="capture-failure" role="alert">
              <strong>{selected.failure_code ?? t("review.evidence.status.invalid")}</strong>
              <span>{selected.failure_message ?? t("review.evidence.invalid_unknown")}</span>
              {selected.recovery_action && <p>{selected.recovery_action}</p>}
            </div>
          ) : (
            <>
              <div className="evidence-modes" role="group" aria-label={t("review.evidence.mode")}>
                {(["side_by_side", "overlay", "diff"] as const).map((item) => (
                  <button
                    className={mode === item ? "evidence-mode is-active" : "evidence-mode"}
                    type="button"
                    aria-pressed={mode === item}
                    onClick={() => setMode(item)}
                    key={item}
                  >
                    {t(`review.evidence.mode.${item}`)}
                  </button>
                ))}
              </div>
              {artifactsLoading && <div className="artifact-stage skel" role="status" aria-label={t("review.evidence.artifacts_loading")} />}
              {artifactsError && <p className="visual-evidence-error" role="alert">{t("review.evidence.artifacts_error")}</p>}
              {artifacts && mode === "side_by_side" && (
                <div className="artifact-side-by-side">
                  <figure>
                    <figcaption>{t("review.compare.spec")}</figcaption>
                    <img src={artifacts.baseline} alt={t("review.evidence.baseline_alt")} />
                  </figure>
                  <figure>
                    <figcaption>{t("review.compare.render")}</figcaption>
                    <img src={artifacts.render} alt={t("review.evidence.render_alt")} />
                  </figure>
                </div>
              )}
              {artifacts && mode === "overlay" && (
                <div className="artifact-overlay-wrap">
                  <div className="artifact-overlay">
                    <img src={artifacts.baseline} alt={t("review.evidence.baseline_alt")} />
                    <img
                      src={artifacts.render}
                      alt={t("review.evidence.render_alt")}
                      style={{ opacity: opacity / 100 }}
                    />
                  </div>
                  <label className="overlay-opacity">
                    <span>{t("review.evidence.opacity")}: {opacity}%</span>
                    <input
                      type="range"
                      min="0"
                      max="100"
                      value={opacity}
                      onChange={(event) => setOpacity(Number(event.target.value))}
                    />
                  </label>
                </div>
              )}
              {artifacts && mode === "diff" && (
                <figure className="artifact-heatmap">
                  <figcaption>{t("review.evidence.heatmap")}</figcaption>
                  <img src={artifacts.heatmap} alt={t("review.evidence.heatmap_alt")} />
                </figure>
              )}
            </>
          )}

          <div className="evidence-metrics" aria-label={t("review.evidence.metrics")}>
            <div><span>{t("review.evidence.pixels")}</span><strong>{(selected.pixel_ratio_micros / 10_000).toFixed(2)}%</strong><small>{selected.changed_pixels.toLocaleString()} / {selected.total_pixels.toLocaleString()}</small></div>
            <div><span>{t("review.evidence.geometry")}</span><strong>{(selected.max_geometry_delta_milli / 1000).toFixed(1)} px</strong></div>
            <div><span>{t("review.evidence.accessibility")}</span><strong>{selected.accessibility_changes}</strong></div>
          </div>

          <details className="evidence-provenance">
            <summary>{t("review.evidence.provenance")}</summary>
            <dl>
              <div><dt>{t("review.evidence.id")}</dt><dd><code>{selected.id}</code></dd></div>
              <div><dt>{t("review.evidence.run")}</dt><dd><code>{selected.run_id}</code></dd></div>
              <div><dt>{t("review.evidence.spec")}</dt><dd><code>{selected.spec_version_id}</code></dd></div>
              <div><dt>{t("review.evidence.commit")}</dt><dd><code title={selected.package_commit_sha}>{shortDigest(selected.package_commit_sha)}</code></dd></div>
              <div><dt>manifest</dt><dd><code title={selected.manifest_digest}>{shortDigest(selected.manifest_digest)}</code></dd></div>
              <div><dt>baseline</dt><dd><code title={selected.baseline_png_digest}>{shortDigest(selected.baseline_png_digest)}</code></dd></div>
              <div><dt>render</dt><dd><code title={selected.render_png_digest}>{shortDigest(selected.render_png_digest)}</code></dd></div>
              <div><dt>heatmap</dt><dd><code title={selected.heatmap_png_digest}>{shortDigest(selected.heatmap_png_digest)}</code></dd></div>
              <div><dt>{t("review.evidence.environment")}</dt><dd><code title={selected.environment_digest}>{shortDigest(selected.environment_digest)}</code></dd></div>
              <div><dt>{t("review.evidence.browser")}</dt><dd>{selected.browser_version ?? "—"}</dd></div>
              <div><dt>{t("review.evidence.fonts")}</dt><dd><code title={selected.font_fingerprint}>{shortDigest(selected.font_fingerprint)}</code></dd></div>
            </dl>
          </details>
        </>
      )}
    </section>
  );
}
