// The transport layer (contract §6: data fetching only through here — no
// direct fetch in components). Injects the bearer token, maps 401 to the
// token screen, and keeps the server's {code, message} error shape.

const TOKEN_KEY = "latoile-token";

export function getToken(): string | null {
  try {
    return localStorage.getItem(TOKEN_KEY);
  } catch {
    return null;
  }
}

export function setToken(token: string | null) {
  try {
    if (token) localStorage.setItem(TOKEN_KEY, token);
    else localStorage.removeItem(TOKEN_KEY);
  } catch {
    // private mode: the session lives in memory only
  }
}

let unauthorizedHandler: () => void = () => {};
export function onUnauthorized(handler: () => void) {
  unauthorizedHandler = handler;
}

/// For paths outside the fetch wrapper (the SSE reader): same 401 flow.
export function forceUnauthorized() {
  setToken(null);
  unauthorizedHandler();
}

export class ApiError extends Error {
  status: number;
  code: string;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const headers: Record<string, string> = {
    ...(init?.headers as Record<string, string>),
  };
  const token = getToken();
  if (token) headers["authorization"] = `Bearer ${token}`;
  if (init?.body) headers["content-type"] = "application/json";

  let response: Response;
  try {
    response = await fetch(path, { ...init, headers });
  } catch {
    throw new ApiError(0, "unreachable", "network");
  }
  if (response.status === 401) {
    setToken(null);
    unauthorizedHandler();
    throw new ApiError(401, "unauthorized", "token required");
  }
  if (response.status === 204) return undefined as T;
  const body = await response.json().catch(() => null);
  if (!response.ok) {
    throw new ApiError(
      response.status,
      body?.code ?? "error",
      body?.message ?? response.statusText,
    );
  }
  return body as T;
}

async function requestText(path: string): Promise<string> {
  const token = getToken();
  let response: Response;
  try {
    response = await fetch(path, {
      headers: token ? { authorization: `Bearer ${token}` } : {},
    });
  } catch {
    throw new ApiError(0, "unreachable", "network");
  }
  if (response.status === 401) {
    setToken(null);
    unauthorizedHandler();
    throw new ApiError(401, "unauthorized", "token required");
  }
  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new ApiError(
      response.status,
      body?.code ?? "error",
      body?.message ?? response.statusText,
    );
  }
  return response.text();
}

async function requestObjectUrl(path: string): Promise<string> {
  const token = getToken();
  let response: Response;
  try {
    response = await fetch(path, {
      headers: token ? { authorization: `Bearer ${token}` } : {},
    });
  } catch {
    throw new ApiError(0, "unreachable", "network");
  }
  if (response.status === 401) {
    setToken(null);
    unauthorizedHandler();
    throw new ApiError(401, "unauthorized", "token required");
  }
  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new ApiError(
      response.status,
      body?.code ?? "error",
      body?.message ?? response.statusText,
    );
  }
  return URL.createObjectURL(await response.blob());
}

// ── DTOs (mirror crates/server/src/routes/dto.rs) ───────────────────────────

export interface Project {
  id: string;
  name: string;
  slug: string;
  github_repo: string;
  work_branch: string;
  status: "draft" | "specced" | "building" | "live";
  dev_command: string;
  last_activity_at?: string;
}

export interface Delivery {
  status: "not_started" | "pushed" | "pull_request_open";
  work_branch: string;
  local_sha?: string;
  remote_sha?: string;
  pull_request_url?: string;
}

export interface Task {
  id: string;
  project_id: string;
  role_id: string;
  title: string;
  description: string;
  status: "ready" | "in_progress" | "review" | "changes_requested" | "done";
  position: number;
  latest_run_id?: string;
  latest_review_status?: "pending" | "granted" | "rejected";
  latest_decision_comment?: string;
  next_action:
    | "ready_to_start"
    | "agent_working"
    | "reviewer_working"
    | "awaiting_owner_decision"
    | "changes_requested"
    | "correction_ready"
    | "corrective_run_in_progress"
    | "completed";
}

export interface Approval {
  id: string;
  run_id: string;
  kind: "spec" | "review" | "permission";
  status: "pending" | "granted" | "rejected";
  payload: string;
  decision_comment?: string;
  corrective_run_id?: string;
  /// Present on the Inbox read model. Decision responses keep these optional
  /// because they return the domain entity after the transition.
  project_id?: string;
  project_name?: string;
  task_title?: string;
  role_id?: string;
  created_at?: string;
  decided_at?: string;
}

export interface Message {
  id: string;
  author: "user" | "manager";
  content: string;
  actions: string | null;
  created_at?: string;
}

export interface ArchitectureQuestion {
  id: string;
  sequence: number;
  prompt: string;
  status: "open" | "answered";
  answer?: string;
}

export interface ArchitectureSession {
  id: string;
  project_id: string;
  status:
    | "discovering"
    | "awaiting_answer"
    | "ready_to_draft"
    | "failed"
    | "cancelled";
  phase:
    | "domain_discovery"
    | "requirements"
    | "ux_discovery"
    | "ready_to_draft";
  skill_name?: string;
  skill_digest?: string;
  operating_mode?: "greenfield" | "reverse_engineering";
  package_status: "not_started" | "generating" | "draft_ready";
  package?: {
    design_dir: string;
    base_sha: string;
    head_sha: string;
    tree_sha: string;
    package_digest: string;
    manifest_digest: string;
    changed_files: string[];
    diff_stat: string;
  };
  failure_reason?: string;
  questions: ArchitectureQuestion[];
}

export interface SpecVersion {
  id: string;
  project_id: string;
  version: number;
  status: "draft" | "approved" | "superseded";
  design_dir: string;
  architecture_session_id?: string;
  skill_name?: string;
  skill_digest?: string;
  operating_mode?: "greenfield" | "reverse_engineering";
  package_digest?: string;
  manifest_digest?: string;
  package_commit_sha?: string;
  package_tree_sha?: string;
}

export interface ArchitectureVisualScenario {
  comparison_id: string;
  screen: string;
  state: string;
  locale: string;
  theme: "light" | "dark";
  route: string;
  fixture: string;
  readiness_selector: string;
  stable_selectors: string[];
  allowed_masks: string[];
  viewport_width: number;
  viewport_height: number;
  device_scale_factor_milli: number;
  mockup: string;
}

export interface VisualBaseline {
  spec_version_id: string;
  comparison_id: string;
  manifest_digest: string;
  package_commit_sha: string;
  status: "ready" | "failed";
  png_digest?: string;
  geometry_digest?: string;
  accessibility_digest?: string;
  environment_digest?: string;
  browser_version?: string;
  font_fingerprint?: string;
  failure_code?: string;
  failure_message?: string;
  recovery_action?: string;
}

export interface VisualComparison {
  id: string;
  spec_version_id: string;
  project_id: string;
  run_id: string;
  comparison_id: string;
  manifest_digest: string;
  package_commit_sha: string;
  baseline_png_digest: string;
  status: "invalid" | "blocking" | "reservation" | "passed";
  changed_pixels: number;
  total_pixels: number;
  pixel_ratio_micros: number;
  max_geometry_delta_milli: number;
  accessibility_changes: number;
  render_png_digest?: string;
  pixel_diff_digest?: string;
  heatmap_png_digest?: string;
  geometry_diff_digest?: string;
  accessibility_diff_digest?: string;
  environment_digest?: string;
  browser_version?: string;
  font_fingerprint?: string;
  failure_code?: string;
  failure_message?: string;
  recovery_action?: string;
}

export interface ArchitecturePackageValidation {
  valid: boolean;
  package_digest: string;
  manifest_digest: string;
  commit_sha: string;
  tree_sha: string;
  file_count: number;
  gallery_path: string;
  scenarios: ArchitectureVisualScenario[];
  findings: { code: string; message: string }[];
}

export interface Preview {
  id: string;
  project_id: string;
  port: number;
  status: "starting" | "ready" | "stale" | "error" | "stopped";
  alive: boolean;
  logs: string[];
}

export type AgentProvider = "claude" | "codex";

export interface ProviderStatus {
  authenticated: boolean;
  detail: string | null;
}

export interface Role {
  id: string;
  label: string;
}

export type Routing = Record<"manager" | "architect" | "backend" | "frontend" | "reviewer", AgentProvider>;

export interface AgentAuthSession {
  session_id: string;
  provider: AgentProvider;
  status: "starting" | "waiting_for_input" | "validating" | "authenticated" | "failed" | "expired";
  url: string | null;
  input_required: boolean;
  user_code: string | null;
  hint: string | null;
  error: string | null;
}

export interface Repo {
  full_name: string;
  description: string | null;
  private: boolean;
}

// ── Calls (one per route, spec §5.3) ────────────────────────────────────────

export const api = {
  projects: () => request<Project[]>("/api/projects"),
  project: (id: string) => request<Project>(`/api/projects/${id}`),
  delivery: (project: string) =>
    request<Delivery>(`/api/projects/${project}/delivery`),
  deliverProject: (project: string) =>
    request<Delivery>(`/api/projects/${project}/delivery`, { method: "POST" }),
  createProject: (body: Record<string, string>) =>
    request<Project>("/api/projects", { method: "POST", body: JSON.stringify(body) }),
  repos: () => request<Repo[]>("/api/github/repos"),
  approvals: () => request<Approval[]>("/api/approvals"),
  approval: (id: string) => request<Approval>(`/api/approvals/${id}`),
  decide: (id: string, granted: boolean, comment?: string) =>
    request<Approval>(`/api/approvals/${id}`, {
      method: "POST",
      body: JSON.stringify({ granted, comment }),
    }),
  messages: (project: string) => request<Message[]>(`/api/projects/${project}/messages`),
  sendMessage: (project: string, content: string, intent?: "architecture_brief") =>
    request<{ message: Message; reply: Message | null }>(
      `/api/projects/${project}/messages`,
      { method: "POST", body: JSON.stringify({ content, ...(intent ? { intent } : {}) }) },
    ),
  architecture: (project: string) =>
    request<ArchitectureSession | null>(`/api/projects/${project}/architecture`),
  specs: (project: string) =>
    request<SpecVersion[]>(`/api/projects/${project}/spec-versions`),
  validateSpec: (spec: string) =>
    request<ArchitecturePackageValidation>(`/api/spec-versions/${spec}/validation`),
  approveSpec: (spec: string) =>
    request<SpecVersion>(`/api/spec-versions/${spec}/approve`, { method: "POST" }),
  baselines: (spec: string) =>
    request<VisualBaseline[]>(`/api/spec-versions/${spec}/baselines`),
  captureBaselines: (spec: string) =>
    request<VisualBaseline[]>(`/api/spec-versions/${spec}/baselines`, { method: "POST" }),
  baselinePng: (spec: string, comparisonId: string) =>
    requestObjectUrl(
      `/api/spec-versions/${encodeURIComponent(spec)}/baselines/${encodeURIComponent(comparisonId)}/image`,
    ),
  visualComparisons: (run: string) =>
    request<VisualComparison[]>(`/api/runs/${encodeURIComponent(run)}/visual-comparisons`),
  visualRender: (evidence: string) =>
    requestObjectUrl(`/api/visual-comparisons/${encodeURIComponent(evidence)}/render`),
  visualHeatmap: (evidence: string) =>
    requestObjectUrl(`/api/visual-comparisons/${encodeURIComponent(evidence)}/heatmap`),
  specArtifact: (spec: string, path: string) =>
    requestText(`/api/spec-versions/${spec}/artifacts/${path}`),
  cancelArchitecture: (project: string) =>
    request<ArchitectureSession>(`/api/projects/${project}/architecture`, {
      method: "DELETE",
    }),
  tasks: (project: string) => request<Task[]>(`/api/projects/${project}/tasks`),
  preview: (project: string) => request<Preview | null>(`/api/projects/${project}/preview`),
  ensurePreview: (project: string) =>
    request<Preview>(`/api/projects/${project}/preview`, { method: "POST" }),
  stopPreview: (project: string) =>
    request<void>(`/api/projects/${project}/preview`, { method: "DELETE" }),
  agentAuthStart: (provider: AgentProvider) =>
    request<AgentAuthSession>("/api/agent-auth/start", {
      method: "POST",
      body: JSON.stringify({ provider }),
    }),
  agentAuthStatus: (id: string) => request<AgentAuthSession>(`/api/agent-auth/${id}`),
  agentAuthStatusAll: () =>
    request<Record<AgentProvider, ProviderStatus>>("/api/agent-auth/status"),
  agentAuthDisconnect: (provider: AgentProvider) =>
    request<ProviderStatus>("/api/agent-auth/disconnect", {
      method: "POST",
      body: JSON.stringify({ provider }),
    }),
  roles: () => request<Role[]>("/api/roles"),
  getRouting: () => request<Routing>("/api/settings/routing"),
  putRouting: (routing: Routing) =>
    request<Routing>("/api/settings/routing", {
      method: "PUT",
      body: JSON.stringify(routing),
    }),
  agentAuthCode: (id: string, code: string) =>
    request<AgentAuthSession>(`/api/agent-auth/${id}/code`, {
      method: "POST",
      body: JSON.stringify({ code }),
    }),
};
