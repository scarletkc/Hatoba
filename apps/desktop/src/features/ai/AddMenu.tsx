import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { Icon, Spinner, Switch } from "@/components/controls";
import { useOverlaysLocked, windowSurface } from "@/components/overlay";
import { useT, type MessageKey } from "@/i18n";
import type { McpServerState } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { setMcpServerOff, setSlotScreen } from "./actions";
import { loadMcp, toolsMenuRows, toolsSummary, transportLabel, useMcp, type ToolsMenuRow } from "./mcp";
import { sendsScreen, useAi, type Slot } from "./store";
import s from "./AddMenu.module.css";

const STATE_LABEL: Record<Exclude<McpServerState, "running">, MessageKey> = {
  stopped: "ai.tools.stopped",
  starting: "ai.tools.starting",
  failed: "ai.tools.failed",
};

type View = "main" | "tools";

/**
 * The input area's + menu: Add Text Files (AI-35), Send Terminal Screen for this conversation
 * (AI-46) and, in its place when chosen, the tools menu (AI-30). The tools menu lists the MCP
 * servers enabled on this device with their state, and switches each one off for this conversation
 * until the app quits.
 */
export function AddButton({ slotId, slot, onAddFiles }: { slotId: string; slot: Slot; onAddFiles: () => void }) {
  const t = useT();
  const ref = useRef<HTMLButtonElement>(null);
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  const servers = useMcp((st) => st.servers);
  const status = useMcp((st) => st.status);
  const rows = useMemo(() => (servers ? toolsMenuRows(servers, status, slot.mcpOff) : []), [servers, status, slot.mcpOff]);
  const failed = toolsSummary(rows).failed;

  useEffect(() => {
    if (useMcp.getState().servers === null) void loadMcp();
  }, []);
  useEffect(() => setAnchor(null), [slotId]);

  const toggle = () => {
    if (anchor) return setAnchor(null);
    void loadMcp();
    if (ref.current) setAnchor(ref.current.getBoundingClientRect());
  };
  const label = failed ? `${t("ai.add")} · ${t("ai.tools.someFailed")}` : t("ai.add");

  return (
    <>
      <button
        ref={ref}
        type="button"
        className={cx(s.button, anchor && s.buttonOpen)}
        aria-haspopup="dialog"
        aria-expanded={!!anchor}
        aria-label={label}
        title={label}
        data-add-trigger
        onClick={toggle}
      >
        <Icon name="plus" size={14} />
        {failed && <span className={s.failedDot} />}
      </button>
      {anchor && (
        <AddPopover
          anchor={anchor}
          slotId={slotId}
          slot={slot}
          rows={rows}
          onAddFiles={onAddFiles}
          onClose={(restoreFocus) => {
            setAnchor(null);
            if (restoreFocus) ref.current?.focus();
          }}
        />
      )}
    </>
  );
}

function AddPopover({
  anchor,
  slotId,
  slot,
  rows,
  onAddFiles,
  onClose,
}: {
  anchor: DOMRect;
  slotId: string;
  slot: Slot;
  rows: ToolsMenuRow[];
  onAddFiles: () => void;
  onClose: (restoreFocus: boolean) => void;
}) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<View>("main");
  const loaded = useMcp((st) => st.servers !== null);
  const failed = useMcp((st) => st.failed);
  const terminal = useTabs((st) => st.tabs.some((x) => x.id === slotId && x.status === "connected"));
  const [pos, setPos] = useState({ left: anchor.left, top: anchor.top });
  const locked = useOverlaysLocked();

  useEffect(() => {
    if (locked) onClose(false);
  }, [locked, onClose]);

  // Above the button (the input area is at the bottom of the panel), kept inside the window.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const b = windowSurface();
    const left = Math.max(b.left + 8, Math.min(anchor.left, b.right - r.width - 8));
    let top = anchor.top - r.height - 6;
    if (top < b.top + 8) top = Math.min(anchor.bottom + 6, b.bottom - r.height - 8);
    setPos({ left, top: Math.max(b.top + 8, top) });
  }, [anchor, rows, loaded, failed, view, terminal]);

  // The first item has the focus when the menu opens, the back button in the tools menu, and the
  // tools item after going back.
  const back = useRef(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const target = view === "tools" ? el.querySelector<HTMLElement>("[data-back]") : back.current ? el.querySelector<HTMLElement>("[data-tools]") : items(el)[0];
    (target ?? el).focus({ preventScroll: true });
  }, [view]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const target = e.target as Element;
      // The button toggles the menu itself; closing here would make its click reopen it.
      if (ref.current && !ref.current.contains(target) && !target.closest?.("[data-add-trigger]")) onClose(false);
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
  }, [onClose]);

  // ↑/↓ move between the menu's items, ← goes back from the tools menu.
  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const el = ref.current;
    if (!el) return;
    if (e.key === "ArrowLeft" && view === "tools") {
      e.preventDefault();
      back.current = true;
      setView("main");
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const list = items(el);
    const at = list.indexOf(document.activeElement as HTMLElement);
    const next = e.key === "ArrowDown" ? at + 1 : at <= 0 ? list.length - 1 : at - 1;
    list[next % list.length]?.focus();
  };

  if (locked) return null;
  return createPortal(
    <div
      ref={ref}
      role="dialog"
      aria-label={view === "main" ? t("ai.add") : t("ai.tools.title")}
      tabIndex={-1}
      className={s.popover}
      style={pos}
      onKeyDown={onKeyDown}
    >
      {view === "main" ? (
        <MainView
          slotId={slotId}
          slot={slot}
          rows={rows}
          terminal={terminal}
          onAddFiles={() => {
            onClose(true);
            onAddFiles();
          }}
          onTools={() => setView("tools")}
        />
      ) : (
        <>
          {/* The item that opened it, as a row of the menu, takes it back. */}
          <button
            type="button"
            className={s.item}
            title={t("btn.back")}
            aria-label={t("btn.back")}
            data-item
            data-back
            onClick={() => {
              back.current = true;
              setView("main");
            }}
          >
            <Icon name="caret-left" className={s.itemIcon} />
            <span className={cx(s.itemLabel, s.itemTitle)}>{t("ai.tools")}</span>
          </button>
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
              onClose(false);
              useApp.getState().openSettings(true, "ai");
            }}
          >
            <Icon name="gear-six" className={s.itemIcon} />
            <span className={s.itemLabel}>{t("ai.tools.manage")}</span>
          </button>
        </>
      )}
    </div>,
    document.body,
  );
}

/** The menu's items that take the focus, in order: its buttons and its switches. */
function items(el: HTMLElement): HTMLElement[] {
  return Array.from(el.querySelectorAll<HTMLElement>("[data-item]:not(:disabled), [role=switch]:not(:disabled)"));
}

function MainView({
  slotId,
  slot,
  rows,
  terminal,
  onAddFiles,
  onTools,
}: {
  slotId: string;
  slot: Slot;
  rows: ToolsMenuRow[];
  /** Whether the tab is connected: without a terminal, the screen switch is off and disabled. */
  terminal: boolean;
  onAddFiles: () => void;
  onTools: () => void;
}) {
  const t = useT();
  const settings = useAi((st) => st.catalog.settings);
  const on = sendsScreen(slot, settings);
  const sum = toolsSummary(rows);
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
      <div className={s.separator} />
      <button
        type="button"
        className={s.item}
        data-item
        data-tools
        title={sum.total > 0 ? t("ai.tools.summary", { on: sum.on, total: sum.total }) : undefined}
        onClick={onTools}
      >
        <Icon name="plug" className={s.itemIcon} />
        <span className={s.itemLabel}>{t("ai.tools")}</span>
        {sum.failed && <span className={s.itemDot} title={t("ai.tools.someFailed")} />}
        {sum.total > 0 && (
          <span className={s.itemHint}>
            {sum.on}/{sum.total}
          </span>
        )}
        <Icon name="caret-right" size={12} className={s.itemCaret} />
      </button>
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
