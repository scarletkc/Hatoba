/**
 * The line diff the approval card of `edit_file` and `write_file` shows (AI-39, AI-40): what the call
 * changes in the file, as a unified diff with a few lines of context.
 */

/** One line of the diff, with its number in the file before and after the change. */
export interface DiffLine {
  kind: "same" | "del" | "add";
  text: string;
  /** 1-based line number before the change; null for an added line. */
  old: number | null;
  /** 1-based line number after the change; null for a removed line. */
  new: number | null;
  /** The last line of a text that has no line break at its end, while the other text has one. */
  noEol?: boolean;
}

/** A run of unchanged lines the diff leaves out. */
export interface DiffGap {
  kind: "gap";
  count: number;
}

export type DiffRow = DiffLine | DiffGap;

/** The diff of the middle part is computed line by line only up to this many line pairs; past it, the middle shows as removed and added. */
const MAX_PAIRS = 4_000_000;

function split(text: string): string[] {
  if (text === "") return [];
  const lines = text.split("\n");
  if (text.endsWith("\n")) lines.pop();
  return lines;
}

/**
 * Every line of `before` and `after`, as unchanged, removed or added. The common start and end are
 * matched first, then the rest by longest common subsequence, so a small edit in a large file stays
 * cheap. A missing line break at the end of one text but not the other counts as a change of that line.
 */
export function diffLines(before: string, after: string): DiffLine[] {
  const a = split(before);
  const b = split(after);
  const eolDiffers = before !== "" && after !== "" && before.endsWith("\n") !== after.endsWith("\n");
  // Lines compare with their final line break when that is what differs.
  const keyA = a.map((l, i) => (eolDiffers && i === a.length - 1 && !before.endsWith("\n") ? `${l}\u0000` : l));
  const keyB = b.map((l, i) => (eolDiffers && i === b.length - 1 && !after.endsWith("\n") ? `${l}\u0000` : l));

  let start = 0;
  while (start < a.length && start < b.length && keyA[start] === keyB[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && keyA[endA - 1] === keyB[endB - 1]) {
    endA--;
    endB--;
  }

  const out: DiffLine[] = [];
  const same = (i: number, j: number) => out.push({ kind: "same", text: a[i], old: i + 1, new: j + 1 });
  const del = (i: number) => out.push({ kind: "del", text: a[i], old: i + 1, new: null, ...(keyA[i] !== a[i] ? { noEol: true } : {}) });
  const add = (j: number) => out.push({ kind: "add", text: b[j], old: null, new: j + 1, ...(keyB[j] !== b[j] ? { noEol: true } : {}) });

  for (let i = 0; i < start; i++) same(i, i);
  const n = endA - start;
  const m = endB - start;
  if (n * m > MAX_PAIRS) {
    for (let i = start; i < endA; i++) del(i);
    for (let j = start; j < endB; j++) add(j);
  } else if (n > 0 || m > 0) {
    // lcs[i][j]: the longest common subsequence of a[start + i..endA] and b[start + j..endB].
    const width = m + 1;
    const lcs = new Uint32Array((n + 1) * width);
    for (let i = n - 1; i >= 0; i--)
      for (let j = m - 1; j >= 0; j--)
        lcs[i * width + j] = keyA[start + i] === keyB[start + j] ? lcs[(i + 1) * width + j + 1] + 1 : Math.max(lcs[(i + 1) * width + j], lcs[i * width + j + 1]);
    let i = 0;
    let j = 0;
    while (i < n || j < m) {
      if (i < n && j < m && keyA[start + i] === keyB[start + j]) {
        same(start + i++, start + j++);
      } else if (i < n && (j === m || lcs[(i + 1) * width + j] >= lcs[i * width + j + 1])) {
        // On a tie the removal goes first, so a changed block lists its removed lines before its added ones, as `diff -u` does.
        del(start + i++);
      } else {
        add(start + j++);
      }
    }
  }
  for (let i = endA, j = endB; i < a.length; i++, j++) same(i, j);
  return out;
}

/** The diff as the card shows it: changed lines with `context` unchanged lines around them, and gaps for the rest. Empty when nothing changes. */
export function withContext(lines: DiffLine[], context = 3): DiffRow[] {
  const changed = lines.map((l) => l.kind !== "same");
  if (!changed.includes(true)) return [];
  const near = new Array<boolean>(lines.length).fill(false);
  changed.forEach((c, i) => {
    if (!c) return;
    for (let k = Math.max(0, i - context); k <= Math.min(lines.length - 1, i + context); k++) near[k] = true;
  });
  const rows: DiffRow[] = [];
  let gap = 0;
  lines.forEach((line, i) => {
    if (near[i]) {
      if (gap > 0) rows.push({ kind: "gap", count: gap });
      gap = 0;
      rows.push(line);
    } else {
      gap++;
    }
  });
  if (gap > 0) rows.push({ kind: "gap", count: gap });
  return rows;
}

/** How many lines the diff adds and removes. */
export function diffStats(lines: DiffLine[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const l of lines) {
    if (l.kind === "add") added++;
    else if (l.kind === "del") removed++;
  }
  return { added, removed };
}
