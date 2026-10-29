import { toAppError } from "@/ipc/api";
import type { AiToolStatus } from "@/ipc/types";
import type { LiveSession } from "@/session/session";
import { readBuffer, OutputCapture, type ScreenText } from "./terminalText";
import { keySequence, parseReadTerminal, parseSendInput } from "../tools";

/** The tools that need the tab's xterm instance (§13.1). Results are text for the model, so they stay in English. */

export interface ToolOutcome {
  status: AiToolStatus;
  content: string;
}

/** `send_input` returns, and a waiting `read_terminal` reads, once the output has been quiet this long (AI-11, AI-13). */
const QUIET_MS = 1000;
const POLL_MS = 100;

export const NO_TAB = "No terminal tab is attached to this conversation, so terminal tools cannot run.";
export const DISCONNECTED = "The terminal tab is disconnected. Ask the user to reconnect it, then try again.";

/** AI-07: `send_input` while another conversation's turn on the tab holds its terminal, named by its title. */
export function terminalInUse(title: string): string {
  const other = title ? `another conversation on this tab ("${title}")` : "another conversation on this tab";
  return `The terminal is in use by ${other}, whose turn typed into it and is still running, so send_input cannot type into it until that turn ends. Commands that need no interaction can still run with run_command. Otherwise tell the user, and try again after that turn ends.`;
}

function screenText(session: LiveSession, lines: number | null): ScreenText {
  const term = session.term;
  return readBuffer(term.buffer.active, term.rows, lines);
}

function describeScreen(s: ScreenText): string {
  const { shown, total } = s;
  const head = s.alternate
    ? `Alternate screen: yes. A full-screen program (such as vim or htop) is running; ${shown < total ? `these are the last ${shown} of its screen's ${total} lines` : `this is its whole screen, ${total} line(s)`}.`
    : `Alternate screen: no. ${shown < total ? `These are the last ${shown} of the terminal's ${total} lines` : `This is all of the terminal's ${total} line(s)`}.`;
  return `${head}\n\n${s.text || "(the terminal is empty)"}`;
}

/**
 * AI-11: the terminal's last `lines` lines as plain text. With `wait_seconds` it first waits until new output has come
 * and been quiet for 1 s, or that time passed, and says which. Resolves to null when `signal` aborts (the turn was
 * stopped; Rust stores a cancelled result).
 */
export async function readTerminal(
  session: LiveSession,
  json: string,
  signal: AbortSignal,
): Promise<ToolOutcome | null> {
  const args = parseReadTerminal(json);
  if (!args.ok) return { status: "error", content: `Invalid arguments: ${args.error}` };
  if (session.status !== "connected") return { status: "error", content: DISCONNECTED };
  const { lines, waitSeconds } = args.value;
  let waited = "";
  if (waitSeconds > 0) {
    const watch = watchOutput(session);
    try {
      const ending = await waitQuiet(session, watch, waitSeconds, signal, true);
      if (ending === null) return null;
      if (ending === "disconnected") return { status: "error", content: DISCONNECTED };
      // Output that comes while xterm catches up shows on the screen, so it counts as having come.
      await flush(session);
      if (signal.aborted) return null;
      waited =
        ending === "quiet"
          ? "New output came, and it has been quiet for 1 s.\n\n"
          : watch.capture.empty
            ? `No new output within ${waitSeconds} s.\n\n`
            : `The output did not go quiet within ${waitSeconds} s, so a command may still be running.\n\n`;
    } finally {
      watch.off();
    }
  }
  return { status: "ok", content: waited + describeScreen(screenText(session, lines)) };
}

/** How a wait for the output ended: quiet for {@link QUIET_MS}, out of time, or with the tab disconnected. */
type Ending = "quiet" | "timeout" | "disconnected";

/** The terminal's output from now on, and when its last chunk came, until `off`. */
interface OutputWatch {
  capture: OutputCapture;
  last: number;
  off: () => void;
}

function watchOutput(session: LiveSession): OutputWatch {
  const watch: OutputWatch = { capture: new OutputCapture(), last: Date.now(), off: () => {} };
  watch.off = session.onOutput((chunk) => {
    watch.capture.push(chunk);
    watch.last = Date.now();
  });
  return watch;
}

/**
 * Waits from now until the output has been quiet for {@link QUIET_MS}, only after some output came when `afterOutput`,
 * or `waitSeconds` passed, or the tab disconnected. Null when `signal` aborts.
 */
async function waitQuiet(
  session: LiveSession,
  watch: OutputWatch,
  waitSeconds: number,
  signal: AbortSignal,
  afterOutput: boolean,
): Promise<Ending | null> {
  const started = Date.now();
  watch.last = started;
  for (;;) {
    if (!(await sleep(POLL_MS, signal))) return null;
    const now = Date.now();
    if (session.status !== "connected") return "disconnected";
    if ((!afterOutput || !watch.capture.empty) && now - watch.last >= QUIET_MS) return "quiet";
    if (now - started >= waitSeconds * 1000) return "timeout";
  }
}

/** Resolves after `ms`, or with false as soon as `signal` aborts. */
function sleep(ms: number, signal: AbortSignal): Promise<boolean> {
  return new Promise((resolve) => {
    if (signal.aborted) return resolve(false);
    const timer = window.setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve(true);
    }, ms);
    const onAbort = () => {
      window.clearTimeout(timer);
      resolve(false);
    };
    signal.addEventListener("abort", onAbort, { once: true });
  });
}

/** Waits until xterm has parsed everything written so far (bounded, in case the terminal is gone). */
function flush(session: LiveSession): Promise<void> {
  return new Promise((resolve) => {
    const timer = window.setTimeout(resolve, 500);
    try {
      session.term.write("", () => {
        window.clearTimeout(timer);
        resolve();
      });
    } catch {
      resolve();
    }
  });
}

/**
 * AI-13: types `text` and ordered keys through the keyboard's path, waits until the output has
 * been quiet for 1 s or `wait_seconds` passed, and returns the output that appeared after the input,
 * or the whole screen when a full-screen program is running. Resolves to null when `signal` aborts
 * (the turn was stopped; Rust stores a cancelled result). Stopping never sends anything to the
 * terminal, so a started command keeps running (AI-18). `onType` runs just before the input goes to the
 * terminal, and not for a call that types nothing, and the function it returns runs when the input could not be
 * sent (AI-07: the turn holds the terminal once it types).
 */
export async function sendInput(
  session: LiveSession,
  json: string,
  signal: AbortSignal,
  onType: () => () => void = () => () => {},
): Promise<ToolOutcome | null> {
  const parsed = parseSendInput(json);
  if (!parsed.ok) return { status: "error", content: `Invalid arguments: ${parsed.error}` };
  if (session.status !== "connected") return { status: "error", content: DISCONNECTED };
  const { text, keys, waitSeconds } = parsed.value;

  const watch = watchOutput(session);
  let ending: Ending | null;
  try {
    const data = text + keys.map((key) => keySequence(key, session.term.modes.applicationCursorKeysMode)).join("");
    const untype = onType();
    try {
      await session.sendInput(data);
    } catch (e) {
      untype();
      return { status: "error", content: `The input could not be sent: ${toAppError(e).detail}` };
    }
    ending = await waitQuiet(session, watch, waitSeconds, signal, false);
  } finally {
    watch.off();
  }
  if (ending === null) return null;

  await flush(session);
  if (signal.aborted) return null;
  if (ending !== "disconnected" && session.term.buffer.active.type === "alternate") {
    return { status: "ok", content: describeScreen(screenText(session, 0)) };
  }
  const { capture } = watch;
  const output = capture.empty ? "(no output)" : capture.text() || "(no visible output)";
  switch (ending) {
    case "quiet":
      return { status: "ok", content: `Output after the input:\n\n${output}` };
    case "timeout":
      return {
        status: "ok",
        content: `The output did not go quiet within ${waitSeconds} s, so the command may still be running. Output so far:\n\n${output}`,
      };
    case "disconnected":
      return {
        status: "error",
        content: `The terminal tab disconnected while waiting. Output before that:\n\n${output}`,
      };
  }
}
