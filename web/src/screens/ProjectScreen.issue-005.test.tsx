import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { ApiError, api, type Project } from "../api";
import { LangProvider } from "../i18n";
import { ProjectScreen } from "./ProjectScreen";

const project: Project = {
  id: "project-delivery-gate",
  name: "Delivery gate",
  slug: "delivery-gate",
  github_repo: "salim4n/delivery-gate",
  work_branch: "work",
  status: "draft",
  dev_command: "",
};

describe("ISSUE-005 delivery prerequisites", () => {
  afterEach(() => vi.restoreAllMocks());

  it("shows the safe domain reason returned by a refused delivery", async () => {
    vi.spyOn(api, "project").mockResolvedValue(project);
    vi.spyOn(api, "delivery").mockResolvedValue({ status: "not_started", work_branch: "work" });
    vi.spyOn(api, "messages").mockResolvedValue([]);
    vi.spyOn(api, "architecture").mockResolvedValue(null);
    vi.spyOn(api, "specs").mockResolvedValue([]);
    vi.spyOn(api, "preview").mockResolvedValue(null);
    vi.spyOn(api, "deliverProject").mockRejectedValue(
      new ApiError(422, "invariant", "delivery requires an approved specification"),
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

    fireEvent.click(await screen.findByRole("button", { name: "Livrer sur GitHub" }));
    expect((await screen.findByRole("alert")).textContent).toBe(
      "delivery requires an approved specification",
    );
  });
});
