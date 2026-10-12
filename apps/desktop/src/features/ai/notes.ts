import type { MessageKey, Params } from "@/i18n";
import { modelName, type Note } from "./attachments";

type Tr = (key: MessageKey, params?: Params) => string;

/**
 * What a note Hatoba stored before a message says in the panel, where it is a divider above the
 * message, and in an export (§13.1, AI-05, AI-09, AI-41): that the user stopped the reply before it,
 * where the conversation moved, whether a terminal became attached or went away, which model it went
 * to, and what came before, the permission mode it was sent in when it changed (AI-16), which file's
 * change the user reverted, how many lines of the terminal's
 * screen it showed (AI-46, the title of its collapsed block), or how many MCP tools it listed (AI-45).
 * A host that no longer exists is named as deleted; a model by its name.
 */
export function noteLabel(t: Tr, note: Note): string {
  switch (note.kind) {
    case "interrupted":
      return t("ai.note.interrupted");
    case "host_change":
      return t("ai.note.hostChange", { to: note.to, from: note.from || t("ai.history.hostGone") });
    case "terminal_change":
      return t(note.to === "attached" ? "ai.note.terminalAttached" : "ai.note.terminalDetached");
    case "model_change":
      return t("ai.note.modelChange", { to: modelName(note.to), from: modelName(note.from) });
    case "permission_mode":
      return t(note.mode === "bypass" ? "ai.note.permissionBypass" : note.mode === "auto" ? "ai.note.permissionAuto" : "ai.note.permissionManual");
    case "file_revert":
      return t("ai.note.fileRevert", { path: note.path });
    case "terminal_state":
      return t("ai.note.screen", { n: note.lines });
    case "mcp_tools":
      return note.tools === 0 ? t("ai.note.mcpToolsNone") : t("ai.note.mcpTools", { n: note.tools });
  }
}

/** The note's full text, for a tooltip: a model's name with its ID. */
export function noteTitle(t: Tr, note: Note): string {
  if (note.kind !== "model_change") return noteLabel(t, note);
  return t("ai.note.modelChange", { to: note.to, from: note.from });
}
