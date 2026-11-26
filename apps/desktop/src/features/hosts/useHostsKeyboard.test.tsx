import { act, type KeyboardEvent } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HostView } from "@/ipc/types";

const connectTarget = vi.hoisted(() => vi.fn(async () => {}));
const connectTelnetTarget = vi.hoisted(() => vi.fn());
vi.mock("@/session/connect", () => ({ connectTarget, connectTelnetTarget }));

import { TELNET_ROW, TYPED_ROW } from "./HostsRows";
import { useHostsKeyboard, type HostsKeyboardOptions } from "./useHostsKeyboard";

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

function keyEvent(target: HTMLElement, key: string, extra: Partial<KeyboardEvent> = {}): KeyboardEvent {
  return {
    target,
    key,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    nativeEvent: { isComposing: false, keyCode: 0 },
    preventDefault: vi.fn(),
    ...extra,
  } as unknown as KeyboardEvent;
}

describe("useHostsKeyboard", () => {
  const searchEl = document.createElement("input");
  const listEl = document.createElement("div");
  const h1 = makeHost("h1");
  const h2 = makeHost("h2");
  const h3 = makeHost("h3");
  const cleanups: (() => void)[] = [];

  function mountHook(opts: HostsKeyboardOptions): (e: KeyboardEvent) => void {
    let handler!: (e: KeyboardEvent) => void;
    function Harness() {
      handler = useHostsKeyboard(opts);
      return null;
    }
    const container = document.createElement("div");
    const root = createRoot(container);
    act(() => root.render(<Harness />));
    cleanups.push(() => act(() => root.unmount()));
    return handler;
  }

  function makeOpts(overrides: Partial<HostsKeyboardOptions> = {}): HostsKeyboardOptions {
    return {
      searchRef: { current: searchEl },
      listRef: { current: listEl },
      cardsRef: { current: null },
      anchorRef: { current: null },
      platform: "linux",
      view: "list",
      stats: true,
      searching: false,
      query: "",
      hosts: [h1, h2, h3],
      telnetHosts: [],
      visible: [h1, h2, h3],
      hostIds: ["h1", "h2", "h3"],
      rowIds: ["h1", "h2", "h3"],
      quickRows: [],
      selected: ["h2"],
      selectedSet: new Set(["h2"]),
      focusId: "h2",
      several: false,
      suggestions: [],
      suggestOpen: false,
      suggestActive: -1,
      setSuggestAt: vi.fn(),
      setSuggestDismissed: vi.fn(),
      connectQuick: vi.fn(),
      pickRows: vi.fn(),
      pickWith: vi.fn(),
      selectNone: vi.fn(),
      moveTo: vi.fn(),
      setDetail: vi.fn(),
      remove: vi.fn(),
      connect: vi.fn(),
      connectAll: vi.fn(),
      ...overrides,
    };
  }

  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    while (cleanups.length > 0) cleanups.pop()?.();
  });

  it("handles arrow movement, Alt+Arrow reorder, Space detail, Delete, and type-to-search", () => {
    const opts = makeOpts();
    const handler = mountHook(opts);

    handler(keyEvent(listEl, "ArrowDown"));
    expect(opts.pickWith).toHaveBeenCalledWith("h3", { range: false, add: false });

    handler(keyEvent(listEl, "ArrowUp", { altKey: true }));
    expect(opts.moveTo).toHaveBeenCalledWith(["h2"], "h1");

    handler(keyEvent(listEl, " "));
    expect(opts.setDetail).toHaveBeenCalledWith("h2");

    handler(keyEvent(listEl, "Delete"));
    expect(opts.remove).toHaveBeenCalledWith([h2]);

    const focusSpy = vi.spyOn(searchEl, "focus");
    handler(keyEvent(listEl, "p"));
    expect(focusSpy).toHaveBeenCalled();
  });

  it("navigates recent suggestions in search and connects typed SSH or Telnet targets on Enter", () => {
    const recent = { address: "example.com", port: 22, username: "alice" };
    const suggestOpts = makeOpts({
      suggestions: [recent],
      suggestOpen: true,
      suggestActive: 0,
    });
    const suggestHandler = mountHook(suggestOpts);
    suggestHandler(keyEvent(searchEl, "Enter"));
    expect(suggestOpts.connectQuick).toHaveBeenCalledWith(recent);

    suggestHandler(keyEvent(searchEl, "Escape"));
    expect(suggestOpts.setSuggestDismissed).toHaveBeenCalledWith(true);

    // Typed SSH quick-connect row connects what the input holds now.
    const sshOpts = makeOpts({ focusId: TYPED_ROW, query: "alice@192.168.1.9" });
    mountHook(sshOpts)(keyEvent(searchEl, "Enter"));
    expect(connectTarget).toHaveBeenCalledWith({ address: "192.168.1.9", port: 22, username: "alice" });

    // Typed Telnet row connects via connectTelnetTarget.
    const telnetOpts = makeOpts({ focusId: TELNET_ROW, query: "telnet 192.168.1.10 2323" });
    mountHook(telnetOpts)(keyEvent(searchEl, "Enter"));
    expect(connectTelnetTarget).toHaveBeenCalledWith("192.168.1.10", 2323);
  });
});
