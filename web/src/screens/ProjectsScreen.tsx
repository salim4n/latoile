// The projects list — rows in the mockups' ActiveProjects shape, an empty
// state pointing at creation, and the ghost button for a new project.

import { Link } from "react-router-dom";
import { api } from "../api";
import { useAsync } from "../hooks";
import { useT } from "../i18n";
import { Shell } from "../components/Shell";
import { EmptyState, ErrorState, Skeletons } from "../components/states";

export function ProjectsScreen() {
  const { t } = useT();
  const projects = useAsync(api.projects, []);

  return (
    <Shell title={t("projects.title")}>
      {projects.loading && <Skeletons />}
      {projects.error && (
        <ErrorState
          title={t("inbox.error.title")}
          body={t("inbox.error.body")}
          onRetry={projects.reload}
        />
      )}
      {projects.data && projects.data.length === 0 && (
        <EmptyState
          title={t("projects.empty.title")}
          body={t("projects.empty.body")}
          action={
            <Link className="btn btn--primary" to="/projects/new">
              {t("projects.new")}
            </Link>
          }
        />
      )}
      {projects.data && projects.data.length > 0 && (
        <>
          <div className="sec">
            {projects.data.map((p) => (
              <Link className="card row" to={`/projects/${p.id}`} key={p.id}>
                <div className="row-main">
                  <h3>{p.name}</h3>
                  <p>{p.github_repo}</p>
                </div>
                <span className="badge badge--neutral">{p.status}</span>
              </Link>
            ))}
          </div>
          <Link className="btn btn--ghost btn--block" to="/projects/new">
            {t("projects.new")}
          </Link>
        </>
      )}
    </Shell>
  );
}
