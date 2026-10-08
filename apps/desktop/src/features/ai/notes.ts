import type { MessageKey, Params } from "@/i18n";
import { modelName, type Note } from "./attachments";

type Tr = (key: MessageKey, params?: Params) => string;

/**
 * What a note Hatoba stored before a message says in the panel, where it is a divider above the
 * message, and in an export (AI-05, AI-09): where the conversation moved, or which model it went to,
 * and what came before. A host that no longer exists is named as deleted; a model by its name.
 */
export function noteLabel(t: Tr, note: Note): string {
  if (note.kind === "host_change") return t("ai.note.hostChange", { to: note.to, from: note.from || t("ai.history.hostGone") });
  return t("ai.note.modelChange", { to: modelName(note.to), from: modelName(note.from) });
}

/** The note's full text, for a tooltip: a model's name with its ID. */
export function noteTitle(t: Tr, note: Note): string {
  if (note.kind === "host_change") return noteLabel(t, note);
  return t("ai.note.modelChange", { to: note.to, from: note.from });
}
