import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, UpdateCheck, UpdateProgress } from "@/ipc/types";

const backend = vi.hoisted(() => ({
  calls: 0,
  answer: null as (() => Promise<UpdateCheck>) | null,
  installs: 0,
  install: null as ((onProgress: (p: UpdateProgress) => void) => Promise<void>) | null,
  autoCheck: false,
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

vi.mock("./store", () => ({
  useApp: { getState: () => ({ prefs: { auto_update_check: backend.autoCheck } }) },
}));

const found = (version: string | null): UpdateCheck => ({
  current_version: "0.1.0",
  update: version
    ? { version, notes: "## New\n\nThings.", published_at: 1_790_856_000_000, release_url: `https://github.com/scarletkc/Hatoba/releases/tag/v${version}` }
    : null,
});
const offline: AppError = { code: "sync_offline", detail: "GitHub could not be reached" };
const badSignature: AppError = { code: "update_signature", detail: "Minisign error" };

// Each test gets fresh module state: the store and the once-per-launch flag.
async function load() {
  vi.resetModules();
  return import("./update");
}

beforeEach(() => {
  backend.calls = 0;
  backend.answer = async () => found(null);
  backend.installs = 0;
  backend.install = () => new Promise(() => {});
  backend.autoCheck = false;
});

describe("update check", () => {
  it("records a newer release, which lights the sidebar dot", async () => {
    const { useUpdate, selectUpdateAvailable } = await load();
    backend.answer = async () => found("0.2.0");
    await useUpdate.getState().check();
    expect(useUpdate.getState()).toMatchObject({ checking: false, error: null, result: found("0.2.0") });
    expect(selectUpdateAvailable(useUpdate.getState())).toBe(true);
  });

  it("does not light the dot when no release is newer", async () => {
    const { useUpdate, selectUpdateAvailable } = await load();
    await useUpdate.getState().check();
    expect(useUpdate.getState().result).toEqual(found(null));
    expect(selectUpdateAvailable(useUpdate.getState())).toBe(false);
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
    const { useUpdate, selectUpdateAvailable } = await load();
    backend.answer = async () => found("0.2.0");
    await useUpdate.getState().check();
    backend.answer = () => Promise.reject(offline);
    await useUpdate.getState().check();
    expect(useUpdate.getState().error).toEqual(offline);
    expect(selectUpdateAvailable(useUpdate.getState())).toBe(true);

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
