import { fireEvent, render, screen, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { api, type Repo } from "../api";
import { LangProvider } from "../i18n";
import { NewProjectScreen } from "./NewProjectScreen";

describe("ISSUE-001 repository filtering", () => {
  afterEach(() => vi.restoreAllMocks());

  it("filters a large repository list by owner, name, or description", async () => {
    const repos: Repo[] = Array.from({ length: 100 }, (_, index) => ({
      full_name: `salim4n/archive-${index}`,
      description: "Archived experiment",
      private: true,
    }));
    repos.push({
      full_name: "westlabs/customer-portal",
      description: "Billing and customer operations",
      private: true,
    });
    vi.spyOn(api, "repos").mockResolvedValue(repos);

    render(
      <LangProvider>
        <MemoryRouter initialEntries={["/projects/new"]}>
          <Routes>
            <Route path="/projects/new" element={<NewProjectScreen />} />
          </Routes>
        </MemoryRouter>
      </LangProvider>,
    );

    const search = await screen.findByRole("searchbox", { name: "Rechercher un dépôt" });
    fireEvent.change(search, { target: { value: "billing" } });
    const picker = screen.getByRole("radiogroup", { name: "Dépôt GitHub" });
    expect(within(picker).getAllByRole("radio")).toHaveLength(1);
    expect(within(picker).getByRole("radio", { name: /westlabs\/customer-portal/ })).toBeTruthy();

    fireEvent.change(search, { target: { value: "does-not-exist" } });
    expect(within(picker).queryAllByRole("radio")).toHaveLength(0);
    expect(screen.getByText("Aucun dépôt ne correspond à cette recherche.")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Créer le projet" }) as HTMLButtonElement).disabled).toBe(true);
  });
});
