/** Markdown building blocks for text that goes into a message or an export. */

/** A fenced code block whose fence is longer than any run of backticks in `text`, so the text cannot close it. */
export function fenced(text: string, lang = ""): string {
  let longest = 0;
  for (const m of text.matchAll(/`+/g)) longest = Math.max(longest, m[0].length);
  const fence = "`".repeat(Math.max(3, longest + 1));
  return `${fence}${lang}\n${text}\n${fence}`;
}

/** Every line as a block quote line. */
export function quoted(text: string): string {
  return text
    .split("\n")
    .map((line) => (line ? `> ${line}` : ">"))
    .join("\n");
}

/** Terminal text without trailing spaces on its lines, leading blank lines and trailing blank lines. */
export function cleanTerminalText(text: string): string {
  return text
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) => line.replace(/\s+$/, ""))
    .join("\n")
    .replace(/^\n+/, "")
    .replace(/\n+$/, "");
}

/**
 * AI-10 Ask AI: the input box's draft with the terminal selection added as a fenced block after it,
 * ending with a new line so the question can follow. An empty selection leaves the draft as it is.
 */
export function withSelection(draft: string, selection: string): string {
  const text = cleanTerminalText(selection);
  if (!text) return draft;
  const block = `${fenced(text)}\n`;
  const before = draft.replace(/\s+$/, "");
  return before ? `${before}\n\n${block}` : block;
}
