const ESCAPES: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };

export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (char) => ESCAPES[char]);
}

/** Renders the inline Markdown of the install steps: `code`, **bold**, and [links](url). */
export function inlineMarkdown(text: string): string {
  return escapeHtml(text)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\[([^\]]+)\]\((https:\/\/[^)\s]+)\)/g, '<a href="$2">$1</a>');
}

/**
 * Puts each code block of rendered Markdown in the frame of the CodeBlock component, whose Copy
 * button the docs page adds to it.
 */
export function frameCodeBlocks(html: string): string {
  return html.replace(/<pre[\s>][\s\S]*?<\/pre>/g, (pre) => `<div class="code-block" data-copy-scope>${pre}</div>`);
}
