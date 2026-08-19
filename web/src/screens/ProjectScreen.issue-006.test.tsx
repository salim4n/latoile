import { render, screen, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { ApiError, api, type ArchitectureSession, type Project } from "../api";
import { LangProvider } from "../i18n";
import { ProjectScreen } from "./ProjectScreen";

const project: Project = {
  id: "project-cancelled",
  name: "Cancelled discovery",
  slug: "cancelled-discovery",
  github_repo: "salim4n/cancelled-discovery",
  work_branch: "work",
  status: "draft",
  dev_command: "",
};

describe("ISSUE-006 cancelled architecture state", () => {
  afterEach(() => vi.restoreAllMocks());

  it("keeps the unanswered question in history without presenting an active decision", async () => {
    const architecture: ArchitectureSession = {
      id: "architecture-cancelled",
      project_id: project.id,
      status: "cancelled",
      phase: "requirements",
      package_status: "not_started",
      questions: [
        {
          id: "question-open",
          sequence: 1,
          prompt: "Which mockup is authoritative?",
          status: "open",
        },
      ],
    };
    vi.spyOn(api, "project").mockResolvedValue(project);
    vi.spyOn(api, "delivery").mockResolvedValue({ status: "not_started", work_branch: "work" });
    vi.spyOn(api, "messages").mockResolvedValue([]);
    vi.spyOn(api, "architecture").mockResolvedValue(architecture);
    vi.spyOn(api, "specs").mockResolvedValue([]);
    vi.spyOn(api, "preview").mockRejectedValue(new ApiError(404, "not_found", "missing"));

    render(
      <LangProvider>
        <MemoryRouter initialEntries={[`/projects/${project.id}`]}>
          <Routes>
            <Route path="/projects/:id" element={<ProjectScreen />} />
          </Routes>
        </MemoryRouter>
      </LangProvider>,
    );

    const panel = await screen.findByRole("region", { name: "Découverte avec l'Architecte" });
    expect(within(panel).getByText("Session annulée")).toBeTruthy();
    expect(within(panel).queryByText("Décision attendue")).toBeNull();
    expect(within(panel).getAllByText("Which mockup is authoritative?")).toHaveLength(1);
  });
});
