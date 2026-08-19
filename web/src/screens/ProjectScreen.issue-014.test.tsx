import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { api, type Project } from "../api";
import { LangProvider } from "../i18n";
import { ProjectScreen } from "./ProjectScreen";

const project: Project = {
  id: "project-package-english",
  name: "English package",
  slug: "english-package",
  github_repo: "salim4n/english-package",
  work_branch: "work",
  status: "draft",
  dev_command: "",
};

describe("ISSUE-014 final architecture package language", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    localStorage.clear();
  });

  it("localizes both legacy package copy and structured metadata", async () => {
    localStorage.setItem("latoile-lang", "en");
    vi.spyOn(api, "project").mockResolvedValue(project);
    vi.spyOn(api, "delivery").mockResolvedValue({ status: "not_started", work_branch: "work" });
    vi.spyOn(api, "messages").mockResolvedValue([
      {
        id: "legacy-package-message",
        author: "manager",
        content: "L'Architecte a produit un paquet confiné et vérifié. La spec attend maintenant votre validation.",
        actions: JSON.stringify([
          {
            type: "architecture_package",
            title: "Paquet architecture v1 prêt",
            sub: "design/v0001-example/ · 16 fichiers · commit 0123456789ab",
          },
        ]),
      },
    ]);
    vi.spyOn(api, "architecture").mockResolvedValue(null);
    vi.spyOn(api, "specs").mockResolvedValue([]);
    vi.spyOn(api, "preview").mockResolvedValue(null);

    render(
      <LangProvider>
        <MemoryRouter initialEntries={[`/projects/${project.id}`]}>
          <Routes>
            <Route path="/projects/:id" element={<ProjectScreen />} />
          </Routes>
        </MemoryRouter>
      </LangProvider>,
    );

    expect(await screen.findByText("The Architect produced a confined, verified package. The specification is ready for your validation.")).toBeTruthy();
    expect(screen.getByText("Architecture package ready")).toBeTruthy();
    expect(screen.getByText(/16 files/)).toBeTruthy();
    expect(screen.queryByText(/L'Architecte/)).toBeNull();
    expect(screen.queryByText(/fichiers/)).toBeNull();
  });
});
