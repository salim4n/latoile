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
});
