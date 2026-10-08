import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, UpdateCheck } from "@/ipc/types";

const backend = vi.hoisted(() => ({
  calls: 0,
  answer: null as (() => Promise<UpdateCheck>) | null,
  autoCheck: false,
}));

vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    update_check: () => {
      backend.calls++;
      return backend.answer!();
    },
  },
}));

vi.mock("./store", () => ({
  useApp: { getState: () => ({ prefs: { auto_update_check: backend.autoCheck } }) },
}));

const found = (latest: string | null, update_available: boolean): UpdateCheck => ({
  current_version: "0.1.0",
  latest_version: latest,
  release_url: latest ? `https://github.com/scarletkc/Hatoba/releases/tag/v${latest}` : null,
  update_available,
});
const offline: AppError = { code: "sync_offline", detail: "GitHub could not be reached" };

// Each test gets fresh module state: the store and the once-per-launch flag.
async function load() {
  vi.resetModules();
  return import("./update");
}

beforeEach(() => {
  backend.calls = 0;
  backend.answer = async () => found(null, false);
  backend.autoCheck = false;
});

describe("update check", () => {
  it("records a newer release, which lights the sidebar dot", async () => {
    const { useUpdate, selectUpdateAvailable } = await load();
    backend.answer = async () => found("0.2.0", true);
    await useUpdate.getState().check();
    expect(useUpdate.getState()).toMatchObject({ checking: false, error: null, result: found("0.2.0", true) });
    expect(selectUpdateAvailable(useUpdate.getState())).toBe(true);
  });

  it("does not light the dot when no release is newer", async () => {
    const { useUpdate, selectUpdateAvailable } = await load();
    await useUpdate.getState().check();
    expect(useUpdate.getState().result).toEqual(found(null, false));
    expect(selectUpdateAvailable(useUpdate.getState())).toBe(false);
  });

  it("joins a check that is already running", async () => {
    const { useUpdate } = await load();
    let finish!: (v: UpdateCheck) => void;
    backend.answer = () => new Promise((resolve) => (finish = resolve));
    const first = useUpdate.getState().check();
    const second = useUpdate.getState().check();
    expect(useUpdate.getState().checking).toBe(true);
    finish(found("0.2.0", true));
    await Promise.all([first, second]);
    expect(backend.calls).toBe(1);
    expect(useUpdate.getState().checking).toBe(false);
  });

  it("keeps the last result through a failure and clears the error on the next check", async () => {
    const { useUpdate, selectUpdateAvailable } = await load();
    backend.answer = async () => found("0.2.0", true);
    await useUpdate.getState().check();
    backend.answer = () => Promise.reject(offline);
    await useUpdate.getState().check();
    expect(useUpdate.getState().error).toEqual(offline);
    expect(selectUpdateAvailable(useUpdate.getState())).toBe(true);

    backend.answer = async () => found("0.2.0", true);
    const next = useUpdate.getState().check();
    expect(useUpdate.getState().error).toBeNull();
    await next;
  });
});

describe("checkOnFirstUnlock", () => {
  it("does nothing while the automatic check is off", async () => {
    const { checkOnFirstUnlock, useUpdate } = await load();
    checkOnFirstUnlock();
    expect(backend.calls).toBe(0);
    expect(useUpdate.getState().result).toBeNull();
  });

  it("checks once per launch when the automatic check is on", async () => {
    backend.autoCheck = true;
    const { checkOnFirstUnlock, useUpdate } = await load();
    checkOnFirstUnlock();
    await vi.waitFor(() => expect(useUpdate.getState().result).not.toBeNull());
    checkOnFirstUnlock();
    expect(backend.calls).toBe(1);
  });
});
