import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { ApiError, api, type ArchitectureSession, type Project } from "../api";
import { LangProvider } from "../i18n";
import { ProjectScreen } from "./ProjectScreen";

const project: Project = {
  id: "project-retry-architecture",
  name: "Retry architecture",
  slug: "retry-architecture",
  github_repo: "salim4n/retry-architecture",
  work_branch: "work",
  status: "draft",
  dev_command: "",
};

const failed: ArchitectureSession = {
  id: "failed-architecture",
  project_id: project.id,
  status: "failed",
  phase: "ready_to_draft",
  requested_locale: "fr-FR",
  package_status: "not_started",
  failure_reason: "architecture package rejected",
  questions: [],
};

describe("ISSUE-010 failed architecture recovery", () => {
  afterEach(() => vi.restoreAllMocks());

  it("makes the next explicit brief start a fresh architecture discovery", async () => {
    vi.spyOn(api, "project").mockResolvedValue(project);
    vi.spyOn(api, "delivery").mockResolvedValue({ status: "not_started", work_branch: "work" });
    vi.spyOn(api, "messages").mockResolvedValue([]);
    vi.spyOn(api, "architecture").mockResolvedValue(failed);
    vi.spyOn(api, "specs").mockResolvedValue([]);
    vi.spyOn(api, "preview").mockRejectedValue(new ApiError(404, "not_found", "missing"));
    const send = vi.spyOn(api, "sendMessage").mockResolvedValue({
      message: { id: "brief", author: "user", content: "Revised brief", actions: null },
      reply: null,
    });

    render(
      <LangProvider>
        <MemoryRouter initialEntries={[`/projects/${project.id}`]}>
          <Routes>
            <Route path="/projects/:id" element={<ProjectScreen />} />
          </Routes>
        </MemoryRouter>
      </LangProvider>,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Préparer une nouvelle découverte" }));
    const input = screen.getByRole("textbox", { name: "Décrire le brief architecture révisé…" });
    fireEvent.change(input, { target: { value: "Revised brief" } });
    fireEvent.click(screen.getByRole("button", { name: "Envoyer" }));

    await waitFor(() =>
      expect(send).toHaveBeenCalledWith(
        project.id,
        "Revised brief",
        "architecture_brief",
        "fr-FR",
      ),
    );
  });
});
