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

// ── DTOs (mirror crates/server/src/routes/dto.rs) ───────────────────────────

export interface Project {
  id: string;
  name: string;
  slug: string;
  github_repo: string;
  work_branch: string;
  status: "draft" | "specced" | "building" | "live";
  dev_command: string;
}

export interface Task {
  id: string;
  project_id: string;
  role_id: string;
  title: string;
  description: string;
  status: "ready" | "in_progress" | "review" | "changes_requested" | "done";
  position: number;
}

export interface Approval {
  id: string;
  run_id: string;
  kind: "spec" | "review" | "permission";
  status: "pending" | "granted" | "rejected";
  payload: string;
}

export interface Message {
  id: string;
  author: "user" | "manager";
  content: string;
  actions: string | null;
}

export interface Preview {
  id: string;
  project_id: string;
  port: number;
  status: "starting" | "ready" | "stale" | "error" | "stopped";
  alive: boolean;
}

export interface Repo {
  full_name: string;
  description: string | null;
}

// ── Calls (one per route, spec §5.3) ────────────────────────────────────────

export const api = {
  projects: () => request<Project[]>("/api/projects"),
  project: (id: string) => request<Project>(`/api/projects/${id}`),
  createProject: (body: Record<string, string>) =>
    request<Project>("/api/projects", { method: "POST", body: JSON.stringify(body) }),
  repos: () => request<Repo[]>("/api/github/repos"),
  approvals: () => request<Approval[]>("/api/approvals"),
  decide: (id: string, granted: boolean) =>
    request<Approval>(`/api/approvals/${id}`, {
      method: "POST",
      body: JSON.stringify({ granted }),
    }),
  messages: (project: string) => request<Message[]>(`/api/projects/${project}/messages`),
  sendMessage: (project: string, content: string) =>
    request<{ message: Message; reply: Message | null }>(
      `/api/projects/${project}/messages`,
      { method: "POST", body: JSON.stringify({ content }) },
    ),
  tasks: (project: string) => request<Task[]>(`/api/projects/${project}/tasks`),
  preview: (project: string) => request<Preview>(`/api/projects/${project}/preview`),
  ensurePreview: (project: string) =>
    request<Preview>(`/api/projects/${project}/preview`, { method: "POST" }),
  stopPreview: (project: string) =>
    request<void>(`/api/projects/${project}/preview`, { method: "DELETE" }),
};
