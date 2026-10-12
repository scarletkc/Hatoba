/**
 * Plain-text views of the terminal for `read_terminal` (AI-11) and `send_input` (AI-13). The xterm
 * buffer is read through the minimal interfaces below so the logic is testable without a terminal.
 */

export interface BufferLineLike {
  /** The row continues the previous one (soft wrap). */
  readonly isWrapped: boolean;
  translateToString(trimRight?: boolean): string;
}

export interface BufferLike {
  readonly type: "normal" | "alternate";
  readonly length: number;
  /** The first row of the screen (rows above it are scrollback). */
  readonly baseY: number;
  getLine(y: number): BufferLineLike | undefined;
}

export interface ScreenText {
  text: string;
  /** A full-screen program (vim, htop, …) is using the alternate screen. */
  alternate: boolean;
  /** Lines in `text`. */
  shown: number;
  /** Lines in the terminal up to the last one with text, `shown` included. */
  total: number;
}

/** The line that ends on row `last`, with its soft-wrapped rows joined and trailing spaces trimmed, and the row it starts on. */
function lineEndingAt(buf: BufferLike, last: number): { first: number; text: string } {
  let first = last;
  while (first > 0 && buf.getLine(first)?.isWrapped) first--;
  let text = "";
  // Keep the spaces at a wrap point: they are part of the joined line.
  for (let y = first; y <= last; y++) text += buf.getLine(y)?.translateToString(y === last) ?? "";
  return { first, text: text.trimEnd() };
}

/**
 * Up to `max` lines counted back from the last line with text at the bottom of the screen, reading
 * no line that ends above row `from`, and the last row above them. A line that starts above `from`
 * and wraps onto it is read whole.
 */
function readLines(buf: BufferLike, rows: number, max: number, from: number): { lines: string[]; above: number } {
  const end = Math.min(buf.length, Math.max(0, buf.baseY) + rows);
  const out: string[] = [];
  let y = end - 1;
  while (y >= from && out.length < max) {
    const line = lineEndingAt(buf, y);
    // Blank lines below the last one with text are dropped.
    if (out.length > 0 || line.text !== "") out.push(line.text);
    y = line.first - 1;
  }
  return { lines: out.reverse(), above: y };
}

/** Lines of the screen a message's `terminal_state` note shows, unless a full-screen program runs (AI-46). */
export const SCREEN_NOTE_LINES = 20;

/**
 * AI-11: the terminal's last `lines` lines, counted back from the last line with text across the
 * screen and the scrollback, with soft-wrapped rows joined into one line. Without `lines`, as many as
 * a `terminal_state` note shows: {@link SCREEN_NOTE_LINES}, or the whole screen of a full-screen program.
 */
export function readBuffer(buf: BufferLike, rows: number, lines: number | null): ScreenText {
  const alternate = buf.type === "alternate";
  const read = readLines(buf, rows, Math.max(1, lines ?? (alternate ? Infinity : SCREEN_NOTE_LINES)), 0);
  let total = read.lines.length;
  // Every row that does not continue the one above it starts a line.
  for (let y = 0; y <= read.above; y++) if (y === 0 || !buf.getLine(y)?.isWrapped) total++;
  return { text: read.lines.join("\n"), alternate, shown: read.lines.length, total };
}

/**
 * AI-46: the screen a message's `terminal_state` note shows: its last {@link SCREEN_NOTE_LINES} lines
 * up to the last one with text, or the whole screen while a full-screen program uses the alternate
 * screen. Scrollback is left out.
 */
export function screenNote(buf: BufferLike, rows: number): { text: string; alternate: boolean } {
  const alternate = buf.type === "alternate";
  const { lines } = readLines(buf, rows, alternate ? Infinity : SCREEN_NOTE_LINES, Math.max(0, buf.baseY));
  return { text: lines.join("\n"), alternate };
}

/** Lines of the screen a `send_input` call's judge reads, and its hard rules check (AI-47). */
export const SCREEN_TAIL_LINES = 10;

/**
 * AI-47: the screen's last {@link SCREEN_TAIL_LINES} lines up to the last one with text, which is where the
 * cursor waits at a prompt, so the judge can tell what a `send_input` call answers.
 */
export function screenTail(buf: BufferLike, rows: number): string {
  return readLines(buf, rows, SCREEN_TAIL_LINES, Math.max(0, buf.baseY)).lines.join("\n");
}

/**
 * Escape sequences: CSI (7- and 8-bit), OSC (ended by BEL or ST), DCS / SOS / PM / APC strings, and
 * two-character ESC sequences such as `ESC =` or charset selection `ESC ( B`.
 */
const ESCAPE = /\x1b\[[0-?]*[ -/]*[@-~]|\x9b[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?|\x1b[PX^_][^\x1b]*(?:\x1b\\)?|\x1b[ -/]*[0-~]?/y;

/** Removes ANSI escape sequences. */
export function stripAnsi(s: string): string {
  return s.replace(new RegExp(ESCAPE.source, "g"), "");
}

/**
 * Turns raw terminal output into the text a reader would see, line by line: escape sequences are
 * dropped, carriage returns and backspaces overwrite (progress bars keep their last state), "erase
 * line" and horizontal cursor moves are applied, and other control characters are dropped.
 */
export function cleanOutput(raw: string): string {
  const lines: string[] = [];
  let line: string[] = [];
  let col = 0;
  const put = (ch: string) => {
    while (line.length < col) line.push(" ");
    line[col++] = ch;
  };

  let i = 0;
  while (i < raw.length) {
    const ch = raw[i];
    if (ch === "\x1b" || ch === "\x9b") {
      ESCAPE.lastIndex = i;
      const m = ESCAPE.exec(raw);
      const seq = m?.[0] ?? ch;
      const csi = /^(?:\x1b\[|\x9b)([0-?]*)[ -/]*([@-~])$/.exec(seq);
      if (csi) {
        const n = Math.max(1, Number.parseInt(csi[1], 10) || 1);
        switch (csi[2]) {
          case "K": // erase in line
            if (csi[1] === "" || csi[1] === "0") line.length = Math.min(line.length, col);
            else if (csi[1] === "2") line = [];
            else if (csi[1] === "1") for (let c = 0; c < Math.min(col + 1, line.length); c++) line[c] = " ";
            break;
          case "G": // cursor horizontal absolute
            col = n - 1;
            break;
          case "C": // cursor forward
            col += n;
            break;
          case "D": // cursor back
            col = Math.max(0, col - n);
            break;
        }
      }
      i += seq.length;
      continue;
    }
    i += ch.length;
    if (ch === "\n") {
      lines.push(line.join(""));
      line = [];
      col = 0;
    } else if (ch === "\r") {
      col = 0;
    } else if (ch === "\b") {
      col = Math.max(0, col - 1);
    } else if (ch === "\t") {
      put("\t");
    } else if (ch >= " " && ch !== "\x7f" && !(ch >= "\x80" && ch <= "\x9f")) {
      put(ch);
    }
  }
  lines.push(line.join(""));
  const trimmed = lines.map((l) => l.trimEnd());
  while (trimmed.length > 0 && trimmed[trimmed.length - 1] === "") trimmed.pop();
  while (trimmed.length > 0 && trimmed[0] === "") trimmed.shift();
  return trimmed.join("\n");
}

/**
 * Collects terminal output for `send_input`, keeping the first `headMax` and the last `tailMax`
 * characters so a command that prints without end cannot use unbounded memory. (Rust trims the
 * stored result to 30,000 characters anyway, §13.4.)
 */
export class OutputCapture {
  private head = "";
  private tail = "";
  private dropped = 0;

  constructor(
    private readonly headMax = 32_000,
    private readonly tailMax = 64_000,
  ) {}

  push(chunk: string) {
    if (!chunk) return;
    if (!this.tail && this.dropped === 0 && this.head.length < this.headMax) {
      const room = this.headMax - this.head.length;
      this.head += chunk.slice(0, room);
      chunk = chunk.slice(room);
      if (!chunk) return;
    }
    this.tail += chunk;
    if (this.tail.length > this.tailMax * 2) {
      const cut = this.tail.length - this.tailMax;
      this.dropped += cut;
      this.tail = this.tail.slice(cut);
    }
  }

  get empty(): boolean {
    return !this.head && !this.tail;
  }

  /** The cleaned output, with a line saying how much was left out of the middle. */
  text(): string {
    if (this.dropped === 0) return cleanOutput(this.head + this.tail);
    return `${cleanOutput(this.head)}\n[… ${this.dropped} characters of output left out …]\n${cleanOutput(this.tail)}`;
  }
}
