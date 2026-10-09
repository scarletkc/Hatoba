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
  /** Scrollback rows included above the screen. */
  scrollback: number;
}

/**
 * The screen plus up to `lines` scrollback rows above it, with soft-wrapped rows joined into one
 * line, trailing spaces trimmed and trailing blank lines dropped. A scrollback window that starts in
 * the middle of a wrapped line is widened to the line's start.
 */
export function readBuffer(buf: BufferLike, rows: number, lines: number): ScreenText {
  const screenStart = Math.max(0, buf.baseY);
  const end = Math.min(buf.length, screenStart + rows);
  let start = Math.max(0, screenStart - Math.max(0, lines));
  while (start > 0 && buf.getLine(start)?.isWrapped) start--;

  const out: string[] = [];
  for (let y = start; y < end; y++) {
    const line = buf.getLine(y);
    if (!line) continue;
    const continues = y + 1 < end && !!buf.getLine(y + 1)?.isWrapped;
    // Keep the spaces at a wrap point: they are part of the joined line.
    const text = line.translateToString(!continues);
    if (line.isWrapped && out.length > 0) out[out.length - 1] += text;
    else out.push(text);
  }
  while (out.length > 0 && out[out.length - 1].trim() === "") out.pop();
  return { text: out.map((l) => l.trimEnd()).join("\n"), alternate: buf.type === "alternate", scrollback: screenStart - start };
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
 * stored result to 16,000 characters anyway, §13.4.)
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
