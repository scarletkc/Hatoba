import { describe, expect, it, vi } from "vitest";
import { ZmodemTerminal, type ZmodemIo } from "./zmodem";

const rz = Uint8Array.from([..."**\x18B0100000023be50\r\x8a\x11"].map((c) => c.charCodeAt(0)));
function harness() {
  const io: ZmodemIo = {
    output: vi.fn(),
    send: vi.fn(async () => {}),
    allowed: () => true,
    view: vi.fn(),
    begin: vi.fn(async () => "file"),
    write: vi.fn(async () => {}),
    finish: vi.fn(async () => null),
    saved: vi.fn(),
    error: vi.fn(),
  };
  return { io, terminal: new ZmodemTerminal(io) };
}

describe("ZMODEM terminal routing", () => {
  it("passes ordinary UTF-8 output through unchanged", () => {
    const { io, terminal } = harness();
    const bytes = new TextEncoder().encode("日志\r\n[root@B ~]# ");
    terminal.consume(bytes);
    expect(Array.from(vi.mocked(io.output).mock.calls[0][0])).toEqual(Array.from(bytes));
    expect(terminal.busy).toBe(false);
  });

  it("recognizes rz across packet boundaries and suppresses its protocol frame", () => {
    const { io, terminal } = harness();
    terminal.consume(rz.subarray(0, 8));
    terminal.consume(rz.subarray(8));
    expect(terminal.busy).toBe(true);
    expect(io.view).toHaveBeenCalledWith({ phase: "upload", name: "", bytes: 0, total: 0 });
    terminal.cancel();
  });

  it("cancels and can detect another transfer on the same terminal", async () => {
    const { io, terminal } = harness();
    terminal.consume(rz);
    terminal.cancel();
    await new Promise((r) => setTimeout(r, 0));
    expect(terminal.busy).toBe(false);
    expect(io.send).toHaveBeenCalled();
    terminal.consume(new TextEncoder().encode("prompt\r\n"));
    terminal.consume(rz);
    expect(terminal.busy).toBe(true);
    terminal.cancel();
  });

  it("does not start a transfer while locked", () => {
    const { io, terminal } = harness();
    io.allowed = () => false;
    terminal.consume(rz);
    expect(terminal.busy).toBe(false);
    expect(io.begin).not.toHaveBeenCalled();
  });

  it("aborts an idle transfer rather than holding terminal input indefinitely", () => {
    vi.useFakeTimers();
    const { io, terminal } = harness();
    terminal.consume(rz);
    vi.advanceTimersByTime(120_000);
    expect(terminal.busy).toBe(false);
    expect(io.error).toHaveBeenCalled();
    vi.useRealTimers();
  });
});
