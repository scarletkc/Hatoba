import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Unlisten } from "./api";
import type { EventMap } from "./types";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

interface Pending<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

function pending<T>(): Pending<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

type Listener = (payload: unknown) => void;

const mocked = vi.hoisted(() => ({
  listen: vi.fn<(event: keyof EventMap, handler: Listener) => Promise<Unlisten>>(),
}));

vi.mock("./api", () => ({
  api: { listen: mocked.listen },
}));

import { useBackendEvent } from "./useBackendEvent";

describe("useBackendEvent", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    mocked.listen.mockReset();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => {
      root.unmount();
    });
    host.remove();
  });

  it("delivers typed payloads and unsubscribes on unmount", async () => {
    let emit: ((payload: EventMap["window://chrome"]) => void) | null = null;
    const unlisten = vi.fn();
    mocked.listen.mockImplementation((_event, handler) => {
      emit = handler as (payload: EventMap["window://chrome"]) => void;
      return Promise.resolve(unlisten);
    });

    const received: boolean[] = [];
    function Probe() {
      useBackendEvent("window://chrome", ({ floating }) => received.push(floating));
      return null;
    }

    await act(async () => {
      root.render(<Probe />);
    });
    expect(mocked.listen).toHaveBeenCalledTimes(1);
    expect(mocked.listen).toHaveBeenCalledWith("window://chrome", expect.any(Function));

    emit!({ floating: true });
    expect(received).toEqual([true]);

    await act(async () => {
      root.unmount();
    });
    expect(unlisten).toHaveBeenCalledTimes(1);

    // Late events arriving before the backend drops the listener are ignored.
    emit!({ floating: false });
    expect(received).toEqual([true]);
  });

  it("calls the latest handler without re-subscribing", async () => {
    let emit: ((payload: EventMap["window://zoom"]) => void) | null = null;
    const unlisten = vi.fn();
    mocked.listen.mockImplementation((_event, handler) => {
      emit = handler as (payload: EventMap["window://zoom"]) => void;
      return Promise.resolve(unlisten);
    });

    const seen: string[] = [];
    function Probe({ tag }: { tag: string }) {
      useBackendEvent("window://zoom", ({ zoom }) => seen.push(`${tag}:${zoom}`));
      return null;
    }

    await act(async () => {
      root.render(<Probe tag="v1" />);
    });
    emit!({ zoom: 100 });

    await act(async () => {
      root.render(<Probe tag="v2" />);
    });
    emit!({ zoom: 110 });

    expect(mocked.listen).toHaveBeenCalledTimes(1);
    expect(unlisten).not.toHaveBeenCalled();
    expect(seen).toEqual(["v1:100", "v2:110"]);
  });

  it("ignores late events, skips onSubscribed, and unlistens if unmounted before resolution", async () => {
    const sub = pending<Unlisten>();
    let emit: ((payload: EventMap["window://close-prompt"]) => void) | null = null;
    mocked.listen.mockImplementation((_event, handler) => {
      emit = handler as (payload: EventMap["window://close-prompt"]) => void;
      return sub.promise;
    });

    const onEvent = vi.fn();
    const onSubscribed = vi.fn();
    function Probe() {
      useBackendEvent("window://close-prompt", onEvent, { onSubscribed });
      return null;
    }

    await act(async () => {
      root.render(<Probe />);
    });
    await act(async () => {
      root.unmount();
    });

    const unlisten = vi.fn();
    await act(async () => {
      sub.resolve(unlisten);
      await Promise.resolve();
    });
    emit!(null);

    expect(onSubscribed).not.toHaveBeenCalled();
    expect(onEvent).not.toHaveBeenCalled();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("respects enabled and subscribes or unsubscribes when toggled", async () => {
    const unlisten = vi.fn();
    mocked.listen.mockResolvedValue(unlisten);

    function Probe({ enabled }: { enabled: boolean }) {
      useBackendEvent("app://launch", () => {}, { enabled });
      return null;
    }

    await act(async () => {
      root.render(<Probe enabled={false} />);
    });
    expect(mocked.listen).not.toHaveBeenCalled();

    await act(async () => {
      root.render(<Probe enabled />);
    });
    expect(mocked.listen).toHaveBeenCalledTimes(1);

    await act(async () => {
      root.render(<Probe enabled={false} />);
    });
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("invokes the latest onSubscribed after api.listen resolves", async () => {
    const sub = pending<Unlisten>();
    mocked.listen.mockReturnValue(sub.promise);

    const calls: string[] = [];
    function Probe({ label }: { label: string }) {
      useBackendEvent("window://chrome", () => {}, {
        onSubscribed: () => calls.push(label),
      });
      return null;
    }

    await act(async () => {
      root.render(<Probe label="first" />);
    });
    expect(calls).toEqual([]);

    await act(async () => {
      root.render(<Probe label="second" />);
    });

    await act(async () => {
      sub.resolve(vi.fn());
      await Promise.resolve();
    });
    expect(calls).toEqual(["second"]);
  });
});
