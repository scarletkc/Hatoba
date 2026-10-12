import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FreshReads } from "./freshReads";

// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { useApp } = await import("@/app/store");
const { useFreshReads } = await import("./freshReads");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

interface Pending<T> {
  promise: Promise<T>;
  resolve(value: T): void;
  reject(e: unknown): void;
}

function pending<T>(): Pending<T> {
  let resolve!: (value: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** The backend's answers to the reads, in the order they were made. */
let reads: Pending<string>[] = [];
let shown: string[] = [];
let hook: FreshReads;
let unmount: (() => void) | null = null;

function Probe() {
  const [, setData] = useState("");
  hook = useFreshReads(
    () => {
      const read = pending<string>();
      reads.push(read);
      return read.promise;
    },
    (data) => {
      shown.push(data);
      setData(data);
    },
    () => {},
  );
  return null;
}

function mount() {
  const host = document.createElement("div");
  const root = createRoot(host);
  act(() => root.render(<Probe />));
  unmount = () => root.unmount();
}

/** Answers read `i` and lets the hook handle the answer. */
async function answer(i: number, data: string) {
  await act(async () => reads[i].resolve(data));
}

afterEach(() => {
  act(() => unmount?.());
  unmount = null;
  reads = [];
  shown = [];
});

describe("useFreshReads", () => {
  it("reads when the pane opens and again after each sync round", async () => {
    mount();
    expect(reads).toHaveLength(1);
    await answer(0, "first");
    act(() => useApp.getState().refreshSynced());
    expect(reads).toHaveLength(2);
    await answer(1, "pulled");
    expect(shown).toEqual(["first", "pulled"]);
  });

  it("drops a read that a later read overtook", async () => {
    mount();
    act(() => useApp.getState().refreshSynced());
    expect(reads).toHaveLength(2);
    await answer(1, "after the second round");
    await answer(0, "after the first round");
    expect(shown).toEqual(["after the second round"]);
  });

  it("drops a read that an edit raced and reads again once the save landed", async () => {
    mount();
    await answer(0, "first");
    act(() => useApp.getState().refreshSynced());
    const saved = pending<void>();
    let done!: Promise<void>;
    act(() => {
      done = hook.save(() => saved.promise);
    });
    // The backend answered the read before it applied the save.
    await answer(1, "before the save");
    expect(reads).toHaveLength(2);
    await act(async () => {
      saved.resolve();
      await done;
    });
    expect(reads).toHaveLength(3);
    await answer(2, "after the save");
    expect(shown).toEqual(["first", "after the save"]);
  });

  it("waits with a sync round's read until the saves in flight landed", async () => {
    mount();
    await answer(0, "first");
    const first = pending<void>();
    const second = pending<void>();
    let done!: Promise<void[]>;
    act(() => {
      done = Promise.all([hook.save(() => first.promise), hook.save(() => second.promise)]);
    });
    act(() => useApp.getState().refreshSynced());
    await act(async () => first.resolve());
    expect(reads).toHaveLength(1);
    await act(async () => {
      second.resolve();
      await done;
    });
    expect(reads).toHaveLength(2);
    await answer(1, "pulled");
    expect(shown).toEqual(["first", "pulled"]);
  });

  it("reads what is stored when a save fails", async () => {
    mount();
    await answer(0, "first");
    const refused = new Error("refused");
    let error: unknown;
    await act(async () => {
      await hook.save(() => Promise.reject(refused)).catch((e: unknown) => (error = e));
    });
    expect(error).toBe(refused);
    expect(reads).toHaveLength(2);
    await answer(1, "stored");
    expect(shown).toEqual(["first", "stored"]);
  });

  it("drops a read that predates an edit saved elsewhere", async () => {
    mount();
    act(() => hook.edited());
    await answer(0, "before the dialog saved");
    expect(reads).toHaveLength(2);
    await answer(1, "after");
    expect(shown).toEqual(["after"]);
  });

  it("stops reading when the pane closes", async () => {
    mount();
    act(() => unmount?.());
    unmount = null;
    await answer(0, "late");
    act(() => useApp.getState().refreshSynced());
    expect(reads).toHaveLength(1);
    expect(shown).toEqual([]);
  });
});
