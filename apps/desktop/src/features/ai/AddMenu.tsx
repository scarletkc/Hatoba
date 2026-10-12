import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { Icon, Spinner, Switch } from "@/components/controls";
import { useOverlaysLocked, windowSurface } from "@/components/overlay";
import { useT, type MessageKey } from "@/i18n";
import type { McpServerState } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { setMcpServerOff, setSlotScreen } from "./actions";
import { loadMcp, toolsMenuRows, transportLabel, useMcp, type ToolsMenuRow } from "./mcp";
import { sendsScreen, useAi, type Slot } from "./store";
import s from "./AddMenu.module.css";

const STATE_LABEL: Record<Exclude<McpServerState, "running">, MessageKey> = {
  stopped: "ai.tools.stopped",
  starting: "ai.tools.starting",
  failed: "ai.tools.failed",
};

/**
 * The input area's + menu: Add Text Files (AI-35) and Send Terminal Screen for this conversation (AI-46). The MCP
 * servers are in `/mcp` (AI-50), which `McpPopover` shows.
 */
export function AddButton({ slotId, slot, onAddFiles }: { slotId: string; slot: Slot; onAddFiles: () => void }) {
  const t = useT();
  const ref = useRef<HTMLButtonElement>(null);
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  // Without a connected terminal, the screen switch is off and disabled, with a note under it.
  const terminal = useTabs((st) => st.tabs.some((x) => x.id === slotId && x.status === "connected"));

  useEffect(() => setAnchor(null), [slotId]);

  const toggle = () => {
    if (anchor) return setAnchor(null);
    if (ref.current) setAnchor(ref.current.getBoundingClientRect());
  };

  return (
    <>
      <button
        ref={ref}
        type="button"
        className={cx(s.button, anchor && s.buttonOpen)}
        aria-haspopup="dialog"
        aria-expanded={!!anchor}
        aria-label={t("ai.add")}
        title={t("ai.add")}
        data-add-trigger
        onClick={toggle}
      >
        <Icon name="plus" size={14} />
      </button>
      {anchor && (
        <Popover
          anchor={anchor}
          label={t("ai.add")}
          trigger="[data-add-trigger]"
          layoutKey={String(terminal)}
          onClose={(restoreFocus) => {
            setAnchor(null);
            if (restoreFocus) ref.current?.focus();
          }}
        >
          {(close) => (
            <MainView
              slotId={slotId}
              slot={slot}
              terminal={terminal}
              onAddFiles={() => {
                close(true);
                onAddFiles();
              }}
            />
          )}
        </Popover>
      )}
    </>
  );
}

/**
 * AI-50 `/mcp`: the MCP servers enabled on this device with their state (AI-32) and, for one that failed, its error
 * and the last lines of its stderr, a switch that turns each off for this conversation until the app quits (AI-30), and
 * the way to Settings → AI. It opens above the input box.
 */
export function McpPopover({ slotId, slot, anchor, onClose }: { slotId: string; slot: Slot; anchor: DOMRect; onClose: (restoreFocus: boolean) => void }) {
  const t = useT();
  const servers = useMcp((st) => st.servers);
  const status = useMcp((st) => st.status);
  const loaded = servers !== null;
  const failed = useMcp((st) => st.failed);
  const rows = useMemo(() => (servers ? toolsMenuRows(servers, status, slot.mcpOff) : []), [servers, status, slot.mcpOff]);

  useEffect(() => {
    void loadMcp();
  }, []);

  return (
    <Popover anchor={anchor} label={t("ai.tools.title")} wide layoutKey={`${rows.length}:${loaded}:${failed}`} onClose={onClose}>
      {(close) => (
        <>
          <div className={cx(s.item, s.itemTitle, s.itemStatic)}>
            <Icon name="plug" className={s.itemIcon} />
            <span className={s.itemLabel}>{t("ai.tools.title")}</span>
          </div>
          <div className={s.separator} />
          {!loaded && !failed && (
            <div className={s.note}>
              <Spinner size={13} />
            </div>
          )}
          {failed && <div className={cx(s.note, s.noteError)}>{t("ai.tools.loadFailed")}</div>}
          {loaded && !failed && rows.length === 0 && <div className={s.note}>{t("ai.tools.empty")}</div>}
          {rows.map((row) => (
            <ServerRow key={row.server.id} row={row} onChange={(on) => setMcpServerOff(slotId, row.server.id, !on)} />
          ))}
          {rows.length > 0 && <div className={s.footnote}>{t("ai.tools.note")}</div>}
          <div className={s.separator} />
          <button
            type="button"
            className={s.item}
            data-item
            onClick={() => {
              close(false);
              useApp.getState().openSettings(true, "ai");
            }}
          >
            <Icon name="gear-six" className={s.itemIcon} />
            <span className={s.itemLabel}>{t("ai.tools.manage")}</span>
          </button>
        </>
      )}
    </Popover>
  );
}

/**
 * A menu of buttons and switches above `anchor`, kept inside the window, whose first item takes the focus. A click
 * outside it, Esc and the window losing focus close it; ↑/↓ move between its items. `trigger` is a selector for the
 * button that opened it, which toggles it itself. `layoutKey` changes when its height may have changed.
 */
function Popover({
  anchor,
  label,
  trigger,
  wide,
  layoutKey,
  onClose,
  children,
}: {
  anchor: DOMRect;
  label: string;
  trigger?: string;
  wide?: boolean;
  layoutKey?: string;
  onClose: (restoreFocus: boolean) => void;
  children: (close: (restoreFocus: boolean) => void) => ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: anchor.left, top: anchor.top });
  const locked = useOverlaysLocked();

  useEffect(() => {
    if (locked) onClose(false);
  }, [locked, onClose]);

  // Above the anchor (the input area is at the bottom of the panel), kept inside the window.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const b = windowSurface();
    const left = Math.max(b.left + 8, Math.min(anchor.left, b.right - r.width - 8));
    let top = anchor.top - r.height - 6;
    if (top < b.top + 8) top = Math.min(anchor.bottom + 6, b.bottom - r.height - 8);
    setPos({ left, top: Math.max(b.top + 8, top) });
  }, [anchor, layoutKey]);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    (items(el)[0] ?? el).focus({ preventScroll: true });
  }, []);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const target = e.target as Element;
      // The button toggles the menu itself; closing here would make its click reopen it.
      if (ref.current && !ref.current.contains(target) && !(trigger && target.closest?.(trigger))) onClose(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose(true);
      }
    };
    const close = () => onClose(false);
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", close);
    };
  }, [onClose, trigger]);

  // ↑/↓ move between the menu's items.
  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const el = ref.current;
    if (!el || (e.key !== "ArrowDown" && e.key !== "ArrowUp")) return;
    e.preventDefault();
    const list = items(el);
    const at = list.indexOf(document.activeElement as HTMLElement);
    const next = e.key === "ArrowDown" ? at + 1 : at <= 0 ? list.length - 1 : at - 1;
    list[next % list.length]?.focus();
  };

  if (locked) return null;
  return createPortal(
    <div ref={ref} role="dialog" aria-label={label} tabIndex={-1} className={cx(s.popover, wide && s.popoverWide)} style={pos} onKeyDown={onKeyDown}>
      {children(onClose)}
    </div>,
    document.body,
  );
}

/** The menu's items that take the focus, in order: its buttons and its switches. */
function items(el: HTMLElement): HTMLElement[] {
  return Array.from(el.querySelectorAll<HTMLElement>("[data-item]:not(:disabled), [role=switch]:not(:disabled)"));
}

function MainView({ slotId, slot, terminal, onAddFiles }: { slotId: string; slot: Slot; terminal: boolean; onAddFiles: () => void }) {
  const t = useT();
  const settings = useAi((st) => st.catalog.settings);
  const on = sendsScreen(slot, settings);
  return (
    <>
      <button type="button" className={s.item} data-item onClick={onAddFiles}>
        <Icon name="paperclip" className={s.itemIcon} />
        <span className={s.itemLabel}>{t("ai.attach")}</span>
      </button>
      {/* The row toggles too, a larger target than the switch. */}
      <div className={cx(s.item, !terminal && s.itemDisabled)} onClick={(e) => terminal && !(e.target as Element).closest("button") && setSlotScreen(slotId, !on)}>
        <Icon name="terminal-window" className={s.itemIcon} />
        <span className={s.itemText}>
          <span className={s.itemLabel}>{t("ai.screen")}</span>
          {!terminal && <span className={s.itemNote}>{t("ai.screen.noTerminal")}</span>}
        </span>
        <span className={s.itemSwitch}>
          <Switch checked={terminal && on} disabled={!terminal} label={t("ai.screen")} onChange={(v) => setSlotScreen(slotId, v)} />
        </span>
      </div>
    </>
  );
}

function ServerRow({ row, onChange }: { row: ToolsMenuRow; onChange: (on: boolean) => void }) {
  const t = useT();
  const failed = row.state === "failed";
  const state = !row.on ? t("ai.tools.off") : row.state === "running" ? t("ai.tools.running", { n: row.tools }) : t(STATE_LABEL[row.state]);
  return (
    <div className={cx(s.row, !row.on && s.rowOff)}>
      <span className={cx(s.dot, s[row.state])} />
      <div className={s.body}>
        <span className={s.name}>{row.server.name}</span>
        <span className={cx(s.transport, "selectable")}>{transportLabel(row.server.transport)}</span>
        <span className={cx(s.state, failed && row.on && s.stateFailed)}>{state}</span>
        {failed && row.error && <span className={cx(s.error, "selectable")}>{row.error}</span>}
        {failed && row.stderr.length > 0 && (
          <>
            <span className={s.stderrLabel}>{t("ai.tools.stderr")}</span>
            <pre className={cx(s.stderr, "selectable")}>{row.stderr.join("\n")}</pre>
          </>
        )}
      </div>
      <Switch checked={row.on} label={t("ai.tools.use", { name: row.server.name })} onChange={onChange} />
    </div>
  );
}
