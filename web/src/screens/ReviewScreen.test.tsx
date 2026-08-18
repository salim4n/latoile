import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import {
  api,
  ApiError,
  type Approval,
  type ArchitecturePackageValidation,
  type VisualComparison,
} from "../api";
import { LangProvider } from "../i18n";
import { ReviewScreen } from "./ReviewScreen";

const mobileEvidence: VisualComparison = {
  id: "visual:executor-40:home-fr-mobile",
  spec_version_id: "spec-3",
  project_id: "project-1",
  run_id: "executor-40",
  comparison_id: "home-default-fr-mobile",
  manifest_digest: "a".repeat(64),
  package_commit_sha: "1".repeat(40),
  baseline_png_digest: "b".repeat(64),
  status: "reservation",
  changed_pixels: 2635,
  total_pixels: 329160,
  pixel_ratio_micros: 8005,
  max_geometry_delta_milli: 4000,
  accessibility_changes: 0,
  render_png_digest: "c".repeat(64),
  pixel_diff_digest: "d".repeat(64),
  heatmap_png_digest: "e".repeat(64),
  geometry_diff_digest: "f".repeat(64),
  accessibility_diff_digest: "1".repeat(64),
  environment_digest: "2".repeat(64),
  browser_version: "Chrome/151.0.0",
  font_fingerprint: "3".repeat(64),
};

const desktopEvidence: VisualComparison = {
  ...mobileEvidence,
  id: "visual:executor-40:home-en-desktop",
  comparison_id: "home-default-en-desktop",
  status: "passed",
  changed_pixels: 0,
  total_pixels: 1_296_000,
  pixel_ratio_micros: 0,
  max_geometry_delta_milli: 0,
};

function trustedReference(evidence: VisualComparison) {
  return {
    evidence_id: evidence.id,
    comparison_id: evidence.comparison_id,
    status: evidence.status,
    manifest_digest: evidence.manifest_digest,
    baseline_png_digest: evidence.baseline_png_digest,
    render_png_digest: evidence.render_png_digest,
    pixel_diff_digest: evidence.pixel_diff_digest,
    heatmap_png_digest: evidence.heatmap_png_digest,
    geometry_diff_digest: evidence.geometry_diff_digest,
    accessibility_diff_digest: evidence.accessibility_diff_digest,
    environment_digest: evidence.environment_digest,
    changed_pixels: evidence.changed_pixels,
    total_pixels: evidence.total_pixels,
    pixel_ratio_micros: evidence.pixel_ratio_micros,
    max_geometry_delta_milli: evidence.max_geometry_delta_milli,
    accessibility_changes: evidence.accessibility_changes,
  };
}

const validation: ArchitecturePackageValidation = {
  valid: true,
  package_digest: "9".repeat(64),
  manifest_digest: "a".repeat(64),
  commit_sha: "1".repeat(40),
  tree_sha: "4".repeat(40),
  file_count: 16,
  gallery_path: "gallery.html",
  findings: [],
  scenarios: [
    {
      comparison_id: mobileEvidence.comparison_id,
      screen: "login",
      state: "default",
      locale: "fr-FR",
      theme: "light",
      route: "/login",
      fixture: "review-user",
      readiness_selector: "main[data-ready]",
      stable_selectors: ["main"],
      allowed_masks: [],
      viewport_width: 390,
      viewport_height: 844,
      device_scale_factor_milli: 1000,
      mockup: "mockups/login-fr-mobile.html",
    },
    {
      comparison_id: desktopEvidence.comparison_id,
      screen: "login",
      state: "default",
      locale: "en-US",
      theme: "light",
      route: "/login",
      fixture: "review-user",
      readiness_selector: "main[data-ready]",
      stable_selectors: ["main"],
      allowed_masks: [],
      viewport_width: 1440,
      viewport_height: 900,
      device_scale_factor_milli: 1000,
      mockup: "mockups/login-en-desktop.html",
    },
  ],
};

const richPayload = JSON.stringify({
  schema_version: 2,
  reviewed_run_id: "executor-40",
  verdict: "approve_with_reservations",
  summary:
    "La page de connexion est fonctionnelle et conforme à la spec v3 dans l'ensemble.",
  findings: [
    {
      severity: "reservation",
      text: "Espacement du bouton : 8 px mesurés au lieu de 16 px.",
      location: "web/src/components/LoginForm.tsx:34",
    },
    {
      severity: "reservation",
      text: "Le bouton n'a pas d'état de chargement.",
      location: "web/src/components/LoginForm.tsx:36",
    },
  ],
  suggested_follow_ups: ["Ajouter un test de soumission pendant le chargement."],
  diff: {
    file: "web/src/components/LoginForm.tsx",
    additions: 15,
    deletions: 4,
    lines: [
      " import { useState } from \"react\";",
      "-async function onSubmit(e: React.FormEvent) {",
      "+async function onSubmit(e: React.FormEvent<HTMLFormElement>) {",
    ],
  },
  visual_evidence: {
    applicability: "required",
    references: [trustedReference(mobileEvidence), trustedReference(desktopEvidence)],
  },
  gate: {
    trusted_v2: true,
    approvable: true,
    code: "trusted",
    message: "Le verdict V2 est lié aux preuves serveur exactes et peut être décidé.",
  },
});

function review(partial: Partial<Approval> = {}): Approval {
  return {
    id: "approval-41",
    run_id: "41",
    kind: "review",
    status: "pending",
    payload: richPayload,
    project_id: "project-1",
    project_name: "LaToile",
    task_title: "Page de connexion",
    role_id: "frontend",
    created_at: new Date(Date.now() - 12 * 60_000).toISOString(),
    ...partial,
  };
}

function renderReview() {
  return render(
    <LangProvider>
      <MemoryRouter initialEntries={["/reviews/approval-41"]}>
        <Routes>
          <Route path="/reviews/:approvalId" element={<ReviewScreen />} />
        </Routes>
      </MemoryRouter>
    </LangProvider>,
  );
}

describe("ReviewScreen visual contract", () => {
  beforeEach(() => {
    vi.spyOn(api, "visualComparisons").mockResolvedValue([mobileEvidence, desktopEvidence]);
    vi.spyOn(api, "validateSpec").mockResolvedValue(validation);
    vi.spyOn(api, "baselinePng").mockImplementation(
      async (_spec, comparison) => `baseline://${comparison}`,
    );
    vi.spyOn(api, "visualRender").mockImplementation(async (id) => `render://${id}`);
    vi.spyOn(api, "visualHeatmap").mockImplementation(async (id) => `heatmap://${id}`);
  });
  afterEach(() => vi.restoreAllMocks());

  it("renders real authenticated artifacts, metrics and immutable provenance", async () => {
    vi.spyOn(api, "approval").mockResolvedValue(review());
    renderReview();

    expect(await screen.findByText("Approuver avec réserve")).toBeTruthy();
    expect(screen.getByText("Findings (2)")).toBeTruthy();
    expect(screen.getByText("web/src/components/LoginForm.tsx:34")).toBeTruthy();
    expect(screen.getByText("Suites suggérées (1)")).toBeTruthy();
    expect(screen.getByText("Ajouter un test de soumission pendant le chargement.")).toBeTruthy();
    const diff = screen.getByRole("region", {
      name: "Diff du fichier web/src/components/LoginForm.tsx",
    });
    expect(within(diff).getByText("+15")).toBeTruthy();
    expect(within(diff).getByText("−4")).toBeTruthy();
    expect(await screen.findByAltText("Baseline navigateur approuvée")).toBeTruthy();
    expect(screen.getByAltText("Rendu navigateur du run executor")).toBeTruthy();
    expect(screen.getByText("0.80%")).toBeTruthy();
    expect(screen.getByText("4.0 px")).toBeTruthy();
    expect(screen.getByText((_, element) =>
      element?.classList.contains("scenario-provenance") === true
      && element.textContent?.includes("login · default · fr-FR · light · 390×844 @ 1.0x · /login") === true,
    )).toBeTruthy();
    expect(screen.getByText("Preuves fiables — approbation autorisée")).toBeTruthy();
    expect(screen.getByText("LaToile / Reviews / run #41")).toBeTruthy();
  });

  it("switches scenario, viewport, locale, overlay and heatmap with keyboard controls", async () => {
    vi.spyOn(api, "approval").mockResolvedValue(review());
    renderReview();

    await screen.findByAltText("Baseline navigateur approuvée");
    fireEvent.change(screen.getByLabelText("Locale"), { target: { value: "en-US" } });
    await waitFor(() => expect(api.visualRender).toHaveBeenCalledWith(desktopEvidence.id));
    expect((screen.getByLabelText("Viewport") as HTMLSelectElement).value).toBe("1440×900");

    fireEvent.click(screen.getByRole("button", { name: "Superposition" }));
    const opacity = await screen.findByRole("slider", { name: /Opacité du rendu/ });
    fireEvent.change(opacity, { target: { value: "70" } });
    expect((opacity as HTMLInputElement).value).toBe("70");

    fireEvent.click(screen.getByRole("button", { name: "Diff / heatmap" }));
    expect(await screen.findByAltText("Heatmap réelle de la comparaison visuelle")).toBeTruthy();
  });

  it("keeps the decision controls locked while the request is pending", async () => {
    vi.spyOn(api, "approval").mockResolvedValue(review());
    vi.spyOn(api, "decide").mockReturnValue(new Promise<never>(() => {}));
    renderReview();

    fireEvent.click(await screen.findByRole("button", { name: "Approuver" }));

    expect(await screen.findByRole("button", { name: "Décision en cours…" })).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: "Demander des changements" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it("shows an approved terminal state without claiming a merge", async () => {
    vi.spyOn(api, "approval").mockResolvedValue(review());
    vi.spyOn(api, "decide").mockResolvedValue(review({ status: "granted" }));
    renderReview();

    fireEvent.click(await screen.findByRole("button", { name: "Approuver" }));

    expect(await screen.findByText("Approuvé")).toBeTruthy();
    expect(screen.getByText("Run #41 approuvé par vous.")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Approuver" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    expect(
      (screen.getByRole("button", { name: "Demander des changements" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it("requires a comment for changes and shows the corrective audit", async () => {
    vi.spyOn(api, "approval").mockResolvedValue(review());
    const decided = review({
      status: "rejected",
      decision_comment: "Corriger le focus clavier et ajouter le test.",
      corrective_run_id: "correction-9",
    });
    const decide = vi.spyOn(api, "decide").mockResolvedValue(decided);
    renderReview();

    const changes = await screen.findByRole("button", { name: "Demander des changements" });
    expect((changes as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByRole("textbox", { name: "Commentaire de décision" }), {
      target: { value: "Corriger le focus clavier et ajouter le test." },
    });
    expect((changes as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(changes);

    expect(decide).toHaveBeenCalledWith(
      "approval-41",
      false,
      "Corriger le focus clavier et ajouter le test.",
    );
    expect(await screen.findByText("Historique de décision")).toBeTruthy();
    expect(screen.getByText("Corriger le focus clavier et ajouter le test.")).toBeTruthy();
    expect(screen.getByText(/Run correctif démarré : #correction-9/)).toBeTruthy();
  });

  it("keeps a decided review readable after a reload", async () => {
    vi.spyOn(api, "approval").mockResolvedValue(review({
      status: "rejected",
      decision_comment: "Reprendre le contraste.",
      corrective_run_id: "correction-10",
    }));
    renderReview();

    expect(await screen.findByText("ÉTAT : CHANGEMENTS DEMANDÉS")).toBeTruthy();
    expect(screen.getByText("Reprendre le contraste.")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Approuver" }) as HTMLButtonElement).disabled)
      .toBe(true);
  });

  it("makes a failed decision recoverable", async () => {
    vi.spyOn(api, "approval").mockResolvedValue(review());
    vi.spyOn(api, "decide").mockRejectedValue(new Error("offline"));
    renderReview();

    fireEvent.click(await screen.findByRole("button", { name: "Approuver" }));

    expect((await screen.findByRole("alert")).textContent).toContain(
      "La décision n'a pas pu être enregistrée",
    );
    await waitFor(() =>
      expect((screen.getByRole("button", { name: "Approuver" }) as HTMLButtonElement).disabled).toBe(
        false,
      ),
    );
  });

  it("uses shaped skeletons while loading", () => {
    vi.spyOn(api, "approval").mockReturnValue(new Promise<never>(() => {}));
    renderReview();

    expect(screen.getByRole("status", { name: "Chargement de la review" })).toBeTruthy();
    expect(screen.getAllByTestId("review-block-skeleton")).toHaveLength(3);
  });

  it("separates an API failure from an already-decided review", async () => {
    vi.spyOn(api, "approval").mockRejectedValue(new Error("offline"));
    renderReview();

    expect(await screen.findByRole("heading", { name: "Review indisponible" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Réessayer" })).toBeTruthy();
  });

  it("uses an actionable terminal state when the approval is no longer pending", async () => {
    vi.spyOn(api, "approval").mockRejectedValue(
      new ApiError(404, "not_found", "approval"),
    );
    renderReview();

    expect(await screen.findByRole("heading", { name: "Review déjà tranchée" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "Retour à l'Inbox" })).toBeTruthy();
  });

  it("is honest when the legacy payload only contains a summary", async () => {
    vi.spyOn(api, "approval").mockResolvedValue(
      review({ payload: JSON.stringify({ summary: "Endpoint implémenté." }) }),
    );
    renderReview();

    expect(await screen.findByText("Endpoint implémenté.")).toBeTruthy();
    expect(screen.getByText(/Le Reviewer n'a pas fourni de diff/)).toBeTruthy();
    expect(screen.getByText("Review historique non fiable")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Approuver" }) as HTMLButtonElement).disabled)
      .toBe(true);
  });

  it("disables approval when the trusted V2 gate reports blocking evidence", async () => {
    const blocked = JSON.parse(richPayload);
    blocked.verdict = "changes_requested";
    blocked.gate = {
      trusted_v2: true,
      approvable: false,
      code: "visual_evidence_blocking",
      message: "Au moins une comparaison visuelle dépasse un seuil bloquant.",
    };
    blocked.visual_evidence.references[0].status = "blocking";
    vi.spyOn(api, "approval").mockResolvedValue(review({ payload: JSON.stringify(blocked) }));
    renderReview();

    expect(await screen.findByText("Approbation bloquée")).toBeTruthy();
    expect(screen.getByText("visual_evidence_blocking")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Approuver" }) as HTMLButtonElement).disabled)
      .toBe(true);
    expect(screen.getByRole("button", { name: "Demander des changements" })).toBeTruthy();
  });

  it("explains an invalid capture and its recovery without inventing images", async () => {
    const invalidEvidence: VisualComparison = {
      ...mobileEvidence,
      status: "invalid",
      changed_pixels: 0,
      total_pixels: 0,
      pixel_ratio_micros: 0,
      render_png_digest: undefined,
      pixel_diff_digest: undefined,
      heatmap_png_digest: undefined,
      geometry_diff_digest: undefined,
      accessibility_diff_digest: undefined,
      environment_digest: undefined,
      browser_version: undefined,
      font_fingerprint: undefined,
      failure_code: "readiness_timeout",
      failure_message: "main[data-ready] absent après 10 s",
      recovery_action: "Rendre le marqueur de readiness déterministe puis relancer.",
    };
    vi.mocked(api.visualComparisons).mockResolvedValue([invalidEvidence]);
    vi.spyOn(api, "approval").mockResolvedValue(review({
      payload: JSON.stringify({
        ...JSON.parse(richPayload),
        visual_evidence: {
          applicability: "required",
          references: [trustedReference(invalidEvidence)],
        },
        gate: {
          trusted_v2: true,
          approvable: false,
          code: "invalid_visual_evidence",
          message: "La capture visuelle est invalide.",
        },
      }),
    }));
    renderReview();

    expect(await screen.findByText("readiness_timeout")).toBeTruthy();
    expect(screen.getByText("main[data-ready] absent après 10 s")).toBeTruthy();
    expect(screen.getByText("Rendre le marqueur de readiness déterministe puis relancer.")).toBeTruthy();
    expect(api.visualRender).not.toHaveBeenCalled();
    expect((screen.getByRole("button", { name: "Approuver" }) as HTMLButtonElement).disabled)
      .toBe(true);
  });
});
