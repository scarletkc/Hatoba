import type { ToolKind } from "../tools";

/** The icon of each tool, on its call's block and approval card. */
export const TOOL_ICON: Record<ToolKind, string> = {
  read_terminal: "terminal-window",
  run_command: "terminal",
  send_input: "keyboard",
  read_file: "file-text",
  edit_file: "note-pencil",
  write_file: "file-plus",
  grep: "file-magnifying-glass",
  glob: "files",
  web_search: "magnifying-glass",
  fetch_url: "globe",
  read_skill: "book-open",
  search_tools: "list-magnifying-glass",
  memory: "brain",
  mcp: "plug",
  unknown: "question",
};
