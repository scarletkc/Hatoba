import { beforeEach, describe, expect, it, vi } from "vitest";

const active = vi.hoisted(() => ({ list: [] as [string, number][], delay: null as Promise<void> | null }));

vi.mock("@/ipc/api", () => ({
  api: {
    forwards_active: async () => {
      await active.delay;
      return active.list;
    },
  },
}));

import { applyForwardEvent, countRunning, dropSessionForwards, getForwardRuns, syncActiveForwards } from "./forwards";

const ev = (session_id: string, forward_id: string, state: "running" | "stopped" | "failed", port: number | null = null, error: string | null = null) =>
  ({ session_id, forward_id, state, port, error }) as const;

let n = 0;
let sid: string;
beforeEach(() => {
  sid = `s-test-${++n}`;
  active.list = [];
  active.delay = null;
});

describe("forward state per session", () => {
  it("keeps events that arrive before anyone asked (auto-start announces before the UI knows the session)", () => {
    applyForwardEvent(ev(sid, "f1", "running", 15432));
    const runs = getForwardRuns(sid);
    expect(runs.f1).toMatchObject({ state: "running", port: 15432 });
    expect(countRunning(runs)).toBe(1);
  });

  it("tracks stop and failure, clearing the port / error that no longer apply", () => {
    applyForwardEvent(ev(sid, "f1", "running", 3000));
    applyForwardEvent(ev(sid, "f1", "failed", null, "Address already in use"));
    expect(getForwardRuns(sid).f1).toMatchObject({ state: "failed", port: null, error: "Address already in use" });
    applyForwardEvent(ev(sid, "f1", "stopped"));
    expect(getForwardRuns(sid).f1).toMatchObject({ state: "stopped", port: null, error: null });
    expect(countRunning(getForwardRuns(sid))).toBe(0);
  });

  it("adopts forwards_active as the initial state", async () => {
    active.list = [["f1", 15432]];
    await syncActiveForwards(sid);
    expect(getForwardRuns(sid).f1).toMatchObject({ state: "running", port: 15432 });
  });

  it("does not let a stale forwards_active snapshot override a newer event", async () => {
    let release!: () => void;
    active.delay = new Promise<void>((resolve) => (release = resolve));
    active.list = [["f1", 15432]];
    const pending = syncActiveForwards(sid);
    applyForwardEvent(ev(sid, "f1", "stopped")); // happened after the snapshot was requested
    release();
    await pending;
    expect(getForwardRuns(sid).f1.state).toBe("stopped");
  });

  it("forgets a session when it ends and ignores its late events", async () => {
    applyForwardEvent(ev(sid, "f1", "running", 1));
    dropSessionForwards(sid);
    expect(getForwardRuns(sid).f1).toBeUndefined();
    applyForwardEvent(ev(sid, "f1", "running", 1));
    active.list = [["f1", 1]];
    await syncActiveForwards(sid);
    expect(getForwardRuns(sid).f1).toBeUndefined();
  });
});
