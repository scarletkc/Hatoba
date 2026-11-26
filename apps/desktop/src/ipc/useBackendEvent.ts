import { useEffect, useRef } from "react";
import { api } from "./api";
import type { EventMap } from "./types";

export interface BackendEventOptions {
  enabled?: boolean;
  onSubscribed?: (isActive: () => boolean) => void;
}

/**
 * Subscribes to a backend event (§10.2) while mounted (and `enabled`, if given), calling the latest
 * `handler` without re-subscribing when it changes.
 */
export function useBackendEvent<K extends keyof EventMap>(
  event: K,
  handler: (payload: EventMap[K]) => void,
  options?: BackendEventOptions,
): void {
  const enabled = options?.enabled ?? true;
  const latest = useRef({ handler, onSubscribed: options?.onSubscribed });
  latest.current = { handler, onSubscribed: options?.onSubscribed };

  useEffect(() => {
    if (!enabled) return;
    let active = true;
    const sub = api.listen(event, (payload) => {
      if (active) latest.current.handler(payload);
    });
    void sub.then(
      () => {
        if (active) latest.current.onSubscribed?.(() => active);
      },
      () => {},
    );
    return () => {
      active = false;
      void sub.then(
        (un) => un(),
        () => {},
      );
    };
  }, [event, enabled]);
}
