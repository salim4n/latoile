import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { api, type Approval } from "../api";
import { LangProvider } from "../i18n";
import { ReviewScreen } from "./ReviewScreen";

const richPayload = JSON.stringify({
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
  comparison: {
    spec_version: 3,
    target: {
      title: "Connexion",
      subtitle: "Accédez à votre espace",
      fields: ["Adresse e-mail", "Mot de passe"],
      cta: "Se connecter",
    },
    render: {
      title: "Connexion",
      subtitle: "Accédez à votre espace",
      fields: ["Adresse e-mail", "Mot de passe"],
      cta: "Se connecter",
    },
    expected_spacing_px: 16,
    actual_spacing_px: 8,
    gap: "L'espacement du bouton ne respecte pas la maquette.",
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
  afterEach(() => vi.restoreAllMocks());

  it("renders the verdict, localized findings, diff and mockup comparison", async () => {
    vi.spyOn(api, "approvals").mockResolvedValue([review()]);
    renderReview();

    expect(await screen.findByText("Approuver avec réserve")).toBeTruthy();
    expect(screen.getByText("Findings (2)")).toBeTruthy();
    expect(screen.getByText("web/src/components/LoginForm.tsx:34")).toBeTruthy();
    const diff = screen.getByRole("region", {
      name: "Diff du fichier web/src/components/LoginForm.tsx",
    });
    expect(within(diff).getByText("+15")).toBeTruthy();
    expect(within(diff).getByText("−4")).toBeTruthy();
    expect(screen.getByRole("figure", { name: "Maquette (spec v3)" })).toBeTruthy();
    expect(screen.getByRole("figure", { name: "Rendu (run #41)" })).toBeTruthy();
    expect(screen.getByText("L'espacement du bouton ne respecte pas la maquette.")).toBeTruthy();
    expect(screen.getByText("LaToile / Reviews / run #41")).toBeTruthy();
  });

  it("keeps the decision controls locked while the request is pending", async () => {
    vi.spyOn(api, "approvals").mockResolvedValue([review()]);
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
    vi.spyOn(api, "approvals").mockResolvedValue([review()]);
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

  it("makes a failed decision recoverable", async () => {
    vi.spyOn(api, "approvals").mockResolvedValue([review()]);
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
    vi.spyOn(api, "approvals").mockReturnValue(new Promise<never>(() => {}));
    renderReview();

    expect(screen.getByRole("status", { name: "Chargement de la review" })).toBeTruthy();
    expect(screen.getAllByTestId("review-block-skeleton")).toHaveLength(3);
  });

  it("separates an API failure from an already-decided review", async () => {
    vi.spyOn(api, "approvals").mockRejectedValue(new Error("offline"));
    renderReview();

    expect(await screen.findByRole("heading", { name: "Review indisponible" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Réessayer" })).toBeTruthy();
  });

  it("uses an actionable terminal state when the approval is no longer pending", async () => {
    vi.spyOn(api, "approvals").mockResolvedValue([]);
    renderReview();

    expect(await screen.findByRole("heading", { name: "Review déjà tranchée" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "Retour à l'Inbox" })).toBeTruthy();
  });

  it("is honest when the legacy payload only contains a summary", async () => {
    vi.spyOn(api, "approvals").mockResolvedValue([
      review({ payload: JSON.stringify({ summary: "Endpoint implémenté." }) }),
    ]);
    renderReview();

    expect(await screen.findByText("Endpoint implémenté.")).toBeTruthy();
    expect(screen.getByText(/Le Reviewer n'a pas fourni de diff/)).toBeTruthy();
  });
});
