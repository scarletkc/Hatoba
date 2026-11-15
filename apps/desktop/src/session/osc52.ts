/** The longest base64 text taken from a program, about 750 KB once decoded. */
export const OSC52_MAX_BASE64 = 1024 * 1024;

/**
 * TERM-04: the text a program asks to put on the clipboard with OSC 52 (`Pc ; Pd`), or null when
 * there is nothing to copy: a query (`?`), which would hand the program what the clipboard holds,
 * a selection other than the clipboard, or data that is too long or not base64.
 */
export function osc52Text(data: string): string | null {
  const split = data.indexOf(";");
  if (split < 0) return null;
  // tmux names no selection; `c` is the clipboard, and a program may name several at once.
  const selections = data.slice(0, split);
  if (selections && !selections.includes("c")) return null;
  const encoded = data.slice(split + 1);
  if (encoded === "?" || encoded.length > OSC52_MAX_BASE64) return null;
  try {
    const bytes = Uint8Array.from(atob(encoded), (ch) => ch.charCodeAt(0));
    return new TextDecoder().decode(bytes) || null;
  } catch {
    return null;
  }
}
