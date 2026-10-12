import { describe, expect, it } from "vitest";
import { MAX_CONCURRENT_CALLS, MAX_CONCURRENT_CHANNELS, nextBatch, runConcurrently, toolBatches } from "./batches";
import { concurrencySafe, toolKind, usesChannel } from "./tools";

describe("which calls run together (§13.1)", () => {
  const safe = (name: string, args: Record<string, unknown> = {}) => concurrencySafe(toolKind(name), args);

  it("counts the tools that only read, and run_command only for a command line that only reads", () => {
    for (const name of ["read_terminal", "read_file", "grep", "glob", "web_search", "fetch_url", "read_skill", "search_tools"]) expect(safe(name), name).toBe(true);
    expect(safe("run_command", { command: "ls -la /etc && git -C /srv/app log -5" })).toBe(true);
    for (const command of ["systemctl restart nginx", "rm -f /tmp/x", "cat a > b", "sudo ls"]) expect(safe("run_command", { command }), command).toBe(false);
    for (const name of ["edit_file", "write_file", "send_input", "mcp__github__list_issues", "call_mcp_tool", "no_such_tool"]) expect(safe(name), name).toBe(false);
  });

  it("names the tools that open an exec channel", () => {
    expect(["run_command", "read_file", "edit_file", "write_file", "grep", "glob"].every((n) => usesChannel(toolKind(n)))).toBe(true);
    expect(["read_terminal", "send_input", "web_search", "fetch_url", "read_skill", "search_tools", "mcp__a__b"].some((n) => usesChannel(toolKind(n)))).toBe(false);
  });

  it("splits calls into runs of calls that join, with every other call alone, in order", () => {
    const joins = (c: string) => c.startsWith("r");
    expect(toolBatches(["r1", "r2", "w1", "r3", "w2", "w3", "r4", "r5", "r6"], joins)).toEqual([["r1", "r2"], ["w1"], ["r3"], ["w2"], ["w3"], ["r4", "r5", "r6"]]);
    expect(toolBatches(["w1"], joins)).toEqual([["w1"]]);
    expect(toolBatches([], joins)).toEqual([]);
    expect(nextBatch(["w1", "r1"], joins)).toEqual(["w1"]);
    expect(nextBatch(["r1", "r2", "w1"], joins)).toEqual(["r1", "r2"]);
  });
});

describe("runConcurrently", () => {
  /** Calls that settle when the test says, recording how many run at once. */
  function harness() {
    const running = new Set<string>();
    const started: string[] = [];
    const finish = new Map<string, () => void>();
    let most = 0;
    let mostChannels = 0;
    const channel = (c: string) => c.startsWith("ch");
    const run = (c: string) =>
      new Promise<void>((resolve) => {
        started.push(c);
        running.add(c);
        most = Math.max(most, running.size);
        mostChannels = Math.max(mostChannels, [...running].filter(channel).length);
        finish.set(c, () => {
          running.delete(c);
          resolve();
        });
      });
    return { running, started, finish, channel, run, most: () => most, mostChannels: () => mostChannels };
  }
  const flush = () => new Promise((r) => setTimeout(r, 0));

  it("runs at most four calls that open an exec channel at once, and up to ten in all, starting them in order", async () => {
    const h = harness();
    const calls = [...Array.from({ length: 6 }, (_, i) => `ch${i}`), ...Array.from({ length: 12 }, (_, i) => `web${i}`)];
    const done = runConcurrently(calls, h.channel, h.run, () => true);
    await flush();
    // The fifth channel call waits for a channel, and the calls after it wait for it to start.
    expect(h.started).toEqual(["ch0", "ch1", "ch2", "ch3"]);
    h.finish.get("ch0")!();
    await flush();
    expect(h.started).toEqual(["ch0", "ch1", "ch2", "ch3", "ch4"]);
    h.finish.get("ch1")!();
    await flush();
    // ch2…ch5 and six web calls make ten.
    expect(h.started.length).toBe(12);
    expect(h.running.size).toBe(MAX_CONCURRENT_CALLS);
    while (h.running.size > 0) {
      for (const c of [...h.running]) h.finish.get(c)!();
      await flush();
    }
    await done;
    expect(h.started).toEqual(calls);
    expect(h.most()).toBe(MAX_CONCURRENT_CALLS);
    expect(h.mostChannels()).toBe(MAX_CONCURRENT_CHANNELS);
  });

  it("starts no more calls once told to stop, and waits for the ones it started", async () => {
    const h = harness();
    let going = true;
    let settled = false;
    const done = runConcurrently(["ch0", "ch1", "ch2", "ch3", "ch4", "ch5"], h.channel, h.run, () => going).then(() => (settled = true));
    await flush();
    going = false;
    h.finish.get("ch0")!();
    await flush();
    expect(h.started).toEqual(["ch0", "ch1", "ch2", "ch3"]);
    expect(settled).toBe(false);
    for (const c of ["ch1", "ch2", "ch3"]) h.finish.get(c)!();
    await done;
    expect(settled).toBe(true);
  });

  it("goes on past a call that fails", async () => {
    const ran: string[] = [];
    await runConcurrently(
      ["a", "b", "c"],
      () => false,
      async (c) => {
        ran.push(c);
        if (c === "a") throw new Error("failed");
      },
      () => true,
    );
    expect(ran).toEqual(["a", "b", "c"]);
  });
});
