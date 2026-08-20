import { onUnauthorized, setToken } from "./api";
import { onEvent } from "./events";

describe("ISSUE-007 unauthorized event stream", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    localStorage.clear();
  });

  it("does not reconnect after a 401 until a new authenticated session starts", async () => {
    vi.useFakeTimers();
    setToken("expired");
    const unauthorized = vi.fn();
    onUnauthorized(unauthorized);
    const fetch = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify({ code: "unauthorized" }), { status: 401 }),
    );

    const unsubscribe = onEvent(() => {});
    await vi.waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
    expect(unauthorized).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetch).toHaveBeenCalledTimes(1);
    unsubscribe();
  });
});
