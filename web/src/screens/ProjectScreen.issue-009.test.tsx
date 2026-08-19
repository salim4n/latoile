import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { ApiError, api, type ArchitectureSession, type Project } from "../api";
import { LangProvider } from "../i18n";
import { ProjectScreen } from "./ProjectScreen";

const project: Project = {
  id: "project-package-failure",
  name: "Package failure",
  slug: "package-failure",
  github_repo: "salim4n/package-failure",
  work_branch: "work",
  status: "draft",
  dev_command: "",
};

const generating: ArchitectureSession = {
  id: "architecture-package-failure",
  project_id: project.id,
  status: "ready_to_draft",
  phase: "ready_to_draft",
  package_status: "generating",
  questions: [],
};

const failed: ArchitectureSession = {
  ...generating,
  status: "failed",
  package_status: "not_started",
  failure_reason: "architecture package rejected: forbidden external content",
};

describe("ISSUE-009 architecture package failure state", () => {
  afterEach(() => vi.restoreAllMocks());

  it("refreshes a rejected generation and shows its terminal reason", async () => {
    vi.spyOn(api, "project").mockResolvedValue(project);
    vi.spyOn(api, "delivery").mockResolvedValue({ status: "not_started", work_branch: "work" });
    vi.spyOn(api, "messages").mockResolvedValue([]);
    vi.spyOn(api, "specs").mockResolvedValue([]);
    vi.spyOn(api, "preview").mockRejectedValue(new ApiError(404, "not_found", "missing"));
    const architecture = vi
      .spyOn(api, "architecture")
      .mockResolvedValueOnce(generating)
      .mockResolvedValue(failed);
    vi.spyOn(api, "sendMessage").mockRejectedValue(
      new ApiError(500, "internal_error", "something went wrong"),
    );

    render(
      <LangProvider>
        <MemoryRouter initialEntries={[`/projects/${project.id}`]}>
          <Routes>
            <Route path="/projects/:id" element={<ProjectScreen />} />
          </Routes>
        </MemoryRouter>
      </LangProvider>,
    );

    expect(await screen.findByText("Génération confinée")).toBeTruthy();
    fireEvent.change(screen.getByRole("textbox", { name: "Message au Manager…" }), {
      target: { value: "Generate the verified package" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Envoyer" }));

    const panel = await screen.findByRole("region", { name: "Découverte avec l'Architecte" });
    await waitFor(() => expect(within(panel).getByText("Session interrompue")).toBeTruthy());
    expect(within(panel).getByRole("alert").textContent).toContain("forbidden external content");
    expect(within(panel).queryByText("Génération confinée")).toBeNull();
    expect(within(panel).queryByRole("button", { name: "Annuler la session" })).toBeNull();
    expect(architecture).toHaveBeenCalledTimes(2);
  });
});
