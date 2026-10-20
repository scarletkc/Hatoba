import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, AvailableUpdate, PackageManager, UpdateCheck, UpdateProgress } from "@/ipc/types";

const backend = vi.hoisted(() => ({
  calls: 0,
  answer: null as (() => Promise<UpdateCheck>) | null,
  installs: 0,
  install: null as ((onProgress: (p: UpdateProgress) => void) => Promise<void>) | null,
  autoCheck: false,
  manager: null as PackageManager | null,
}));

vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    update_check: () => {
      backend.calls++;
      return backend.answer!();
    },
    update_install: (onProgress: (p: UpdateProgress) => void) => {
      backend.installs++;
      return backend.install!(onProgress);
    },
  },
}));

vi.mock("./store", () => {
  const state = () => ({ prefs: { auto_update_check: backend.autoCheck }, info: { package_manager: backend.manager } });
  return { useApp: Object.assign((select: (st: ReturnType<typeof state>) => unknown) => select(state()), { getState: state }) };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const found = (version: string | null): UpdateCheck => ({
  current_version: "0.1.0",
  update: version
    ? { version, notes: "## New\n\nThings.", published_at: 1_790_856_000_000, release_url: `https://github.com/scarletkc/Hatoba/releases/tag/v${version}` }
    : null,
});
const offline: AppError = { code: "sync_offline", detail: "GitHub could not be reached" };
const badSignature: AppError = { code: "update_signature", detail: "Minisign error" };

// Each test gets fresh module state: the store and the automatic check's once-per-launch start.
async function load() {
  vi.resetModules();
  return import("./update");
}

/** What the sidebar dot and the tray menu offer, read as a component reads it. */
function offered({ useOfferedUpdate }: Awaited<ReturnType<typeof load>>): AvailableUpdate | null {
  let update: AvailableUpdate | null = null;
  const Probe = () => {
    update = useOfferedUpdate();
    return null;
  };
  const root = createRoot(document.createElement("div"));
  act(() => root.render(createElement(Probe)));
  act(() => root.unmount());
  return update;
}

beforeEach(() => {
  backend.calls = 0;
  backend.answer = async () => found(null);
  backend.installs = 0;
  backend.install = () => new Promise(() => {});
  backend.autoCheck = false;
  backend.manager = null;
});

describe("update check", () => {
  it("records a newer release, which lights the sidebar dot", async () => {
    const update = await load();
    backend.answer = async () => found("0.2.0");
    await update.useUpdate.getState().check();
    expect(update.useUpdate.getState()).toMatchObject({ checking: false, error: null, result: found("0.2.0") });
    expect(offered(update)).toEqual(found("0.2.0").update);
  });

  it("does not light the dot when no release is newer", async () => {
    const update = await load();
    await update.useUpdate.getState().check();
    expect(update.useUpdate.getState().result).toEqual(found(null));
    expect(offered(update)).toBeNull();
  });

  it("records a newer release without the dot when a package manager installed Hatoba", async () => {
    backend.manager = "scoop";
    const update = await load();
    backend.answer = async () => found("0.2.0");
    await update.useUpdate.getState().check();
    expect(update.useUpdate.getState().result).toEqual(found("0.2.0"));
    expect(offered(update)).toBeNull();
  });

  it("joins a check that is already running", async () => {
    const { useUpdate } = await load();
    let finish!: (v: UpdateCheck) => void;
    backend.answer = () => new Promise((resolve) => (finish = resolve));
    const first = useUpdate.getState().check();
    const second = useUpdate.getState().check();
    expect(useUpdate.getState().checking).toBe(true);
    finish(found("0.2.0"));
    await Promise.all([first, second]);
    expect(backend.calls).toBe(1);
    expect(useUpdate.getState().checking).toBe(false);
  });

  it("keeps the last result through a failure and clears the error on the next check", async () => {
    const update = await load();
    const { useUpdate } = update;
    backend.answer = async () => found("0.2.0");
    await useUpdate.getState().check();
    backend.answer = () => Promise.reject(offline);
    await useUpdate.getState().check();
    expect(useUpdate.getState().error).toEqual(offline);
    expect(offered(update)).toEqual(found("0.2.0").update);

    backend.answer = async () => found("0.2.0");
    const next = useUpdate.getState().check();
    expect(useUpdate.getState().error).toBeNull();
    await next;
  });
});

describe("update install", () => {
  it("follows the download until the app closes for the installer", async () => {
    const { useUpdate } = await load();
    let report!: (p: UpdateProgress) => void;
    backend.install = (onProgress) => {
      report = onProgress;
      return new Promise(() => {});
    };
    void useUpdate.getState().installUpdate();
    expect(useUpdate.getState().install).toEqual({ kind: "starting" });
    report({ kind: "downloading", downloaded: 1024, total: 4096 });
    expect(useUpdate.getState().install).toEqual({ kind: "downloading", downloaded: 1024, total: 4096 });
    report({ kind: "installing" });
    expect(useUpdate.getState().install).toEqual({ kind: "installing" });

    void useUpdate.getState().installUpdate();
    expect(backend.installs).toBe(1);
  });

  it("records why an install failed and allows another try", async () => {
    const { useUpdate } = await load();
    backend.install = () => Promise.reject(badSignature);
    await useUpdate.getState().installUpdate();
    expect(useUpdate.getState()).toMatchObject({ install: null, installError: badSignature });

    backend.install = () => Promise.reject(offline);
    await useUpdate.getState().installUpdate();
    expect(useUpdate.getState().installError).toEqual(offline);
    expect(backend.installs).toBe(2);
  });

  it("clears the install error when the next check starts", async () => {
    const { useUpdate } = await load();
    backend.install = () => Promise.reject(offline);
    await useUpdate.getState().installUpdate();
    const next = useUpdate.getState().check();
    expect(useUpdate.getState().installError).toBeNull();
    await next;
  });
});

describe("startAutoCheck", () => {
  const HOUR = 60 * 60 * 1000;
  // Lets the clock run, and the checks it starts finish.
  const pass = (ms: number) => vi.advanceTimersByTimeAsync(ms);

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.clearAllTimers();
    vi.useRealTimers();
  });

  it("does nothing while the automatic check is off", async () => {
    const { startAutoCheck, useUpdate } = await load();
    startAutoCheck();
    await pass(48 * HOUR);
    expect(backend.calls).toBe(0);
    expect(useUpdate.getState().result).toBeNull();
  });

  it("never checks when a package manager installed Hatoba", async () => {
    backend.autoCheck = true;
    backend.manager = "aur";
    const { startAutoCheck } = await load();
    startAutoCheck();
    await pass(48 * HOUR);
    expect(backend.calls).toBe(0);
  });

  it("checks after the first unlock, once however often it is asked", async () => {
    backend.autoCheck = true;
    const { startAutoCheck, useUpdate } = await load();
    startAutoCheck();
    await pass(0);
    expect(useUpdate.getState().result).not.toBeNull();
    startAutoCheck();
    await pass(HOUR);
    expect(backend.calls).toBe(1);
  });

  it("checks again a day after the last check while Hatoba runs", async () => {
    backend.autoCheck = true;
    const { startAutoCheck } = await load();
    startAutoCheck();
    await pass(0);
    await pass(23 * HOUR);
    expect(backend.calls).toBe(1);
    await pass(HOUR);
    expect(backend.calls).toBe(2);
    await pass(24 * HOUR);
    expect(backend.calls).toBe(3);
  });

  it("counts the day from a check the user ran", async () => {
    backend.autoCheck = true;
    const { startAutoCheck, useUpdate } = await load();
    startAutoCheck();
    await pass(0);
    await pass(12 * HOUR);
    await useUpdate.getState().check();
    await pass(12 * HOUR);
    expect(backend.calls).toBe(2);
    await pass(12 * HOUR);
    expect(backend.calls).toBe(3);
  });

  it("tries a failed check again an hour later", async () => {
    backend.autoCheck = true;
    backend.answer = () => Promise.reject(offline);
    const { startAutoCheck, useUpdate } = await load();
    startAutoCheck();
    await pass(0);
    expect(useUpdate.getState().error).toEqual(offline);
    backend.answer = async () => found("0.2.0");
    await pass(HOUR);
    expect(backend.calls).toBe(2);
    expect(useUpdate.getState().result).toEqual(found("0.2.0"));
  });

  it("starts checking once the user turns the automatic check on", async () => {
    const { startAutoCheck } = await load();
    startAutoCheck();
    await pass(HOUR);
    expect(backend.calls).toBe(0);
    backend.autoCheck = true;
    await pass(HOUR);
    expect(backend.calls).toBe(1);
  });

  it("does not check while an update installs", async () => {
    backend.autoCheck = true;
    const { startAutoCheck, useUpdate } = await load();
    startAutoCheck();
    await pass(0);
    void useUpdate.getState().installUpdate();
    await pass(48 * HOUR);
    expect(backend.calls).toBe(1);
  });
});
