import { afterEach, describe, expect, it, vi } from "vitest";
import type { LiveSession } from "@/session/session";
import { readTerminal, sendInput } from "./terminalTools";

afterEach(() => vi.useRealTimers());

/** A connected session whose screen shows `$ ` and whose output the test writes with `emit`. */
function liveScreen() {
  let listener: (chunk: string) => void = () => {};
  const line = { isWrapped: false, translateToString: () => "$ " };
  const live = {
    status: "connected",
    onOutput: vi.fn((fn: (chunk: string) => void) => {
      listener = fn;
      return vi.fn();
    }),
    term: {
      rows: 24,
      buffer: { active: { type: "normal", length: 1, baseY: 0, getLine: (y: number) => (y === 0 ? line : undefined) } },
      write: (_text: string, done: () => void) => done(),
    },
  } as unknown as LiveSession;
  return { live, emit: (chunk: string) => listener(chunk) };
}

describe("read_terminal waiting for output (AI-11)", () => {
  const screen = "Alternate screen: no. This is all of the terminal's 1 line(s).\n\n$";

  it("reads at once without wait_seconds", async () => {
    const { live } = liveScreen();
    expect(await readTerminal(live, "{}", new AbortController().signal)).toEqual({ status: "ok", content: screen });
    expect(live.onOutput).not.toHaveBeenCalled();
  });

  it("reads once new output has come and been quiet for a second", async () => {
    vi.useFakeTimers();
    const { live, emit } = liveScreen();
    const pending = readTerminal(live, '{"wait_seconds": 30}', new AbortController().signal);
    // Quiet with nothing new yet is no reason to read.
    await vi.advanceTimersByTimeAsync(5000);
    emit("Resolving deltas: 100% (472/472), done.\r\n$ ");
    await vi.advanceTimersByTimeAsync(1200);
    expect(await pending).toEqual({
      status: "ok",
      content: `New output came, and it has been quiet for 1 s.\n\n${screen}`,
    });
  });

  it("says when nothing came, or when the output never went quiet", async () => {
    vi.useFakeTimers();
    const quiet = liveScreen();
    const none = readTerminal(quiet.live, '{"wait_seconds": 2}', new AbortController().signal);
    await vi.advanceTimersByTimeAsync(2200);
    expect((await none)?.content).toBe(`No new output within 2 s.\n\n${screen}`);

    const busy = liveScreen();
    const pending = readTerminal(busy.live, '{"wait_seconds": 2}', new AbortController().signal);
    for (let i = 0; i < 22; i++) {
      busy.emit(`Receiving objects: ${i}%\r`);
      await vi.advanceTimersByTimeAsync(100);
    }
    expect((await pending)?.content).toBe(
      `The output did not go quiet within 2 s, so a command may still be running.\n\n${screen}`,
    );
  });

  it("resolves to null when the turn stops while it waits", async () => {
    vi.useFakeTimers();
    const { live } = liveScreen();
    const stop = new AbortController();
    const pending = readTerminal(live, '{"wait_seconds": 10}', stop.signal);
    await vi.advanceTimersByTimeAsync(300);
    stop.abort();
    expect(await pending).toBeNull();
  });
});

function session(applicationCursor = false): LiveSession {
  return {
    status: "connected",
    sendInput: vi.fn().mockResolvedValue(undefined),
    onOutput: vi.fn(() => vi.fn()),
    term: {
      modes: { applicationCursorKeysMode: applicationCursor },
      buffer: { active: { type: "normal" } },
      write: (_text: string, done: () => void) => done(),
    },
  } as unknown as LiveSession;
}

describe("send_input terminal delivery", () => {
  it.each([
    [{ keys: ["ctrl+x", "n"] }, "\x18n"],
    [{ keys: ["alt++", "+", "N"] }, "\x1b++N"],
    [{ text: "nano /tmp/example", key: "enter" }, "nano /tmp/example\r"],
    [{ text: "ls", key: null, keys: null }, "ls"],
    [{ key: "esc", keys: [":", "w", "q", "enter"] }, "\x1b:wq\r"],
    [{ keys: ["home", "f10"] }, "\x1bOH\x1b[21~"],
  ])("sends text and keys in order through the existing keyboard path", async (args, bytes) => {
    vi.useFakeTimers();
    const live = session(true);
    const pending = sendInput(live, JSON.stringify(args), new AbortController().signal);
    await vi.advanceTimersByTimeAsync(1200);
    expect(await pending).toEqual({ status: "ok", content: "Output after the input:\n\n(no output)" });
    expect(live.sendInput).toHaveBeenCalledExactlyOnceWith(bytes);
  });

  it.each(["pagedown", "\u200d", "enter\r"])(
    "rejects %j without sending text or the valid prefix, and explains the notation",
    async (key) => {
      const live = session();
      const result = await sendInput(
        live,
        JSON.stringify({ text: "hello", keys: ["enter", key] }),
        new AbortController().signal,
      );
      expect(result?.status).toBe("error");
      expect(result?.content).toContain("ctrl+a");
      expect(result?.content).toContain("page_down");
      expect(result?.content).toContain("one printable character");
      expect(live.sendInput).not.toHaveBeenCalled();
    },
  );
});
