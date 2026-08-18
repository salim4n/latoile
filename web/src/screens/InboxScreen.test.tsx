import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { api, type Approval, type Project } from "../api";
import { LangProvider } from "../i18n";
import { InboxScreen } from "./InboxScreen";

const project: Project = {
  id: "project-1",
  name: "LaToile",
  slug: "latoile",
  github_repo: "salim4n/latoile",
  work_branch: "work/latoile",
  status: "building",
  dev_command: "pnpm dev",
};

const approvalContext = {
  project_id: project.id,
  project_name: project.name,
  task_title: "Page de connexion",
  role_id: "frontend",
  created_at: "2026-08-17T15:20:00.000Z",
};

function approval(
  partial: Partial<Approval> & Pick<Approval, "id" | "kind">,
): Approval {
  return {
    run_id: `run-${partial.id}`,
    status: "pending",
    payload: "{}",
    ...approvalContext,
    ...partial,
  } as Approval;
}

function mockInbox({
  approvals = [],
  projects = [],
}: {
  approvals?: Approval[];
  projects?: Project[];
} = {}) {
  vi.spyOn(api, "approvals").mockResolvedValue(approvals);
  vi.spyOn(api, "projects").mockResolvedValue(projects);
  vi.spyOn(api, "agentAuthStatusAll").mockResolvedValue({
    claude: { authenticated: true, detail: "Claude CLI" },
    codex: { authenticated: false, detail: null },
  });
}

function renderInbox() {
  return render(
    <LangProvider>
      <MemoryRouter initialEntries={["/"]}>
        <InboxScreen />
      </MemoryRouter>
    </LangProvider>,
  );
}

describe("InboxScreen visual contract", () => {
  afterEach(() => vi.restoreAllMocks());

  it("renders decision context, exact commands and active project status", async () => {
    mockInbox({
      approvals: [
        approval({
          id: "review-1",
          kind: "review",
          payload: JSON.stringify({
            summary: "Review : page de connexion",
            verdict: "changes_requested",
          }),
        }),
        approval({
          id: "permission-1",
          kind: "permission",
          role_id: "backend",
          task_title: "Backend bloqué",
          payload: JSON.stringify({ command: "npm install tailwindcss" }),
        } as Partial<Approval> & Pick<Approval, "id" | "kind">),
      ],
      projects: [project],
    });
    renderInbox();

    expect(await screen.findByRole("heading", { name: /Approbations en attente/ })).toBeTruthy();
    expect(screen.getByRole("banner").querySelector(".title")).toBeNull();
    expect(screen.getByText("Review : page de connexion")).toBeTruthy();
    expect(screen.getByText(/LaToile · Frontend · run run-review-1/)).toBeTruthy();
    expect(screen.getByText("Changements demandés")).toBeTruthy();
    expect(screen.getByText("npm install tailwindcss")).toBeTruthy();
    expect(screen.getByRole("heading", { name: /Runs bloqués/ })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "LaToile" })).toBeTruthy();
    expect(screen.getByText("En cours")).toBeTruthy();
  });

  it("keeps spec approvals actionable instead of dropping them", async () => {
    mockInbox({
      approvals: [
        approval({
          id: "spec-1",
          kind: "spec",
          task_title: "Spécification v1",
          role_id: "architect",
          payload: JSON.stringify({ summary: "Architecture et maquettes prêtes" }),
        } as Partial<Approval> & Pick<Approval, "id" | "kind">),
      ],
    });
    const decide = vi.spyOn(api, "decide").mockResolvedValue(
      approval({ id: "spec-1", kind: "spec", status: "granted" }),
    );
    renderInbox();

    const card = await screen.findByRole("article", { name: "Spécification v1" });
    expect(within(card).getByText("Architecture et maquettes prêtes")).toBeTruthy();
    fireEvent.click(within(card).getByRole("button", { name: "Approuver la spec" }));

    await waitFor(() => expect(decide).toHaveBeenCalledWith("spec-1", true));
  });

  it("makes a failed permission decision recoverable in place", async () => {
    mockInbox({
      approvals: [
        approval({
          id: "permission-1",
          kind: "permission",
          payload: JSON.stringify({ command: "pnpm add zod" }),
        }),
      ],
    });
    vi.spyOn(api, "decide").mockRejectedValue(new Error("offline"));
    renderInbox();

    fireEvent.click(await screen.findByRole("button", { name: "Autoriser" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("La décision n'a pas pu être enregistrée");
    expect(screen.getByRole("button", { name: "Autoriser" })).toBeTruthy();
  });

  it("uses the actionable all-clear state when nothing needs attention", async () => {
    mockInbox();
    renderInbox();

    expect(await screen.findByRole("heading", { name: "Aucune décision en attente" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "Voir les projets" })).toBeTruthy();
  });

  it("keeps card-shaped skeletons while decisions are loading", () => {
    const pending = new Promise<never>(() => {});
    vi.spyOn(api, "approvals").mockReturnValue(pending);
    vi.spyOn(api, "projects").mockReturnValue(pending);
    vi.spyOn(api, "agentAuthStatusAll").mockReturnValue(pending);
    renderInbox();

    expect(screen.getByRole("status", { name: "Chargement de l'Inbox" })).toBeTruthy();
    expect(screen.getAllByTestId("inbox-approval-skeleton")).toHaveLength(2);
    expect(screen.getAllByTestId("inbox-project-skeleton")).toHaveLength(3);
  });
});
