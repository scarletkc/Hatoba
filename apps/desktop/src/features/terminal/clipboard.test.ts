import { beforeEach, describe, expect, it, vi } from "vitest";
import capabilities from "../../../src-tauri/capabilities/default.json";
import { openExternal } from "./clipboard";

const native = vi.hoisted(() => ({ enabled: false, openUrl: vi.fn() }));
vi.mock("@/ipc/api", () => ({ isTauri: () => native.enabled }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: native.openUrl }));

beforeEach(() => {
  native.enabled = false;
  vi.clearAllMocks();
});

describe("terminal links", () => {
  it("allows only HTTP and HTTPS in the native opener scope", () => {
    const permission = capabilities.permissions.find((p) => typeof p !== "string" && p.identifier === "opener:allow-open-url");
    expect(typeof permission !== "string" && permission?.allow.map((entry) => entry.url).sort()).toEqual(["http://*", "https://*"]);
  });

  it.each(["http://localhost:3000", "http://192.168.1.10", "https://example.com"])("opens %s with the native opener", async (uri) => {
    native.enabled = true;
    await openExternal(uri);
    expect(native.openUrl).toHaveBeenCalledWith(uri);
  });

  it.each(["file:///tmp/test", "javascript:alert(1)", "ssh://example.com", "ftp://example.com"])("does not open %s", async (uri) => {
    native.enabled = true;
    await openExternal(uri);
    expect(native.openUrl).not.toHaveBeenCalled();
  });

  it("opens browser links without access to the originating window", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    await openExternal("http://localhost:3000");
    expect(open).toHaveBeenCalledWith("http://localhost:3000", "_blank", "noopener,noreferrer");
    open.mockRestore();
  });
});
