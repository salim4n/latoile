// The API wrapper: bearer header on, 401 clears the token and fires the
// unauthorized handler (→ token screen).

import { ApiError, api, getToken, onUnauthorized, setToken } from "./api";

describe("api", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.restoreAllMocks();
  });

  it("sends the bearer token and parses the body", async () => {
    setToken("abc");
    const spy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify([{ id: "p1" }]), { status: 200 }),
    );
    const projects = await api.projects();
    expect(projects).toEqual([{ id: "p1" }]);
    const headers = spy.mock.calls[0][1]?.headers as Record<string, string>;
    expect(headers.authorization).toBe("Bearer abc");
  });

  it("maps the {code,message} shape to ApiError", async () => {
    setToken("abc");
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify({ code: "not_found", message: "project not found" }), {
        status: 404,
      }),
    );
    const err = await api.projects().catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.code).toBe("not_found");
  });

  it("a 401 clears the token and notifies", async () => {
    setToken("expired");
    let notified = false;
    onUnauthorized(() => {
      notified = true;
    });
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify({ code: "unauthorized", message: "no" }), { status: 401 }),
    );
    await api.projects().catch(() => {});
    expect(getToken()).toBeNull();
    expect(notified).toBe(true);
  });

  it("agent-auth start posts to the right route", async () => {
    setToken("abc");
    const spy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify({ session_id: "s1", status: "starting", url: null, error: null }),
        { status: 200 },
      ),
    );
    const session = await api.agentAuthStart("claude");
    expect(session.session_id).toBe("s1");
    expect(spy.mock.calls[0][0]).toBe("/api/agent-auth/start");
    expect(spy.mock.calls[0][1]?.method).toBe("POST");
  });

  it("routing reads and writes the settings route", async () => {
    setToken("abc");
    const routing = {
      manager: "claude", architect: "claude", backend: "codex",
      frontend: "codex", reviewer: "claude",
    } as const;
    const spy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify(routing), { status: 200 }),
    );
    expect((await api.getRouting()).backend).toBe("codex");
    await api.putRouting(routing);
    const put = spy.mock.calls[1];
    expect(put[0]).toBe("/api/settings/routing");
    expect(put[1]?.method).toBe("PUT");
  });

  it("loads visual evidence images through authenticated object URLs", async () => {
    setToken("review-token");
    const objectUrl = vi.fn(() => "blob:trusted-visual-evidence");
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: objectUrl,
    });
    const spy = vi.spyOn(globalThis, "fetch").mockImplementation(async () =>
      new Response(new Blob(["png"], { type: "image/png" }), { status: 200 })
    );

    await expect(api.baselinePng("spec/3", "home fr")).resolves.toBe(
      "blob:trusted-visual-evidence",
    );
    await api.visualRender("visual:run/home");
    await api.visualHeatmap("visual:run/home");

    expect(spy.mock.calls.map(([path]) => path)).toEqual([
      "/api/spec-versions/spec%2F3/baselines/home%20fr/image",
      "/api/visual-comparisons/visual%3Arun%2Fhome/render",
      "/api/visual-comparisons/visual%3Arun%2Fhome/heatmap",
    ]);
    for (const [, init] of spy.mock.calls) {
      expect((init?.headers as Record<string, string>).authorization).toBe(
        "Bearer review-token",
      );
    }
    expect(objectUrl).toHaveBeenCalledTimes(3);
  });
});
