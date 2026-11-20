import { useEffect, useMemo, useState } from "react";
import { api } from "@/ipc/api";
import type { AnyHostView } from "@/ipc/types";

export const PROBE_INTERVAL_MS = 60_000;
/** Hosts probed per refresh (HOST-10 probes what is on screen; the list is ordered, so the top comes first). */
export const PROBE_LIMIT = 200;

/** TCP reachability of the listed hosts, checked immediately and every 60 seconds while enabled (HOST-10). */
export function useHostProbe(hosts: readonly AnyHostView[], enabled: boolean): Record<string, boolean> {
  const [probe, setProbe] = useState<Record<string, boolean>>({});
  const probeKey = useMemo(
    () =>
      hosts
        .slice(0, PROBE_LIMIT)
        .map((h) => h.id)
        .sort()
        .join(","),
    [hosts],
  );

  useEffect(() => {
    if (!enabled || !probeKey) return;
    const ids = probeKey.split(",");
    let cancelled = false;
    const run = async () => {
      try {
        const results = await api.hosts_probe(ids);
        if (cancelled) return;
        setProbe((prev) => {
          const next = { ...prev };
          for (const r of results) next[r.id] = r.online;
          return next;
        });
      } catch {
        /* probing is best effort */
      }
    };
    void run();
    const timer = window.setInterval(() => void run(), PROBE_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [probeKey, enabled]);

  return probe;
}
