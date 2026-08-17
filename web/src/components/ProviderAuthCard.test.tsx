import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { api, type AgentAuthSession } from "../api";
import { LangProvider } from "../i18n";
import { ProviderAuthCard } from "./ProviderAuthCard";

const authenticated: AgentAuthSession = {
  session_id: "session-1",
  provider: "claude",
  status: "authenticated",
  url: null,
  input_required: true,
  user_code: null,
  hint: null,
  error: null,
};

function renderCard() {
  return render(
    <LangProvider>
      <ProviderAuthCard
        provider="claude"
        label="Claude"
        status={{ authenticated: false, detail: null }}
        onChanged={() => {}}
      />
    </LangProvider>,
  );
}

function renderConnectedCard() {
  return render(
    <LangProvider>
      <ProviderAuthCard
        provider="claude"
        label="Claude"
        status={{ authenticated: true, detail: "moi@example.com" }}
        onChanged={() => {}}
      />
    </LangProvider>,
  );
}

describe("ProviderAuthCard", () => {
  afterEach(() => vi.restoreAllMocks());

  it("shows an actionable error when the login command cannot start", async () => {
    vi.spyOn(api, "agentAuthStart").mockRejectedValue(new Error("offline"));
    renderCard();

    fireEvent.click(screen.getByRole("button", { name: "Connecter Claude" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("vérifiez que le serveur et le CLI sont disponibles");
  });

  it("uses the completed session immediately and clears it after disconnect", async () => {
    vi.spyOn(api, "agentAuthStart").mockResolvedValue(authenticated);
    vi.spyOn(api, "agentAuthDisconnect").mockResolvedValue({
      authenticated: false,
      detail: null,
    });
    renderCard();

    fireEvent.click(screen.getByRole("button", { name: "Connecter Claude" }));
    expect(await screen.findByText("Connecté")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Déconnecter" }));
    fireEvent.click(screen.getAllByRole("button", { name: "Déconnecter" })[0]);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Connecter Claude" })).toBeTruthy();
    });
  });

  it("shows the connect action immediately after a successful disconnect", async () => {
    vi.spyOn(api, "agentAuthDisconnect").mockResolvedValue({
      authenticated: false,
      detail: null,
    });
    renderConnectedCard();

    fireEvent.click(screen.getByRole("button", { name: "Déconnecter" }));
    fireEvent.click(screen.getAllByRole("button", { name: "Déconnecter" })[0]);

    expect(await screen.findByRole("button", { name: "Connecter Claude" })).toBeTruthy();
  });
});
