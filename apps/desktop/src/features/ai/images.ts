export const IMAGE_MAX_BYTES = 4 * 1024 * 1024;
export const IMAGES_MAX_BYTES = 8 * 1024 * 1024;
export const IMAGES_MAX_COUNT = 4;
export const IMAGE_TOKENS = 4096;

export function imageMime(bytes: Uint8Array): string | null {
  if (bytes.length >= 8 && [137, 80, 78, 71, 13, 10, 26, 10].every((value, index) => bytes[index] === value))
    return "image/png";
  if (bytes[0] === 255 && bytes[1] === 216 && bytes[2] === 255) return "image/jpeg";
  const head = String.fromCharCode(...bytes.subarray(0, 12));
  if (head.startsWith("GIF87a") || head.startsWith("GIF89a")) return "image/gif";
  if (head.startsWith("RIFF") && head.slice(8, 12) === "WEBP") return "image/webp";
  return null;
}

export function imageBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 8192)
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
  return btoa(binary);
}

export function imageUrl(image: { mime: string; text: string }): string {
  return `data:${image.mime};base64,${image.text}`;
}

export function estimatedImageText(text: string): string {
  let rest = text;
  let start = 0;
  let result = "";
  const seen = new Set<string>();
  let attachments = false;
  for (;;) {
    const opening =
      /^<(interrupted|host_change|terminal_change|shell_change|model_change|permission_mode|memory|file_revert|terminal_state|mcp_tools|mcp_change|terminal_selection|connection_diagnostics|pasted_text|file|image|host_file|host_directory|mcp_resource|skill|mcp_prompt)((?: [a-z_]+="[^"]*")*)>\n/.exec(
        rest,
      );
    if (!opening) break;
    const tag = opening[1];
    const note =
      /^(interrupted|host_change|terminal_change|shell_change|model_change|permission_mode|memory|file_revert|terminal_state|mcp_tools|mcp_change)$/.test(
        tag,
      );
    if (
      (note && (attachments || (seen.has(tag) && tag !== "file_revert"))) ||
      ((tag === "terminal_selection" || tag === "connection_diagnostics") && seen.has(tag))
    )
      break;
    const body = rest.slice(opening[0].length);
    const close = `\n</${tag}>`;
    let end = body.indexOf(close);
    while (end >= 0 && body[end + close.length] !== undefined && body[end + close.length] !== "\n")
      end = body.indexOf(close, end + 1);
    if (end < 0) break;
    const after = body.slice(end + close.length).replace(/^\n\n?/, "");
    if (tag === "image") {
      if (
        !/^ name="[^"]*" mime="image\/(png|jpeg|gif|webp)"$/.test(opening[2]) ||
        !/^[A-Za-z0-9+/]+={0,2}$/.test(body.slice(0, end))
      )
        break;
      result += text.slice(start, text.length - rest.length) + " ".repeat(IMAGE_TOKENS * 4);
      start = text.length - after.length;
    }
    seen.add(tag);
    attachments ||= !note;
    rest = after;
  }
  return result + text.slice(start);
}
