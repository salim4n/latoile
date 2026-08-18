import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { ApiError, api, type Message, type Preview, type Project, type Task } from "../api";
import { LangProvider } from "../i18n";
import { ProjectScreen } from "./ProjectScreen";

const project: Project = {
  id: "project-1",
  name: "LaToile",
  slug: "latoile",
  github_repo: "salim4n/latoile",
  work_branch: "work/latoile",
  status: "building",
  dev_command: "pnpm dev",
};

function renderProject() {
  return render(
    <LangProvider>
      <MemoryRouter initialEntries={[`/projects/${project.id}`]}>
        <Routes>
          <Route path="/projects/:id" element={<ProjectScreen />} />
        </Routes>
      </MemoryRouter>
    </LangProvider>,
  );
}

function mockProject({
  messages = [],
  preview = null,
}: {
  messages?: Message[];
  preview?: Preview | null;
} = {}) {
  vi.spyOn(api, "project").mockResolvedValue(project);
  vi.spyOn(api, "messages").mockResolvedValue(messages);
  if (preview) vi.spyOn(api, "preview").mockResolvedValue(preview);
  else vi.spyOn(api, "preview").mockRejectedValue(new ApiError(404, "not_found", "preview not found"));
}

describe("ProjectScreen visual contract", () => {
  afterEach(() => vi.restoreAllMocks());

  it("renders timestamped chat messages and structured Manager actions", async () => {
    mockProject({
      messages: [
        {
          id: "message-1",
          author: "manager",
          content: "Le plan est prêt.",
          actions: JSON.stringify([
            { title: "Architecture validée", sub: "4 tâches prêtes" },
            { title: "Run démarré", sub: "Frontend" },
          ]),
          created_at: "2026-08-17T15:20:00",
        },
      ],
    });
    renderProject();

    expect(await screen.findByText("Le plan est prêt.")).toBeTruthy();
    expect(screen.getByText(/Manager · 15:20/)).toBeTruthy();
    expect(screen.getByText("Architecture validée")).toBeTruthy();
    expect(screen.getByText("4 tâches prêtes")).toBeTruthy();
    expect(screen.getByText("Run démarré")).toBeTruthy();
    expect(screen.getByRole("link", { name: "Réglages du projet" })).toBeTruthy();
  });

  it("keeps a failed message recoverable without clearing the draft", async () => {
    mockProject();
    vi.spyOn(api, "sendMessage").mockRejectedValue(new Error("offline"));
    renderProject();

    const input = await screen.findByRole("textbox", { name: "Message au Manager…" });
    fireEvent.change(input, { target: { value: "Relance le Frontend" } });
    fireEvent.click(screen.getByRole("button", { name: "Envoyer" }));

    expect((await screen.findByRole("alert")).textContent).toContain(
      "Le message n'a pas pu être envoyé",
    );
    expect((input as HTMLInputElement).value).toBe("Relance le Frontend");
    await waitFor(() =>
      expect((screen.getByRole("button", { name: "Envoyer" }) as HTMLButtonElement).disabled).toBe(false),
    );
  });

  it("renders the four board columns with translated roles and latest runs", async () => {
    mockProject();
    const tasks: Task[] = [
      {
        id: "T-104",
        project_id: project.id,
        role_id: "architect",
        title: "Valider l'architecture",
        description: "",
        status: "ready",
        position: 0,
        latest_run_id: "42",
      },
      {
        id: "T-105",
        project_id: project.id,
        role_id: "frontend",
        title: "Construire la page d'accueil",
        description: "",
        status: "in_progress",
        position: 1,
      },
    ];
    vi.spyOn(api, "tasks").mockResolvedValue(tasks);
    renderProject();

    fireEvent.click(await screen.findByRole("tab", { name: "Board" }));
    const board = await screen.findByRole("region", { name: "Board du projet" });
    expect(within(board).getAllByRole("group")).toHaveLength(4);
    expect(within(board).getByText("Architecte")).toBeTruthy();
    expect(within(board).getByText("T-104 · run #42")).toBeTruthy();
    expect(within(board).getByText("Construire la page d'accueil")).toBeTruthy();
  });

  it("shows the live Preview badge, captured logs and both viewport formats", async () => {
    mockProject({
      preview: {
        id: "preview-1",
        project_id: project.id,
        port: 4100,
        status: "ready",
        alive: true,
        logs: ["VITE ready in 312 ms"],
      },
    });
    renderProject();

    const previewTab = await screen.findByRole("tab", { name: "Preview live" });
    fireEvent.click(previewTab);
    expect(await screen.findByTitle("Preview du projet")).toBeTruthy();
    expect(screen.getByRole("group", { name: "Format de la preview" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Desktop" }));
    expect(screen.getByTestId("preview-frame").classList.contains("phone--desktop")).toBe(true);
  });

  it("shows the captured build log when the preview process is dead", async () => {
    mockProject({
      preview: {
        id: "preview-1",
        project_id: project.id,
        port: 4100,
        status: "ready",
        alive: false,
        logs: ["error TS2322: Type mismatch", "Build failed"],
      },
    });
    renderProject();

    fireEvent.click(await screen.findByRole("tab", { name: "Preview" }));
    expect(await screen.findByRole("heading", { name: "Le build a échoué" })).toBeTruthy();
    expect(screen.getByText(/error TS2322/)).toBeTruthy();
    expect(screen.queryByTitle("Preview du projet")).toBeNull();
    expect(screen.getByRole("button", { name: "Relancer" })).toBeTruthy();
  });

  it("treats a missing preview as an actionable empty state", async () => {
    mockProject();
    vi.spyOn(api, "ensurePreview").mockRejectedValue(new Error("could not start"));
    renderProject();

    fireEvent.click(await screen.findByRole("tab", { name: "Preview" }));
    expect(await screen.findByRole("heading", { name: "Aucune preview" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Démarrer la preview" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "La preview n'a pas pu démarrer",
    );
  });

  it("does not disguise a server failure as an absent preview", async () => {
    vi.spyOn(api, "project").mockResolvedValue(project);
    vi.spyOn(api, "messages").mockResolvedValue([]);
    vi.spyOn(api, "preview").mockRejectedValue(new ApiError(500, "internal_error", "failed"));
    renderProject();

    fireEvent.click(await screen.findByRole("tab", { name: "Preview" }));
    expect(await screen.findByRole("heading", { name: "Preview indisponible" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Réessayer" })).toBeTruthy();
  });

  it("uses a project-shaped loading state before the workspace is ready", () => {
    const pending = new Promise<never>(() => {});
    vi.spyOn(api, "project").mockReturnValue(pending);
    vi.spyOn(api, "preview").mockReturnValue(pending);
    renderProject();

    expect(screen.getByRole("status", { name: "Chargement du projet" })).toBeTruthy();
    expect(screen.getAllByTestId("project-message-skeleton")).toHaveLength(3);
  });
});
