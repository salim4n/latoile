import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { api, type Project } from "../api";
import { LangProvider } from "../i18n";
import { ProjectScreen } from "./ProjectScreen";

const project: Project = {
  id: "project-english",
  name: "English workflow",
  slug: "english-workflow",
  github_repo: "salim4n/english-workflow",
  work_branch: "work",
  status: "draft",
  dev_command: "",
};

describe("ISSUE-003 architecture message language", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    localStorage.clear();
  });

  it("renders the Architect text and localizes the structured label", async () => {
    localStorage.setItem("latoile-lang", "en");
    vi.spyOn(api, "project").mockResolvedValue(project);
    vi.spyOn(api, "delivery").mockResolvedValue({ status: "not_started", work_branch: "work" });
    vi.spyOn(api, "messages").mockResolvedValue([
      {
        id: "legacy-message",
        author: "manager",
        content: "L'Architecte a besoin de votre décision avant de poursuivre.",
        actions: JSON.stringify([
          {
            type: "architecture",
            title: "Question de l'Architecte",
            sub: "Who must approve the reference mockup?",
            status: "awaiting_answer",
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

    expect(await screen.findByText("Who must approve the reference mockup?")).toBeTruthy();
    expect(screen.getByText("Architect question")).toBeTruthy();
    expect(screen.queryByText("L'Architecte a besoin de votre décision avant de poursuivre.")).toBeNull();
    expect(screen.queryByText("Question de l'Architecte")).toBeNull();
  });
});
