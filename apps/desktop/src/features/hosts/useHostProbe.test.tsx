import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HostView } from "@/ipc/types";

const hostsProbe = vi.hoisted(() => vi.fn());
vi.mock("@/ipc/api", async (importActual) => {
  const actual = await importActual<typeof import("@/ipc/api")>();
  return {
    ...actual,
    api: new Proxy(actual.api, {
      get: (target, prop) => (prop === "hosts_probe" ? hostsProbe : Reflect.get(target, prop)),
    }),
  };
});

import { PROBE_INTERVAL_MS, PROBE_LIMIT, useHostProbe } from "./useHostProbe";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function makeHost(id: string): HostView {
  return {
    id,
    name: id,
    address: "10.0.0.1",
    port: 22,
    username: "ops",
    auth_kind: "password",
    has_password: false,
    key_id: null,
    group_id: null,
    tags: [],
    favorite: false,
    sort: 0,
    jump_host_id: null,
    proxy_mode: "device_default",
    proxy_id: null,
    note: "",
    ai_notes: "",
    updated_at: 0,
    last_connected_at: null,
    os: null,
    show_stats: false,
    dashboard: true,
    env: [],
  };
}

describe("useHostProbe", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.useFakeTimers();
    hostsProbe.mockReset();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    vi.useRealTimers();
  });

  it("probes up to PROBE_LIMIT hosts immediately, repeats on interval, and ignores errors", async () => {
    let latest: Record<string, boolean> = {};
    function Probe({ list, enabled }: { list: HostView[]; enabled: boolean }) {
      latest = useHostProbe(list, enabled);
      return null;
    }

    const many = Array.from({ length: PROBE_LIMIT + 5 }, (_, i) => makeHost(`h-${String(i).padStart(3, "0")}`));
    hostsProbe
      .mockResolvedValueOnce([{ id: "h-000", online: true }])
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce([{ id: "h-000", online: false }]);

    await act(async () => {
      root.render(<Probe list={many} enabled />);
    });
    expect(hostsProbe).toHaveBeenCalledTimes(1);
    expect(hostsProbe.mock.calls[0][0]).toHaveLength(PROBE_LIMIT);
    expect(latest["h-000"]).toBe(true);

    // Failed refresh leaves previous reachability intact.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(PROBE_INTERVAL_MS);
    });
    expect(hostsProbe).toHaveBeenCalledTimes(2);
    expect(latest["h-000"]).toBe(true);

    // Next successful refresh updates the state.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(PROBE_INTERVAL_MS);
    });
    expect(hostsProbe).toHaveBeenCalledTimes(3);
    expect(latest["h-000"]).toBe(false);
  });

  it("does not probe when disabled or empty", async () => {
    function Probe({ list, enabled }: { list: HostView[]; enabled: boolean }) {
      useHostProbe(list, enabled);
      return null;
    }

    await act(async () => {
      root.render(<Probe list={[makeHost("h1")]} enabled={false} />);
    });
    await act(async () => {
      root.render(<Probe list={[]} enabled />);
    });
    expect(hostsProbe).not.toHaveBeenCalled();
  });
});
