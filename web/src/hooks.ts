// Small shared hooks: async loading with the mockups' three states, and
// SSE-driven reloads (D10: the board/chat/inbox refresh themselves when the
// journal says something happened).

import { useCallback, useEffect, useRef, useState } from "react";
import { onEvent } from "./events";

export interface Async<T> {
  data: T | null;
  loading: boolean;
  error: boolean;
  reload: () => void;
}

export function useAsync<T>(load: () => Promise<T>, deps: unknown[]): Async<T> {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const loadRef = useRef(load);
  loadRef.current = load;

  const reload = useCallback(() => {
    setLoading(true);
    setError(false);
    loadRef
      .current()
      .then((value) => {
        setData(value);
        setLoading(false);
      })
      .catch(() => {
        setError(true);
        setLoading(false);
      });
  }, []);

  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(reload, deps);
  return { data, loading, error, reload };
}

/// Reload when one of the given event kinds lands on the SSE channel.
export function useEventReload(kinds: string[], reload: () => void) {
  const ref = useRef(reload);
  ref.current = reload;
  useEffect(() => {
    return onEvent((kind) => {
      if (kinds.includes(kind)) ref.current();
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kinds.join(",")]);
}
