import type { LiveSession } from "@/features/terminal/session";
import { toAppError } from "@/ipc/api";
import type { AiToolStatus } from "@/ipc/types";
import { readBuffer, OutputCapture, type ScreenText } from "./terminalText";
import { keySequence, parseReadTerminal, parseSendInput } from "./tools";

/** The tools that need the tab's xterm instance (§13.1). Results are text for the model, so they stay in English. */

export interface ToolOutcome {
  status: AiToolStatus;
  content: string;
}

/** `send_input` returns once the output has been quiet this long (AI-13). */
const QUIET_MS = 1000;
const POLL_MS = 100;

export const NO_TAB = "No terminal tab is attached to this conversation, so terminal tools cannot run.";
export const DISCONNECTED = "The terminal tab is disconnected. Ask the user to reconnect it, then try again.";

function screenText(session: LiveSession, lines: number): ScreenText {
  const term = session.term;
  return readBuffer(term.buffer.active, term.rows, lines);
}

function describeScreen(s: ScreenText): string {
  const head = s.alternate
    ? "Alternate screen: yes. A full-screen program (such as vim or htop) is running; this is its current screen."
    : `Alternate screen: no. This is the visible screen with ${s.scrollback} line(s) of scrollback above it.`;
  return `${head}\n\n${s.text || "(the screen is empty)"}`;
}

/** AI-11: the screen and up to `lines` scrollback lines as plain text. */
export function readTerminal(session: LiveSession, json: string): ToolOutcome {
  const args = parseReadTerminal(json);
  if (!args.ok) return { status: "error", content: `Invalid arguments: ${args.error}` };
  if (session.status !== "connected") return { status: "error", content: DISCONNECTED };
  return { status: "ok", content: describeScreen(screenText(session, args.value.lines)) };
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
 * AI-13: types `text` and an optional key through the keyboard's path, waits until the output has
 * been quiet for 1 s or `wait_seconds` passed, and returns the output that appeared after the input,
 * or the whole screen when a full-screen program is running. Resolves to null when `signal` aborts
 * (the turn was stopped; Rust stores a cancelled result). Stopping never sends anything to the
 * terminal, so a started command keeps running (AI-18).
 */
export async function sendInput(session: LiveSession, json: string, signal: AbortSignal): Promise<ToolOutcome | null> {
  const parsed = parseSendInput(json);
  if (!parsed.ok) return { status: "error", content: `Invalid arguments: ${parsed.error}` };
  if (session.status !== "connected") return { status: "error", content: DISCONNECTED };
  const { text, key, waitSeconds } = parsed.value;

  const capture = new OutputCapture();
  let lastOutput: number;
  const off = session.onOutput((chunk) => {
    capture.push(chunk);
    lastOutput = Date.now();
  });
  let ending: "quiet" | "timeout" | "disconnected" = "quiet";
  try {
    const data = text + (key ? keySequence(key, session.term.modes.applicationCursorKeysMode) : "");
    try {
      await session.sendInput(data);
    } catch (e) {
      return { status: "error", content: `The input could not be sent: ${toAppError(e).detail}` };
    }
    const started = Date.now();
    lastOutput = started;
    for (;;) {
      if (!(await sleep(POLL_MS, signal))) return null;
      const now = Date.now();
      if (session.status !== "connected") {
        ending = "disconnected";
        break;
      }
      if (now - lastOutput >= QUIET_MS) break;
      if (now - started >= waitSeconds * 1000) {
        ending = "timeout";
        break;
      }
    }
  } finally {
    off();
  }

  await flush(session);
  if (signal.aborted) return null;
  if (ending !== "disconnected" && session.term.buffer.active.type === "alternate") {
    return { status: "ok", content: describeScreen(screenText(session, 0)) };
  }
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
      return { status: "error", content: `The terminal tab disconnected while waiting. Output before that:\n\n${output}` };
  }
}
