import { useCallback, useEffect, useState } from "react";
import { Menu, toast, type MenuAnchor, type MenuEntry } from "@/components/overlay";
import { readClipboard, writeClipboard } from "@/features/terminal/clipboard";
import { useT, type MessageKey } from "@/i18n";
import { shortcutLabel, type Platform } from "@/lib/platform";

type TextField = HTMLInputElement | HTMLTextAreaElement;

const TEXT_INPUTS = new Set(["text", "search", "password", "email", "url", "tel", "number"]);

function textFieldOf(target: EventTarget | null): TextField | null {
  if (!(target instanceof Element)) return null;
  const el = target.closest("input, textarea");
  if (el instanceof HTMLTextAreaElement) return el;
  if (el instanceof HTMLInputElement && TEXT_INPUTS.has(el.type)) return el;
  return null;
}

function fieldEntries(field: TextField, t: (key: MessageKey) => string, platform: Platform): MenuEntry[] {
  // Null in a number field, which has no text selection to read.
  const start = field.selectionStart;
  const end = field.selectionEnd;
  const selected = start !== null && end !== null ? field.value.slice(start, end) : "";
  const editable = !field.readOnly && !field.disabled;
  const secret = field.type === "password";
  // Choosing an item takes the focus to the menu: put it and the selection back before editing.
  const restore = () => {
    field.focus();
    if (start !== null && end !== null) field.setSelectionRange(start, end);
  };
  const key = (k: string) => shortcutLabel(platform, `Ctrl+${k}`, `⌘${k}`);
  const denied = () => toast(t("terminal.clipboardDenied"), "error");
  return [
    {
      label: t("edit.cut"),
      hint: key("X"),
      disabled: !selected || !editable || secret,
      onSelect: () =>
        void writeClipboard(selected).then(() => {
          restore();
          document.execCommand("delete");
        }, denied),
    },
    { label: t("edit.copy"), hint: key("C"), disabled: !selected || secret, onSelect: () => void writeClipboard(selected).catch(denied) },
    { label: t("edit.paste"), hint: key("V"), disabled: !editable, onSelect: () => void paste(field, restore).catch(denied) },
    { kind: "separator" },
    {
      label: t("edit.selectAll"),
      hint: key("A"),
      disabled: !field.value,
      onSelect: () => {
        field.focus();
        field.select();
      },
    },
  ];
}

async function paste(field: TextField, restore: () => void) {
  const text = await readClipboard();
  if (!text) return;
  restore();
  // The field's own paste handling goes first: the AI input turns a long paste into an attachment.
  const data = new DataTransfer();
  data.setData("text/plain", text);
  const event = new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true });
  if (field.dispatchEvent(event)) document.execCommand("insertText", false, text);
}

/**
 * Right-click where the app shows no menu of its own: Cut, Copy, Paste and Select All in a text
 * field, Copy on selected text, and nothing elsewhere, instead of the web view's menu with Back,
 * Reload and Inspect. In development builds, Shift+right-click still opens the web view's menu.
 */
export function ContextMenuHost({ platform }: { platform: Platform }) {
  const t = useT();
  const [menu, setMenu] = useState<{ anchor: MenuAnchor; entries: MenuEntry[] } | null>(null);
  const close = useCallback(() => setMenu(null), []);

  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      // The hosts list, the terminal, the file browser and others have opened their own menu.
      if (e.defaultPrevented || (import.meta.env.DEV && e.shiftKey)) return;
      e.preventDefault();
      const field = textFieldOf(e.target);
      let entries: MenuEntry[] = [];
      if (field) entries = fieldEntries(field, t, platform);
      else {
        const selection = window.getSelection();
        const text = selection?.toString() ?? "";
        if (text.trim() && e.target instanceof Node && selection?.containsNode(e.target, true))
          entries = [{ label: t("edit.copy"), hint: shortcutLabel(platform, "Ctrl+C", "⌘C"), onSelect: () => void writeClipboard(text).catch(() => toast(t("terminal.clipboardDenied"), "error")) }];
      }
      setMenu(entries.length > 0 ? { anchor: { x: e.clientX, y: e.clientY }, entries } : null);
    };
    window.addEventListener("contextmenu", onContextMenu);
    return () => window.removeEventListener("contextmenu", onContextMenu);
  }, [t, platform]);

  if (!menu) return null;
  // Text fields include the master password field on the lock screen.
  return <Menu anchor={menu.anchor} entries={menu.entries} onClose={close} minWidth={160} whileLocked />;
}
