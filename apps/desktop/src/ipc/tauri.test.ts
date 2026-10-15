import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (message: unknown) => void = () => {};
  },
  invoke,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const { createTauriApi } = await import("./tauri");

/** The channel the last command was given, under its argument name. */
const lastChannel = (name: string) => (invoke.mock.lastCall?.[1] as Record<string, { onmessage: (m: unknown) => void }>)[name];

describe("createTauriApi", () => {
  const api = createTauriApi();

  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(null);
  });

  it("passes arguments under Rust's parameter names", async () => {
    await api.host_set_favorite("h1", true);
    expect(invoke).toHaveBeenLastCalledWith("host_set_favorite", { id: "h1", favorite: true });
    await api.mcp_proxy_info("c1", "{\"a\":1}");
    expect(invoke).toHaveBeenLastCalledWith("mcp_proxy_info", { conversationId: "c1", callArguments: "{\"a\":1}" });
    await api.ai_conversation_fork("c1", null, "Fork");
    expect(invoke).toHaveBeenLastCalledWith("ai_conversation_fork", { id: "c1", entryId: null, title: "Fork" });
    await api.app_info();
    expect(invoke).toHaveBeenLastCalledWith("app_info");
  });

  it("resolves a command that returns () to undefined and passes results and errors through", async () => {
    await expect(api.host_delete("h1")).resolves.toBeUndefined();
    invoke.mockResolvedValueOnce(["prod"]);
    await expect(api.hosts_probe(["h1"])).resolves.toEqual(["prod"]);
    const error = { code: "not_found", detail: "host" };
    invoke.mockRejectedValueOnce(error);
    await expect(api.host_get("gone")).rejects.toBe(error);
  });

  it("hands terminal frames over as bytes, whichever form the channel delivers", async () => {
    const frames: Uint8Array[] = [];
    await api.ssh_connect("h1", { cols: 80, rows: 24 } as never, (frame) => frames.push(frame));
    const channel = lastChannel("channel");
    channel.onmessage(new Uint8Array([1, 2]).buffer);
    channel.onmessage([3, 4]);
    expect(frames).toEqual([new Uint8Array([1, 2]), new Uint8Array([3, 4])]);
  });

  it("forwards channel events to the callback", async () => {
    const events: unknown[] = [];
    await api.ai_send({ conversation_id: "c1" } as never, (event) => events.push(event));
    lastChannel("channel").onmessage({ kind: "done" });
    expect(events).toEqual([{ kind: "done" }]);
  });
});
