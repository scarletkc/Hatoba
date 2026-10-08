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
