import { beforeEach, describe, expect, it, vi } from "vitest";
import { useApp } from "@/app/store";
import { confirmMultilinePaste } from "./paste";
import { schemeFor, searchColors } from "./schemes";
import { LiveSession } from "./session";
import { useTermSettings } from "./theme";

vi.hoisted(() => {
  Object.defineProperty(window, "matchMedia", { value: () => ({ matches: false, addEventListener() {} }), configurable: true });
});

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    loadAddon = vi.fn();
    attachCustomKeyEventHandler = vi.fn();
    onData = vi.fn();
    onBinary = vi.fn();
    onResize = vi.fn();
    paste = vi.fn();
    focus = vi.fn();
    options = {};
    buffer = { active: { viewportY: 0 } };
    scrollToLine = vi.fn();
  },
}));
vi.mock("./paste", () => ({ confirmMultilinePaste: vi.fn(async () => true) }));
const writeBytes = vi.hoisted(() => vi.fn(async () => {}));
vi.mock("@/ipc/api", async (importActual) => {
  const actual = await importActual<typeof import("@/ipc/api")>();
  return {
    ...actual,
    api: new Proxy(actual.api, { get: (target, prop) => (prop === "ssh_write_bytes" ? writeBytes : Reflect.get(target, prop)) }),
  };
});

let session: LiveSession;
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(confirmMultilinePaste).mockResolvedValue(true);
  useApp.setState({ phase: "unlocked" });
  useTermSettings.setState((s) => ({ settings: { ...s.settings, confirm_multiline_paste: true } }));
  session = new LiveSession("paste-test", { hostId: "host", target: null });
  session.status = "connected";
});

describe("terminal paste", () => {
  it.each(["echo hello\n", "echo hello\r\n", "echo hello\r", "echo hello\n\n", "echo hello\r\n\r\n"])("trims single-line trailing newlines in %j without asking", async (text) => {
    await session.paste(text);
    expect(confirmMultilinePaste).not.toHaveBeenCalled();
    expect(session.term.paste).toHaveBeenCalledWith("echo hello");
  });

  it("preserves a single line's whitespace", async () => {
    await session.paste("  echo hello  \r\n");
    expect(session.term.paste).toHaveBeenCalledWith("  echo hello  ");
  });

  it.each(["echo one\necho two", "echo one\r\necho two\r\n", "echo one\recho two\r", "\necho two"])("confirms and preserves multi-line text %j", async (text) => {
    await session.paste(text);
    expect(confirmMultilinePaste).toHaveBeenCalledWith(text);
    expect(session.term.paste).toHaveBeenCalledWith(text);
  });

  it("does not paste when the user rejects multiple lines", async () => {
    vi.mocked(confirmMultilinePaste).mockResolvedValue(false);
    await session.paste("echo one\necho two");
    expect(session.term.paste).not.toHaveBeenCalled();
    expect(session.term.focus).toHaveBeenCalled();
  });

  it("still trims a single line when confirmation is disabled", async () => {
    useTermSettings.setState((s) => ({ settings: { ...s.settings, confirm_multiline_paste: false } }));
    await session.paste("echo hello\n");
    expect(session.term.paste).toHaveBeenCalledWith("echo hello");
    await session.paste("echo one\necho two\n");
    expect(session.term.paste).toHaveBeenLastCalledWith("echo one\necho two\n");
    expect(confirmMultilinePaste).not.toHaveBeenCalled();
  });

  it.each(["", "\n", "\r\n", "\n\n"])("ignores empty or newline-only text %j", async (text) => {
    await session.paste(text);
    expect(confirmMultilinePaste).not.toHaveBeenCalled();
    expect(session.term.paste).not.toHaveBeenCalled();
  });

  it("does not paste after the vault locks during confirmation", async () => {
    vi.mocked(confirmMultilinePaste).mockImplementation(async () => {
      useApp.setState({ phase: "locked" });
      return true;
    });
    await session.paste("echo one\necho two");
    expect(session.term.paste).not.toHaveBeenCalled();
  });

  it("does not paste after disconnecting during confirmation", async () => {
    vi.mocked(confirmMultilinePaste).mockImplementation(async () => {
      session.status = "disconnected";
      return true;
    });
    await session.paste("echo one\necho two");
    expect(session.term.paste).not.toHaveBeenCalled();
  });
});

describe("terminal binary input", () => {
  const binaryInput = () => vi.mocked(session.term.onBinary).mock.calls[0]![0];

  it("sends a legacy mouse report as its bytes, not UTF-8", () => {
    session.sessionId = "sid";
    // A left-button press at column 101, row 10: the column byte is 32 + 101 = 133.
    binaryInput()("[M *");
    expect(writeBytes).toHaveBeenCalledWith("sid", [27, 91, 77, 32, 133, 42]);
  });

  it("sends nothing while not connected", () => {
    session.sessionId = "sid";
    session.status = "disconnected";
    binaryInput()("[M *");
    expect(writeBytes).not.toHaveBeenCalled();
  });
});

describe("terminal search", () => {
  it("redraws highlighted matches in a new scheme's colors", () => {
    const findNext = vi.spyOn(session.search, "findNext").mockReturnValue(true);
    const clear = vi.spyOn(session.search, "clearDecorations").mockImplementation(() => {});
    const pick = (dark_scheme: string) => useTermSettings.setState((s) => ({ settings: { ...s.settings, theme: "dark", dark_scheme } }));
    pick("hatoba-dark");
    session.find("ops", "next");
    session.applySettings();
    expect(findNext).toHaveBeenCalledTimes(1);

    // The user scrolled up through the scrollback; the redraw leaves the view there.
    Object.assign(session.term.buffer.active, { viewportY: 120 });
    pick("dracula");
    session.applySettings();
    expect(clear).toHaveBeenCalled();
    expect(findNext).toHaveBeenLastCalledWith("ops", { incremental: true, decorations: searchColors(schemeFor("dark", "dracula")) });
    expect(session.term.scrollToLine).toHaveBeenCalledWith(120);

    // Closed find bar: nothing to redraw.
    session.endFind();
    pick("nord");
    session.applySettings();
    expect(findNext).toHaveBeenCalledTimes(2);
  });
});
