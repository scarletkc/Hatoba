import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StarPrompt } from "@/ipc/types";

const backend = vi.hoisted(() => ({ gets: 0, dones: 0, stored: null as StarPrompt | null }));

vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    star_prompt_get: async () => {
      backend.gets++;
      return { ...backend.stored! };
    },
    star_prompt_done: async () => {
      backend.dones++;
    },
  },
}));

const DAY = 24 * 60 * 60 * 1000;
const NOW = 1_800_000_000_000;
const waiting = (since: number, done = false): StarPrompt => ({ first_seen_at: NOW - since, done });

// Each test gets fresh module state: the stores and the once-per-launch load.
async function load() {
  vi.resetModules();
  return { ...(await import("./star")), ...(await import("./tabs")) };
}

beforeEach(() => {
  backend.gets = 0;
  backend.dones = 0;
  backend.stored = waiting(0);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("starPromptDue", () => {
  it("shows a day after the first read, once a session has connected", async () => {
    const { starPromptDue } = await load();
    expect(starPromptDue(waiting(DAY), true, NOW)).toBe(true);
    expect(starPromptDue(waiting(DAY - 1), true, NOW)).toBe(false);
    expect(starPromptDue(waiting(DAY), false, NOW)).toBe(false);
  });

  it("never shows once done, or before the state loads", async () => {
    const { starPromptDue } = await load();
    expect(starPromptDue(waiting(7 * DAY, true), true, NOW)).toBe(false);
    expect(starPromptDue(null, true, NOW)).toBe(false);
  });
});

describe("star prompt store", () => {
  it("loads the state once per launch", async () => {
    const { useStarPrompt } = await load();
    backend.stored = waiting(DAY);
    await Promise.all([useStarPrompt.getState().load(), useStarPrompt.getState().load()]);
    await useStarPrompt.getState().load();
    expect(backend.gets).toBe(1);
    expect(useStarPrompt.getState().prompt).toEqual(waiting(DAY));
  });

  it("hides the prompt at once and records it as done", async () => {
    const { useStarPrompt, starPromptDue } = await load();
    backend.stored = waiting(DAY);
    await useStarPrompt.getState().load();
    useStarPrompt.getState().finish();
    expect(starPromptDue(useStarPrompt.getState().prompt, true, NOW)).toBe(false);
    await vi.waitFor(() => expect(backend.dones).toBe(1));
  });
});

describe("useStarPromptDue", () => {
  it("renders again when the day is up", async () => {
    vi.useFakeTimers({ now: NOW });
    backend.stored = waiting(DAY - 60_000);
    const { useStarPrompt, useStarPromptDue } = await load();
    // React is loaded after resetModules too, so the hook and the renderer share one copy.
    const { act, createElement } = await import("react");
    const { createRoot } = await import("react-dom/client");
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    await useStarPrompt.getState().load();

    const seen: boolean[] = [];
    function Probe() {
      seen.push(useStarPromptDue(true));
      return null;
    }
    const root = createRoot(document.createElement("div"));
    act(() => root.render(createElement(Probe)));
    expect(seen.at(-1)).toBe(false);
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(seen.at(-1)).toBe(true);
    act(() => root.unmount());
  });
});

describe("connectedOnce", () => {
  it("stays set after the connected tab closes", async () => {
    const { useTabs } = await load();
    const id = useTabs.getState().openSession({ hostId: "h1", target: null }, "web-1");
    useTabs.getState().update(id, { status: "failed" });
    expect(useTabs.getState().connectedOnce).toBe(false);
    useTabs.getState().update(id, { status: "connected", sessionId: "s1" });
    useTabs.getState().closeTab(id);
    expect(useTabs.getState().connectedOnce).toBe(true);
  });
});
