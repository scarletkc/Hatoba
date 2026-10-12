/**
 * §13.1: how the tool calls of a response run. Consecutive calls that can run at the same time and need no
 * approval card form a batch whose calls run together, as Claude Code and Codex run them; every other call runs
 * alone. Batches run in order, so a call never overtakes an earlier one that changes something, and Rust stores
 * the results in the order of the calls, whichever finishes first.
 */

/** Most calls of a batch that run at once. */
export const MAX_CONCURRENT_CALLS = 10;

/**
 * Most calls of a batch that hold an exec channel of the tab's SSH connection at once: OpenSSH allows 10 sessions
 * on a connection by default (`MaxSessions`), and the shell, the resource usage readout (TERM-12) and SFTP may
 * hold some.
 */
export const MAX_CONCURRENT_CHANNELS = 4;

/** The calls at the start of `calls` that run next: the run of calls that `joins`, or else the first call alone. */
export function nextBatch<T>(calls: readonly T[], joins: (call: T) => boolean): T[] {
  const end = calls.findIndex((c) => !joins(c));
  return calls.slice(0, end < 0 ? calls.length : Math.max(end, 1));
}

/** `calls` split into the batches they run in, in order. */
export function toolBatches<T>(calls: readonly T[], joins: (call: T) => boolean): T[][] {
  const batches: T[][] = [];
  for (let rest = calls; rest.length > 0; ) {
    const batch = nextBatch(rest, joins);
    batches.push(batch);
    rest = rest.slice(batch.length);
  }
  return batches;
}

/**
 * Runs `run` for the calls of a batch, starting them in order, at most {@link MAX_CONCURRENT_CALLS} at once and of
 * those at most {@link MAX_CONCURRENT_CHANNELS} that `usesChannel`. A call starts only after every earlier one has,
 * so a call that waits for an earlier one's result to be stored never holds the room that one needs. Starts no
 * more once `starts` refuses the next call, and resolves, once every call it started has settled, with how many it
 * started.
 */
export async function runConcurrently<T>(calls: readonly T[], usesChannel: (call: T) => boolean, run: (call: T) => Promise<void>, starts: (call: T) => boolean): Promise<number> {
  const running = new Set<Promise<void>>();
  let channels = 0;
  let started = 0;
  for (const call of calls) {
    const channel = usesChannel(call);
    while (running.size >= MAX_CONCURRENT_CALLS || (channel && channels >= MAX_CONCURRENT_CHANNELS)) await Promise.race(running);
    if (!starts(call)) break;
    started++;
    if (channel) channels++;
    const done: Promise<void> = run(call)
      .catch(() => {})
      .finally(() => {
        running.delete(done);
        if (channel) channels--;
      });
    running.add(done);
  }
  await Promise.all(running);
  return started;
}
