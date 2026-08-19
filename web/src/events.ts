// The single SSE channel (D10), over fetch rather than EventSource: the
// stream must carry the bearer token, and EventSource cannot set headers.
// Reconnects with backoff and resumes from the last seq (`?after=`).

import { forceUnauthorized, getToken } from "./api";

export type ConnStatus = "up" | "down";
type EventHandler = (kind: string, payload: string) => void;
type StatusHandler = (status: ConnStatus) => void;

const eventHandlers = new Set<EventHandler>();
const statusHandlers = new Set<StatusHandler>();
let started = false;

export function onEvent(handler: EventHandler): () => void {
  eventHandlers.add(handler);
  ensureStarted();
  return () => eventHandlers.delete(handler);
}

export function onStatus(handler: StatusHandler): () => void {
  statusHandlers.add(handler);
  return () => statusHandlers.delete(handler);
}

function emitStatus(status: ConnStatus) {
  statusHandlers.forEach((h) => h(status));
}

function ensureStarted() {
  if (!started) {
    started = true;
    void tail(0);
  }
}

async function tail(cursor: number): Promise<void> {
  let next = cursor;
  let delay = 1000;
  for (;;) {
    try {
      const resumed = await read(next);
      if (resumed === null) {
        // A 401 is a terminal state for this authenticated session. The token
        // screen will start a fresh tail after the owner signs in again.
        started = false;
        emitStatus("down");
        return;
      }
      next = resumed;
      delay = 1000;
      emitStatus("up");
    } catch {
      emitStatus("down");
      await new Promise((resolve) => setTimeout(resolve, delay));
      delay = Math.min(delay * 2, 15000);
    }
  }
}

/// One connection's worth of frames; resolves with the cursor to resume
/// from when the server (or the network) closes the stream.
async function read(cursor: number): Promise<number | null> {
  const token = getToken();
  const response = await fetch(`/api/events?after=${cursor}`, {
    headers: token ? { authorization: `Bearer ${token}` } : {},
  });
  if (response.status === 401) {
    forceUnauthorized();
    return null;
  }
  if (!response.ok || !response.body) throw new Error(`sse: ${response.status}`);

  const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();
  let buffer = "";
  let last = cursor;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) return last;
    buffer += value;
    const frames = buffer.split("\n\n");
    buffer = frames.pop() ?? "";
    for (const frame of frames) {
      const parsed = parseFrame(frame);
      if (!parsed) continue;
      if (parsed.id) last = Number(parsed.id);
      if (parsed.event) {
        eventHandlers.forEach((h) => h(parsed.event!, parsed.data ?? ""));
      }
    }
  }
}

function parseFrame(frame: string): { id?: string; event?: string; data?: string } | null {
  const out: { id?: string; event?: string; data?: string } = {};
  for (const line of frame.split("\n")) {
    if (line.startsWith(":")) return null; // heartbeat comment
    const colon = line.indexOf(":");
    if (colon < 0) continue;
    const field = line.slice(0, colon);
    const value = line.slice(colon + 1).replace(/^ /, "");
    if (field === "id") out.id = value;
    else if (field === "event") out.event = value;
    else if (field === "data") out.data = value;
  }
  return out.event ? out : null;
}
