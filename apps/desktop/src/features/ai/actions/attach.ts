/**
 * What goes with a slot's next message besides its text: the terminal selection (AI-10), a connection's diagnostics,
 * and long pastes and text files (AI-35).
 */

import { toast } from "@hatoba/ui";
import { useVaultData } from "@/app/data";
import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { t } from "@/i18n";
import { api } from "@/ipc/api";
import type { DroppedFile } from "@/ipc/types";
import {
  baseName,
  checkFile,
  chipShown,
  droppedFile,
  FILE_MAX_BYTES,
  fitsMessage,
  makeAttachment,
  makeDiagnostics,
  makePaste,
  MESSAGE_MAX_BYTES,
  nextSelection,
  textFile,
  type Attachment,
  type FileAttachment,
  type FileRefusal,
  type PasteAttachment,
  type SelectionAttachment,
} from "../attachments";
import { localName } from "../place";
import { pickInput } from "../queue";
import {
  getSlot,
  HOME_SLOT,
  NO_SELECTION,
  patchSlot,
  useAi,
  type InputChip,
  type PendingAttachment,
  type TabSelection,
} from "../store";
import { aiErrorMessage } from "./context";
import { focusInput, toggleAiPanel } from "./panel";

export function tabSelection(tabId: string): TabSelection {
  return useAi.getState().selections[tabId] ?? NO_SELECTION;
}

export function setTabSelection(tabId: string, update: (sel: TabSelection) => TabSelection) {
  useAi.setState((st) => {
    const cur = st.selections[tabId] ?? NO_SELECTION;
    const next = update(cur);
    return next === cur ? {} : { selections: { ...st.selections, [tabId]: next } };
  });
}

/** xterm's selection of a tab changed; `renewed`: it was cleared on the way, as a new drag does. */
export function noteSelection(tabId: string, text: string, renewed = false) {
  setTabSelection(tabId, (sel) => {
    const next = nextSelection(sel, text, renewed);
    return next === sel ? sel : { ...sel, ...next };
  });
}

/** × on the chip, or the chip was sent: hidden until the selection changes. */
export function hideSelection(tabId: string) {
  setTabSelection(tabId, (sel) => (sel.hidden === sel.seq ? sel : { ...sel, hidden: sel.seq }));
}

/** The host name the selection block carries: the tab host's display name, or a local tab's computer (§7.5). */
export function selectionHost(tabId: string): string {
  const tab = useTabs.getState().tabs.find((x) => x.id === tabId);
  if (!tab) return "";
  if (tab.local) return localName(useApp.getState().info.device_name);
  return useVaultData.getState().hosts.find((h) => h.id === tab.hostId)?.name ?? tab.title;
}

/** What the chip attaches to the next message, or null when it does not show. */
export function selectionAttachment(slotId: string): SelectionAttachment | null {
  if (slotId === HOME_SLOT) return null;
  const sel = tabSelection(slotId);
  return chipShown(sel, sel.hidden) ? makeAttachment(selectionHost(slotId), sel.text) : null;
}

/** AI-10 Ask AI: opens the panel on the tab, where the selection shows as the chip, and focuses the input. */
export function askAi(slotId: string, selection: string) {
  noteSelection(slotId, selection);
  setTabSelection(slotId, (sel) => (sel.hidden === null ? sel : { ...sel, hidden: null }));
  toggleAiPanel(true);
}

/**
 * Ask AI on the terminal's connection error card: attaches the connection's diagnostics to the
 * tab's next message, suggests a question when the input is empty (selected, so typing replaces
 * it), and opens the panel on the tab. Nothing is sent until the user sends it.
 */
export function askAiAboutConnection(tabId: string, host: string, text: string) {
  const diagnostics = makeDiagnostics(host, text);
  if (!diagnostics) return;
  useAi.setState((st) => ({ diagnostics: { ...st.diagnostics, [tabId]: diagnostics } }));
  suggestQuestion(tabId, t("ai.connection.question"));
}

/** Puts `question` in the tab's input when it is empty, selected so typing replaces it, and opens the panel on the tab. */
export function suggestQuestion(tabId: string, question: string) {
  const suggest = !getSlot(tabId).draft.trim();
  if (suggest) patchSlot(tabId, { draft: question });
  toggleAiPanel(true);
  focusInput(suggest);
}

/** The tab's own chips (AI-10): its diagnostics, then its selection while the chip shows. */
export function liveChips(slotId: string): InputChip[] {
  const diagnostics = slotId === HOME_SLOT ? undefined : useAi.getState().diagnostics[slotId];
  const selection = selectionAttachment(slotId);
  return [...(diagnostics ? [diagnostics] : []), ...(selection ? [selection] : [])];
}

// ───────────────────────── long pastes and text files (AI-35) ─────────────────────────

let extraSeq = 0;

/** A new id for an attachment an input area holds. */
export function extraId(): string {
  return `x${++extraSeq}`;
}

/** Everything that would go with the slot's next message, for the size cap. */
export function pendingAttachments(slotId: string): Attachment[] {
  const extras = getSlot(slotId).extras;
  return [...pickInput(liveChips(slotId), extras).chips, ...extras.map((x) => x.attachment)];
}

const kb = (bytes: number) => `${bytes / 1024} KB`;

/** The toast for an attachment that would pass the per-message cap. */
export function tooMuchMessage(): string {
  return t("ai.attach.total", { size: kb(MESSAGE_MAX_BYTES) });
}

/** Adds a paste or a file to the slot's next message, unless the message would pass the cap. */
function addExtra(slotId: string, attachment: PasteAttachment | FileAttachment): boolean {
  return addAttachment(slotId, attachment);
}

/**
 * Adds an attachment that is neither the tab's selection nor its diagnostics to the slot's next message, unless the
 * message would pass the cap (AI-35): a paste or a file at the end, what the `@` menu attached (AI-51) too, and the
 * skill or MCP prompt of a `/` command (AI-50) first, in place of one the same command attached before, so that
 * running the command again does not attach it twice.
 */
export function addAttachment(slotId: string, attachment: Exclude<Attachment, InputChip>, command?: string): boolean {
  const others = command ? getSlot(slotId).extras.filter((x) => x.command !== command) : getSlot(slotId).extras;
  const pending = [...pickInput(liveChips(slotId), others).chips, ...others.map((x) => x.attachment)];
  if (!fitsMessage(pending, attachment)) {
    toast(tooMuchMessage(), "error");
    return false;
  }
  const added: PendingAttachment = { id: extraId(), attachment, ...(command ? { command } : {}) };
  patchSlot(slotId, (s) => {
    const rest = command ? s.extras.filter((x) => x.command !== command) : s.extras;
    return { extras: command ? [added, ...rest] : [...rest, added] };
  });
  return true;
}

/** A long paste into the input box becomes an attachment. */
export function addPaste(slotId: string, text: string): boolean {
  const paste = makePaste(text);
  return paste ? addExtra(slotId, paste) : false;
}

export function removeExtra(slotId: string, id: string) {
  patchSlot(slotId, (s) => ({ extras: s.extras.filter((x) => x.id !== id) }));
}

function refusal(reason: FileRefusal, name: string): string {
  if (reason === "image") return t("ai.attach.image", { name });
  if (reason === "too_large") return t("ai.attach.tooLarge", { name, size: kb(FILE_MAX_BYTES) });
  if (reason === "unreadable") return t("ai.attach.readFailed", { name });
  return t("ai.attach.binary", { name });
}

/**
 * Reads files the user picked or dropped, in the WebView. UTF-8 text files come back; an image, a
 * file over the size cap, or one with NUL bytes or invalid UTF-8 is refused with a toast naming it.
 */
export async function readTextFiles(files: readonly File[]): Promise<FileAttachment[]> {
  const read: FileAttachment[] = [];
  for (const file of files) {
    const name = baseName(file.name);
    const early = checkFile(name, file.type, file.size);
    if (early && !early.ok) {
      toast(refusal(early.reason, name), "error");
      continue;
    }
    let bytes: Uint8Array;
    try {
      bytes = new Uint8Array(await file.arrayBuffer());
    } catch {
      toast(t("ai.attach.readFailed", { name }), "error");
      continue;
    }
    const result = textFile(name, bytes);
    if (result.ok) read.push(result.file);
    else toast(refusal(result.reason, name), "error");
  }
  return read;
}

/** Add Text Files in the + menu, or files dropped on the panel: they go with the slot's next message. */
export async function attachFiles(slotId: string, files: readonly File[]) {
  for (const file of await readTextFiles(files)) if (!addExtra(slotId, file)) break;
  focusInput();
}

/**
 * Files dropped on the panel in the desktop app, whose webview hands over paths only: Rust reads them
 * (only paths of the window's last drop) with the same rules, and they go with the slot's next message.
 */
export async function attachDroppedPaths(slotId: string, paths: readonly string[]) {
  let results: DroppedFile[];
  try {
    results = await api.ai_read_dropped_files([...paths]);
  } catch (e) {
    toast(aiErrorMessage(e), "error");
    return;
  }
  for (const result of results) {
    const read = droppedFile(result);
    if (!read.ok) toast(refusal(read.reason, read.name), "error");
    else if (!addExtra(slotId, read.file)) break;
  }
  focusInput();
}

/** × on the diagnostics chip, or the diagnostics were sent. */
export function removeDiagnostics(tabId: string) {
  useAi.setState((st) => {
    if (!st.diagnostics[tabId]) return {};
    const diagnostics = { ...st.diagnostics };
    delete diagnostics[tabId];
    return { diagnostics };
  });
}
