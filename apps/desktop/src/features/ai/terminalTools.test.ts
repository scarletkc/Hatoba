import { afterEach, describe, expect, it, vi } from "vitest";
import type { LiveSession } from "@/features/terminal/session";
import { sendInput } from "./terminalTools";

afterEach(() => vi.useRealTimers());

function session(applicationCursor = false): LiveSession {
  return {
    status: "connected",
    sendInput: vi.fn().mockResolvedValue(undefined),
    onOutput: vi.fn(() => vi.fn()),
    term: { modes: { applicationCursorKeysMode: applicationCursor }, buffer: { active: { type: "normal" } }, write: (_text: string, done: () => void) => done() },
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

  it.each(["pagedown", "\u200d", "enter\r"])("rejects %j without sending text or the valid prefix, and explains the notation", async (key) => {
    const live = session();
    const result = await sendInput(live, JSON.stringify({ text: "hello", keys: ["enter", key] }), new AbortController().signal);
    expect(result?.status).toBe("error");
    expect(result?.content).toContain("ctrl+a");
    expect(result?.content).toContain("page_down");
    expect(result?.content).toContain("one printable character");
    expect(live.sendInput).not.toHaveBeenCalled();
  });
});
