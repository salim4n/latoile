import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { api, type ProviderStatus, type Role, type Routing } from "../api";
import { LangProvider } from "../i18n";
import { SettingsScreen } from "./SettingsScreen";

const roles: Role[] = [
  { id: "manager", label: "Manager" },
  { id: "architect", label: "Architecte" },
  { id: "backend", label: "Backend" },
  { id: "frontend", label: "Frontend" },
  { id: "reviewer", label: "Reviewer" },
];

const routing: Routing = {
  manager: "claude",
  architect: "claude",
  backend: "codex",
  frontend: "codex",
  reviewer: "claude",
};

function mockLoadedStatus(claude: ProviderStatus, codex: ProviderStatus) {
  vi.spyOn(api, "agentAuthStatusAll").mockResolvedValue({ claude, codex });
  vi.spyOn(api, "roles").mockResolvedValue(roles);
  vi.spyOn(api, "getRouting").mockResolvedValue(routing);
}

function renderSettings() {
  return render(
    <LangProvider>
      <MemoryRouter initialEntries={["/settings"]}>
        <SettingsScreen />
      </MemoryRouter>
    </LangProvider>,
  );
}

describe("SettingsScreen visual contract", () => {
  afterEach(() => vi.restoreAllMocks());

  it("renders the five role missions and persists a changed provider", async () => {
    mockLoadedStatus(
      { authenticated: true, detail: "Claude CLI · compte salim4n" },
      { authenticated: false, detail: null },
    );
    const save = vi.spyOn(api, "putRouting").mockResolvedValue(routing);
    renderSettings();

    expect(await screen.findByRole("heading", { name: "Connexions" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Équipe IA" })).toBeTruthy();
    expect(screen.getAllByTestId("role-routing")).toHaveLength(5);
    expect(screen.getByText(/produit la spécification ainsi que les maquettes HTML/)).toBeTruthy();

    const backend = screen.getByRole("group", { name: "Provider du Backend" });
    fireEvent.click(within(backend).getByRole("button", { name: "Claude" }));
    fireEvent.click(screen.getByRole("button", { name: "Enregistrer la configuration" }));

    await waitFor(() => {
      expect(save).toHaveBeenCalledWith({ ...routing, backend: "claude" });
    });
    expect(await screen.findByText("Configuration enregistrée")).toBeTruthy();
  });

  it("turns a fully disconnected account into an actionable empty state", async () => {
    mockLoadedStatus(
      { authenticated: false, detail: null },
      { authenticated: false, detail: null },
    );
    renderSettings();

    expect(await screen.findByRole("heading", { name: "Connectez votre premier agent" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Connecter Claude" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Connecter Codex" })).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "Équipe IA" })).toBeNull();
  });

  it("explains how to recover when the settings contract cannot load", async () => {
    vi.spyOn(api, "agentAuthStatusAll").mockRejectedValue(new Error("cli unavailable"));
    vi.spyOn(api, "roles").mockResolvedValue(roles);
    vi.spyOn(api, "getRouting").mockResolvedValue(routing);
    renderSettings();

    expect(await screen.findByRole("heading", { name: "Impossible de charger les réglages" })).toBeTruthy();
    expect(screen.getByText(/CLI Claude et Codex n'ont pas pu être interrogés/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Réessayer" })).toBeTruthy();
  });

  it("uses settings-shaped skeletons while the contract is loading", () => {
    const pending = new Promise<never>(() => {});
    vi.spyOn(api, "agentAuthStatusAll").mockReturnValue(pending);
    vi.spyOn(api, "roles").mockReturnValue(pending);
    vi.spyOn(api, "getRouting").mockReturnValue(pending);
    renderSettings();

    expect(screen.getByRole("status", { name: "Chargement des réglages" })).toBeTruthy();
    expect(screen.getAllByTestId("provider-skeleton")).toHaveLength(2);
    expect(screen.getAllByTestId("role-skeleton")).toHaveLength(5);
  });

  it("keeps a failed save recoverable in place", async () => {
    mockLoadedStatus(
      { authenticated: true, detail: "Claude CLI · compte salim4n" },
      { authenticated: false, detail: null },
    );
    vi.spyOn(api, "putRouting").mockRejectedValue(new Error("write failed"));
    renderSettings();

    fireEvent.click(await screen.findByRole("button", { name: "Enregistrer la configuration" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("L'enregistrement a échoué");
    expect(screen.getByRole("button", { name: "Enregistrer la configuration" })).toBeTruthy();
  });
});
