import { describe, expect, it } from "vitest";
import { CallRoom, MAX_CONCURRENT_CALLS, MAX_CONCURRENT_CHANNELS, type CallLimit, nextBatch, runConcurrently, toolBatches } from "./batches";
import { concurrencySafe, limitOf, toolKind } from "./tools";

describe("which calls run together (§13.1)", () => {
  const safe = (name: string, args: Record<string, unknown> = {}) => concurrencySafe(toolKind(name), args);

  it("counts the tools that only read, and run_command only for a command line that only reads", () => {
    for (const name of ["read_terminal", "read_file", "grep", "glob", "web_search", "fetch_url", "read_skill", "search_tools"]) expect(safe(name), name).toBe(true);
    expect(safe("run_command", { command: "ls -la /etc && git -C /srv/app log -5" })).toBe(true);
    for (const command of ["systemctl restart nginx", "rm -f /tmp/x", "cat a > b", "sudo ls"]) expect(safe("run_command", { command }), command).toBe(false);
    for (const name of ["edit_file", "write_file", "send_input", "mcp__github__list_issues", "call_mcp_tool", "no_such_tool"]) expect(safe(name), name).toBe(false);
  });

  it("counts the tools that open an exec channel, and web searches, toward their limits", () => {
    const limits = (names: string[]) => names.map((n) => limitOf(toolKind(n)));
    expect(limits(["run_command", "read_file", "edit_file", "write_file", "grep", "glob"])).toEqual(Array(6).fill("channel"));
    expect(limitOf(toolKind("web_search"))).toBe("search");
    // AI-14: the chat provider's own search takes no such limit.
    expect(limitOf(toolKind("web_search"), true)).toBeNull();
    expect(limitOf(toolKind("grep"), true)).toBe("channel");
    expect(limits(["read_terminal", "send_input", "fetch_url", "read_skill", "search_tools", "mcp__a__b"])).toEqual(Array(6).fill(null));
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
    const limit = (c: string): CallLimit | null => (channel(c) ? "channel" : c.startsWith("search") ? "search" : null);
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
    return { running, started, finish, limit, run, most: () => most, mostChannels: () => mostChannels };
  }
  const flush = () => new Promise((r) => setTimeout(r, 0));

  it("runs at most four calls that open an exec channel at once, and up to ten in all, starting them in order", async () => {
    const h = harness();
    const calls = [...Array.from({ length: 6 }, (_, i) => `ch${i}`), ...Array.from({ length: 12 }, (_, i) => `web${i}`)];
    const done = runConcurrently(calls, h.limit, h.run, () => true);
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
    expect(await done).toBe(calls.length);
    expect(h.started).toEqual(calls);
    expect(h.most()).toBe(MAX_CONCURRENT_CALLS);
    expect(h.mostChannels()).toBe(MAX_CONCURRENT_CHANNELS);
  });

  it("runs one web search at a time, beside the other calls", async () => {
    const h = harness();
    const calls = ["search0", "web0", "search1", "ch0", "search2"];
    const done = runConcurrently(calls, h.limit, h.run, () => true);
    await flush();
    // search1 waits for search0, and the calls after it wait for it to start.
    expect(h.started).toEqual(["search0", "web0"]);
    h.finish.get("search0")!();
    await flush();
    expect(h.started).toEqual(["search0", "web0", "search1", "ch0"]);
    h.finish.get("search1")!();
    await flush();
    expect(h.started).toEqual(calls);
    for (const c of ["web0", "ch0", "search2"]) h.finish.get(c)!();
    expect(await done).toBe(calls.length);
  });

  it("starts no more calls once one may not start, and waits for the ones it started", async () => {
    const h = harness();
    let going = true;
    let settled = false;
    const done = runConcurrently(["ch0", "ch1", "ch2", "ch3", "ch4", "ch5"], h.limit, h.run, () => going).then((started) => {
      settled = true;
      return started;
    });
    await flush();
    going = false;
    h.finish.get("ch0")!();
    await flush();
    expect(h.started).toEqual(["ch0", "ch1", "ch2", "ch3"]);
    expect(settled).toBe(false);
    for (const c of ["ch1", "ch2", "ch3"]) h.finish.get(c)!();
    expect(await done).toBe(4);
    expect(settled).toBe(true);
  });

  it("starts the next call that opens a channel once a running call releases its own", async () => {
    const started: string[] = [];
    const releases = new Map<string, () => void>();
    const finish = new Map<string, () => void>();
    const run = (c: string, release: () => void) =>
      new Promise<void>((resolve) => {
        started.push(c);
        releases.set(c, release);
        finish.set(c, resolve);
      });
    const calls = ["ch0", "ch1", "ch2", "ch3", "ch4", "ch5"];
    const done = runConcurrently(calls, () => "channel", run, () => true);
    await flush();
    expect(started).toEqual(["ch0", "ch1", "ch2", "ch3"]);
    // ch1's tool ended, but its call waits for ch0's result: its channel is free.
    releases.get("ch1")!();
    releases.get("ch1")!();
    await flush();
    expect(started).toEqual(["ch0", "ch1", "ch2", "ch3", "ch4"]);
    // A call that settles after releasing frees nothing more.
    finish.get("ch1")!();
    await flush();
    expect(started).toHaveLength(5);
    finish.get("ch0")!();
    await flush();
    expect(started).toEqual(calls);
    for (const c of ["ch2", "ch3", "ch4", "ch5"]) finish.get(c)!();
    expect(await done).toBe(6);
  });

  it("shares a room's channels with the other batches and calls that take room from it", async () => {
    const h = harness();
    const room = new CallRoom();
    const lone = room.take("channel");
    const first = runConcurrently(["ch0", "ch1", "ch2", "ch3"], h.limit, h.run, () => true, room);
    const second = runConcurrently(["ch4", "ch5"], h.limit, h.run, () => true, room);
    await flush();
    // The call that runs alone holds one of the four channels.
    expect(h.started).toEqual(["ch0", "ch1", "ch2"]);
    lone();
    lone();
    await flush();
    expect(h.started).toEqual(["ch0", "ch1", "ch2", "ch3"]);
    expect(room.full("channel")).toBe(true);
    h.finish.get("ch1")!();
    await flush();
    expect(h.started).toEqual(["ch0", "ch1", "ch2", "ch3", "ch4"]);
    for (const c of ["ch0", "ch2", "ch3", "ch4"]) h.finish.get(c)!();
    await flush();
    h.finish.get("ch5")!();
    expect([await first, await second]).toEqual([4, 2]);
    expect(h.mostChannels()).toBe(MAX_CONCURRENT_CHANNELS);
    expect(room.full("channel")).toBe(false);
  });

  it("lets a call that waits for room give up once woken", async () => {
    const room = new CallRoom();
    const held = Array.from({ length: MAX_CONCURRENT_CHANNELS }, () => room.take("channel"));
    let waits = true;
    const entered = room.enter("channel", () => waits);
    let settled = false;
    void entered.then(() => (settled = true));
    await flush();
    expect(settled).toBe(false);
    waits = false;
    room.wake();
    expect(await entered).toBeNull();
    // It took nothing: the room is as full as before, and has room again once a call gives its back.
    expect(room.full("channel")).toBe(true);
    held[0]();
    expect(await room.enter("channel", () => true)).toBeTypeOf("function");
  });

  it("goes on past a call that fails", async () => {
    const ran: string[] = [];
    await runConcurrently(
      ["a", "b", "c"],
      () => null,
      async (c) => {
        ran.push(c);
        if (c === "a") throw new Error("failed");
      },
      () => true,
    );
    expect(ran).toEqual(["a", "b", "c"]);
  });
});
