import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { api, type Project, type Repo } from "../api";
import { LangProvider } from "../i18n";
import { NewProjectScreen } from "./NewProjectScreen";

const repos: Repo[] = [
  {
    full_name: "salim4n/latoile",
    description: "Workbench de gestion de projet AI-native",
    private: true,
  },
  {
    full_name: "westlabs/portail-client-facturation-2026",
    description: "Refonte complète du portail client avec paiements et relances",
    private: true,
  },
];

const project: Project = {
  id: "project-1",
  name: "latoile",
  slug: "latoile",
  github_repo: "salim4n/latoile",
  work_branch: "work",
  status: "draft",
  dev_command: "pnpm dev --port $PORT",
};

function renderNewProject() {
  return render(
    <LangProvider>
      <MemoryRouter initialEntries={["/projects/new"]}>
        <Routes>
          <Route path="/projects/new" element={<NewProjectScreen />} />
          <Route path="/projects/:id" element={<p>Project created</p>} />
        </Routes>
      </MemoryRouter>
    </LangProvider>,
  );
}

describe("NewProjectScreen visual contract", () => {
  afterEach(() => vi.restoreAllMocks());

  it("defaults to the first repository and renders visibility plus the connect row", async () => {
    vi.spyOn(api, "repos").mockResolvedValue(repos);
    renderNewProject();

    const picker = await screen.findByRole("radiogroup", { name: "Dépôt GitHub" });
    const first = within(picker).getByRole("radio", { name: /salim4n\/latoile/ }) as HTMLInputElement;
    expect(first.checked).toBe(true);
    expect(within(picker).getAllByText("Privé")).toHaveLength(2);
    expect(screen.getByRole("link", { name: "Connecter un autre dépôt GitHub…" })).toBeTruthy();
  });

  it("uses an actionable empty state when GitHub has no repositories", async () => {
    vi.spyOn(api, "repos").mockResolvedValue([]);
    renderNewProject();

    expect(await screen.findByRole("heading", { name: "Aucun dépôt accessible" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "Connecter GitHub" })).toBeTruthy();
  });

  it("keeps repository-shaped skeletons while loading", () => {
    vi.spyOn(api, "repos").mockReturnValue(new Promise<never>(() => {}));
    renderNewProject();

    expect(screen.getByRole("status", { name: "Chargement des dépôts GitHub" })).toBeTruthy();
    expect(screen.getAllByTestId("repo-skeleton")).toHaveLength(4);
  });

  it("validates an empty brief inline on submit", async () => {
    vi.spyOn(api, "repos").mockResolvedValue(repos);
    renderNewProject();

    fireEvent.click(await screen.findByRole("button", { name: "Créer le projet" }));

    expect((await screen.findByRole("alert")).textContent).toContain("Décrivez le projet");
    expect(screen.getByRole("textbox", { name: "Brief initial" })).toBeTruthy();
  });

  it("locks the fields and shows the sending state while creation is pending", async () => {
    vi.spyOn(api, "repos").mockResolvedValue(repos);
    vi.spyOn(api, "createProject").mockReturnValue(new Promise<never>(() => {}));
    renderNewProject();

    const brief = await screen.findByRole("textbox", { name: "Brief initial" });
    fireEvent.change(brief, { target: { value: "Construire une app mobile-first." } });
    fireEvent.click(screen.getByRole("button", { name: "Créer le projet" }));

    expect(await screen.findByRole("button", { name: "Création en cours…" })).toBeTruthy();
    expect((brief as HTMLTextAreaElement).disabled).toBe(true);
    expect((screen.getByRole("radio", { name: /salim4n\/latoile/ }) as HTMLInputElement).disabled).toBe(true);
    expect(screen.getByText("Le Manager prépare le dépôt et la première planification.")).toBeTruthy();
  });

  it("preserves the form and makes a failed creation recoverable", async () => {
    vi.spyOn(api, "repos").mockResolvedValue(repos);
    vi.spyOn(api, "createProject").mockRejectedValue(new Error("offline"));
    renderNewProject();

    const brief = await screen.findByRole("textbox", { name: "Brief initial" });
    fireEvent.change(brief, { target: { value: "Construire le portail client." } });
    fireEvent.click(screen.getByRole("button", { name: "Créer le projet" }));

    expect((await screen.findByRole("alert")).textContent).toContain("La création a échoué");
    expect((brief as HTMLTextAreaElement).value).toBe("Construire le portail client.");
    await waitFor(() =>
      expect((screen.getByRole("button", { name: "Créer le projet" }) as HTMLButtonElement).disabled).toBe(false),
    );
  });

  it("creates from the selected repository, sends the brief, then opens the workspace", async () => {
    vi.spyOn(api, "repos").mockResolvedValue(repos);
    const create = vi.spyOn(api, "createProject").mockResolvedValue(project);
    const send = vi.spyOn(api, "sendMessage").mockResolvedValue({ message: {
      id: "message-1", author: "user", content: "Construire le portail client.", actions: null,
    }, reply: null });
    renderNewProject();

    fireEvent.click(await screen.findByRole("radio", { name: /westlabs\/portail/ }));
    fireEvent.change(screen.getByRole("textbox", { name: "Brief initial" }), {
      target: { value: "Construire le portail client." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Créer le projet" }));

    expect(await screen.findByText("Project created")).toBeTruthy();
    expect(create).toHaveBeenCalledWith(expect.objectContaining({
      name: "portail-client-facturation-2026",
      slug: "portail-client-facturation-2026",
      github_repo: "westlabs/portail-client-facturation-2026",
    }));
    expect(create.mock.calls[0][0]).not.toHaveProperty("local_path");
    expect(create.mock.calls[0][0]).not.toHaveProperty("dev_command");
    expect(send).toHaveBeenCalledWith(project.id, "Construire le portail client.");
  });

  it("sends an explicit preview command only when the owner provides one", async () => {
    vi.spyOn(api, "repos").mockResolvedValue(repos);
    const create = vi.spyOn(api, "createProject").mockResolvedValue(project);
    vi.spyOn(api, "sendMessage").mockResolvedValue({ message: {
      id: "message-1", author: "user", content: "Construire.", actions: null,
    }, reply: null });
    renderNewProject();

    fireEvent.change(await screen.findByRole("textbox", { name: "Brief initial" }), {
      target: { value: "Construire." },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Commande de preview (optionnelle)" }), {
      target: { value: "make dev PORT=$PORT" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Créer le projet" }));

    await screen.findByText("Project created");
    expect(create).toHaveBeenCalledWith(expect.objectContaining({
      dev_command: "make dev PORT=$PORT",
    }));
  });
});
