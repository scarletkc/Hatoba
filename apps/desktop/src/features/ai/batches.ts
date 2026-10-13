/**
 * §13.1: how the tool calls of a response run. Consecutive calls that can run at the same time and need no
 * approval card form a batch whose calls run together, as Claude Code and Codex run them; every other call runs
 * alone. Batches run in order, so a call never overtakes an earlier one that changes something, and Rust stores
 * the results in the order of the calls, whichever finishes first.
 */

/** Most calls of a batch that run at once. */
export const MAX_CONCURRENT_CALLS = 10;

/**
 * Most calls that hold an exec channel of a tab's SSH connection at once, across the tab's conversations (AI-07):
 * OpenSSH allows 10 sessions on a connection by default (`MaxSessions`), and the shell, the resource usage readout
 * (TERM-12) and SFTP may hold some.
 */
export const MAX_CONCURRENT_CHANNELS = 4;

/**
 * Most `web_search` calls of a tab's conversations that run at once: search APIs limit how many requests come a
 * second (Brave's free plan allows one), and searches sent together would be turned away together, then sent again
 * together.
 */
export const MAX_CONCURRENT_SEARCHES = 1;

/** A limit that a call counts toward while it runs, besides {@link MAX_CONCURRENT_CALLS}. */
export type CallLimit = "channel" | "search";

const LIMITS: Record<CallLimit, number> = { channel: MAX_CONCURRENT_CHANNELS, search: MAX_CONCURRENT_SEARCHES };

/**
 * The room a tab has for calls that count toward a limit. The tab's conversations, also those in its background
 * (AI-07), run their tools on its one SSH connection, so every batch and every call that runs alone takes room
 * from the same {@link CallRoom}.
 */
export class CallRoom {
  private readonly held: Record<CallLimit, number> = { channel: 0, search: 0 };
  private woken: () => void = () => {};
  /** Settles when room is given back, or on {@link wake}, so that whoever waits for room looks again. */
  change: Promise<void>;

  constructor() {
    this.change = this.next();
  }

  private next(): Promise<void> {
    return new Promise((resolve) => (this.woken = resolve));
  }

  /** Whether a call of `limit` has to wait for room. */
  full(limit: CallLimit): boolean {
    return this.held[limit] >= LIMITS[limit];
  }

  /** Takes room for a call of `limit`; the function returned gives it back, once. */
  take(limit: CallLimit): () => void {
    this.held[limit]++;
    let holds = true;
    return () => {
      if (!holds) return;
      holds = false;
      this.held[limit]--;
      this.wake();
    };
  }

  /**
   * Waits for room for a call of `limit` while `waits` says the call still wants it, then takes it. Resolves with
   * the function that gives it back, or null once `waits` refused.
   */
  async enter(limit: CallLimit, waits: () => boolean): Promise<(() => void) | null> {
    while (this.full(limit) && waits()) await this.change;
    return waits() ? this.take(limit) : null;
  }

  /** Settles {@link change}, as a stop does for the calls of its turn that wait for room. */
  wake() {
    const woken = this.woken;
    this.change = this.next();
    woken();
  }
}

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
 * Runs `run` for the calls of a batch, starting them in order, at most {@link MAX_CONCURRENT_CALLS} at once, and
 * those that count toward a limit, by the limit that `limitOf` names, only while `room` has room for them: with the
 * calls of the tab's other conversations, at most {@link MAX_CONCURRENT_CHANNELS} that hold an exec channel and
 * {@link MAX_CONCURRENT_SEARCHES} web searches. A call holds its room until it settles or calls the `release` it is
 * given, as it does once its tool ended while its result waits for an earlier call's. A call starts only after every
 * earlier one has, so a call that waits for an earlier one's result to be stored never holds the room that one
 * needs. Starts no more once `starts` refuses the next call, also while it waits, and resolves, once every call it
 * started has settled, with how many it started.
 */
export async function runConcurrently<T>(
  calls: readonly T[],
  limitOf: (call: T) => CallLimit | null,
  run: (call: T, release: () => void) => Promise<void>,
  starts: (call: T) => boolean,
  room: CallRoom = new CallRoom(),
): Promise<number> {
  const running = new Set<Promise<void>>();
  let started = 0;
  // Settles when a call of this batch settles; `room.change`, when room is given back.
  let changed!: () => void;
  let change = new Promise<void>((resolve) => (changed = resolve));
  const notify = () => {
    changed();
    change = new Promise<void>((resolve) => (changed = resolve));
  };
  for (const call of calls) {
    const limit = limitOf(call);
    while ((running.size >= MAX_CONCURRENT_CALLS || (limit && room.full(limit))) && starts(call)) await Promise.race([change, room.change]);
    if (!starts(call)) break;
    started++;
    const release = limit ? room.take(limit) : () => {};
    const done: Promise<void> = run(call, release)
      .catch(() => {})
      .finally(() => {
        running.delete(done);
        release();
        notify();
      });
    running.add(done);
  }
  await Promise.all(running);
  return started;
}
