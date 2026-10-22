import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { Icon, Spinner, Switch } from "@/components/controls";
import { overlayStyles, useExitAnimation, useOverlaysLocked, windowSurface } from "@/components/overlay";
import { ErrorText, HintText, ListRow, StatusLine } from "@/components/patterns";
import { useT, type MessageKey } from "@/i18n";
import type { McpServerState } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { setMcpServerOff, setSlotScreen } from "./actions";
import { loadSkills, useSkills } from "./CommandMenu";
import { failedServers, loadMcp, toolsMenuRows, transportLabel, useMcp, type ToolsMenuRow } from "./mcp";
import { sendsScreen, useAi, type Slot } from "./store";
import s from "./AddMenu.module.css";

/** The lists that take the + menu's place. */
type List = "skills" | "tools";
type View = "main" | List;

const STATE_LABEL: Record<Exclude<McpServerState, "running">, MessageKey> = {
  stopped: "ai.tools.stopped",
  starting: "ai.tools.starting",
  failed: "ai.tools.failed",
};

/**
 * The input area's + menu: Add Text Files (AI-35), Slash Commands, which starts the box with `/` and so opens the menu
 * of AI-50, Send Terminal Screen for this conversation (AI-46), and two lists that take the menu's place when chosen,
 * each with a first row that goes back: Skills, whose skills start the box with their command (AI-50), and MCP Tools,
 * the MCP servers that `/mcp` shows too (AI-30). `onCommand`: start the box with `/`, or with a skill's command.
 */
export function AddButton({
  slotId,
  slot,
  onAddFiles,
  onCommand,
}: {
  slotId: string;
  slot: Slot;
  onAddFiles: () => void;
  onCommand: (skill: string | null) => void;
}) {
  const t = useT();
  const ref = useRef<HTMLButtonElement>(null);
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  // Without a connected terminal, the screen switch is off and disabled, with a note under it.
  const terminal = useTabs((st) => st.tabs.some((x) => x.id === slotId && x.status === "connected"));
  const servers = useMcp((st) => st.servers);
  const status = useMcp((st) => st.status);
  const rows = useMemo(() => (servers ? toolsMenuRows(servers, status, slot.mcpOff) : []), [servers, status, slot.mcpOff]);
  // AI-32: a server the conversation uses failed.
  const failed = failedServers(rows).length > 0;
  const label = failed ? `${t("ai.add")} · ${t("ai.tools.someFailed")}` : t("ai.add");

  useEffect(() => setAnchor(null), [slotId]);

  const toggle = () => {
    if (anchor) return setAnchor(null);
    // Skills and MCP Tools show the skills and the servers as they are now.
    loadSkills();
    void loadMcp();
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
        aria-label={label}
        title={label}
        data-add-trigger
        onClick={toggle}
      >
        <Icon name="plus" size={14} />
        {failed && <span className={s.failedDot} />}
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
              rows={rows}
              terminal={terminal}
              onAddFiles={() => {
                close(true);
                onAddFiles();
              }}
              onCommand={(skill) => {
                close(false);
                onCommand(skill);
              }}
              onClose={close}
            />
          )}
        </Popover>
      )}
    </>
  );
}

/**
 * AI-50 `/mcp`: the MCP servers of `McpList`, above the input box.
 */
export function McpPopover({ slotId, slot, anchor, onClose }: { slotId: string; slot: Slot; anchor: DOMRect; onClose: (restoreFocus: boolean) => void }) {
  const t = useT();
  const servers = useMcp((st) => st.servers);
  const failed = useMcp((st) => st.failed);
  return (
    <Popover anchor={anchor} label={t("ai.tools.title")} layoutKey={`${servers?.length}:${failed}`} onClose={onClose}>
      {(close) => <McpList slotId={slotId} slot={slot} onManage={() => close(false)} />}
    </Popover>
  );
}

/**
 * The MCP servers enabled on this device with their state (AI-32) and, for one that failed, its error and the last
 * lines of its stderr, a switch that turns each off for this conversation until the app quits (AI-30), and the way to
 * Settings → AI. `/mcp` shows them, and the + menu's tools menu, whose first row (`head`) goes back. `onManage`: the
 * menu closes for the settings.
 */
function McpList({ slotId, slot, head, onManage }: { slotId: string; slot: Slot; head?: ReactNode; onManage: () => void }) {
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
    <>
      {head ?? (
        <div className={cx(s.item, s.itemStatic)} title={t("ai.tools.title")}>
          <Icon name="plug" className={s.itemIcon} />
          <span className={cx(s.itemLabel, s.itemTitle)}>{t("ai.tools")}</span>
        </div>
      )}
      <div className={s.separator} />
      {!loaded && !failed && (
        <div className={s.note}>
          <Spinner size={14} />
        </div>
      )}
      {failed && <ErrorText className={s.note}>{t("ai.tools.loadFailed")}</ErrorText>}
      {loaded && !failed && rows.length === 0 && <HintText className={s.note}>{t("ai.tools.empty")}</HintText>}
      {rows.map((row) => (
        <ServerRow key={row.server.id} row={row} onChange={(on) => setMcpServerOff(slotId, row.server.id, !on)} />
      ))}
      {rows.length > 0 && <HintText className={s.footnote}>{t("ai.tools.note")}</HintText>}
      <div className={s.separator} />
      <ManageItem onManage={onManage} />
    </>
  );
}

/** Settings → AI, where the skills and the MCP servers are managed. */
function ManageItem({ onManage }: { onManage: () => void }) {
  const t = useT();
  return (
    <button
      type="button"
      className={s.item}
      data-item
      onClick={() => {
        onManage();
        useApp.getState().openSettings(true, "ai");
      }}
    >
      <Icon name="gear-six" className={s.itemIcon} />
      <span className={s.itemLabel}>{t("ai.tools.manage")}</span>
    </button>
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
  layoutKey,
  onClose,
  children,
}: {
  anchor: DOMRect;
  label: string;
  trigger?: string;
  layoutKey?: string;
  onClose: (restoreFocus: boolean) => void;
  children: (close: (restoreFocus: boolean) => void) => ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: anchor.left, top: anchor.top, transformOrigin: "left bottom" });
  const locked = useOverlaysLocked();
  useExitAnimation(ref, !locked);

  useEffect(() => {
    if (locked) onClose(false);
  }, [locked, onClose]);

  // Above the anchor (the input area is at the bottom of the panel), kept inside the window, and placed again
  // whenever its height changes, as when a server's state or stderr arrives.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const place = () => {
      // Its layout size: the bounding box would take in the opening animation's scale.
      const r = { width: el.offsetWidth, height: el.offsetHeight };
      const b = windowSurface();
      const left = Math.max(b.left + 8, Math.min(anchor.left, b.right - r.width - 8));
      let top = anchor.top - r.height - 6;
      const below = top < b.top + 8;
      if (below) top = Math.min(anchor.bottom + 6, b.bottom - r.height - 8);
      setPos({ left, top: Math.max(b.top + 8, top), transformOrigin: below ? "left top" : "left bottom" });
    };
    place();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(place);
    observer.observe(el);
    return () => observer.disconnect();
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
    <div ref={ref} role="dialog" aria-label={label} tabIndex={-1} className={cx(s.popover, overlayStyles.popIn)} style={pos} onKeyDown={onKeyDown}>
      {children(onClose)}
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
  onCommand,
  onClose,
}: {
  slotId: string;
  slot: Slot;
  rows: ToolsMenuRow[];
  /** Whether the tab is connected: without a terminal, the screen switch is off and disabled. */
  terminal: boolean;
  onAddFiles: () => void;
  onCommand: (skill: string | null) => void;
  onClose: (restoreFocus: boolean) => void;
}) {
  const t = useT();
  const settings = useAi((st) => st.catalog.settings);
  const on = sendsScreen(slot, settings);
  const [view, setView] = useState<View>("main");
  const used = rows.filter((r) => r.on).length;
  const failed = failedServers(rows).length > 0;

  // A list's back row has the focus, and back in the menu, the item that opened the list.
  const opener = useRef<Record<List, HTMLButtonElement | null>>({ skills: null, tools: null });
  const backRow = useRef<HTMLButtonElement>(null);
  const left = useRef<List | null>(null);
  useEffect(() => {
    if (view !== "main") backRow.current?.focus({ preventScroll: true });
    else if (left.current) opener.current[left.current]?.focus({ preventScroll: true });
  }, [view]);
  const goBack = () => {
    if (view !== "main") left.current = view;
    setView("main");
  };

  if (view !== "main") {
    // The item that opened the list, as its first row, takes it back.
    const head = (
      <button ref={backRow} type="button" className={s.item} title={t("btn.back")} aria-label={t("btn.back")} data-item onClick={goBack}>
        <Icon name="caret-left" className={s.itemIcon} />
        <span className={cx(s.itemLabel, s.itemTitle)}>{view === "skills" ? t("ai.add.skills") : t("ai.tools")}</span>
      </button>
    );
    return (
      // ← goes back to the menu.
      <div
        className={s.view}
        onKeyDown={(e) => {
          if (e.key !== "ArrowLeft") return;
          e.preventDefault();
          goBack();
        }}
      >
        {view === "skills" ? (
          <SkillList head={head} onPick={(name) => onCommand(name)} onManage={() => onClose(false)} />
        ) : (
          <McpList slotId={slotId} slot={slot} head={head} onManage={() => onClose(false)} />
        )}
      </div>
    );
  }

  /** An item that opens a list in the menu's place, with → too. */
  const listItem = (list: List, icon: string, label: string, extra: ReactNode, title?: string) => (
    <button
      ref={(el) => {
        opener.current[list] = el;
      }}
      type="button"
      className={s.item}
      data-item
      title={title}
      onClick={() => setView(list)}
      onKeyDown={(e) => {
        if (e.key !== "ArrowRight") return;
        e.preventDefault();
        setView(list);
      }}
    >
      <Icon name={icon} className={s.itemIcon} />
      <span className={s.itemLabel}>{label}</span>
      {extra}
      <Icon name="caret-right" size={12} className={s.itemCaret} />
    </button>
  );

  return (
    <>
      <button type="button" className={s.item} data-item onClick={onAddFiles}>
        <Icon name="paperclip" className={s.itemIcon} />
        <span className={s.itemLabel}>{t("ai.attach")}</span>
      </button>
      <button type="button" className={s.item} data-item onClick={() => onCommand(null)}>
        <Icon name="terminal" className={s.itemIcon} />
        <span className={s.itemLabel}>{t("ai.add.slash")}</span>
        <span className={s.itemHint}>/</span>
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
      {listItem("skills", "book-open-text", t("ai.add.skills"), null)}
      {listItem(
        "tools",
        "plug",
        t("ai.tools"),
        <>
          {failed && <span className={s.itemDot} title={t("ai.tools.someFailed")} />}
          {rows.length > 0 && (
            <span className={s.itemHint}>
              {used}/{rows.length}
            </span>
          )}
        </>,
        rows.length > 0 ? t("ai.tools.summary", { on: used, total: rows.length }) : undefined,
      )}
    </>
  );
}

/** The + menu's Skills: the enabled skills, each of which starts the box with its command (AI-50). */
function SkillList({ head, onPick, onManage }: { head: ReactNode; onPick: (name: string) => void; onManage: () => void }) {
  const t = useT();
  const skills = useSkills((st) => st.skills);
  return (
    <>
      {head}
      <div className={s.separator} />
      {skills === null && (
        <div className={s.note}>
          <Spinner size={14} />
        </div>
      )}
      {skills?.length === 0 && <HintText className={s.note}>{t("ai.add.skillsEmpty")}</HintText>}
      {skills?.map((skill) => (
        <button key={skill.name} type="button" className={s.item} data-item title={skill.description || undefined} onClick={() => onPick(skill.name)}>
          <Icon name="book-open-text" className={s.itemIcon} />
          <span className={s.itemText}>
            <span className={s.itemLabel}>/{skill.name}</span>
            {skill.description && <span className={cx(s.itemNote, s.itemClamp)}>{skill.description}</span>}
          </span>
        </button>
      ))}
      <div className={s.separator} />
      <ManageItem onManage={onManage} />
    </>
  );
}

function ServerRow({ row, onChange }: { row: ToolsMenuRow; onChange: (on: boolean) => void }) {
  const t = useT();
  const failed = row.state === "failed";
  const state = !row.on ? t("ai.tools.off") : row.state === "running" ? t("ai.tools.running", { n: row.tools }) : t(STATE_LABEL[row.state]);
  return (
    <ListRow
      size="sm"
      interactive
      className={cx(s.row, !row.on && s.rowOff)}
      leading={<span className={cx(s.dot, s[row.state])} />}
      trailing={<Switch checked={row.on} label={t("ai.tools.use", { name: row.server.name })} onChange={onChange} />}
    >
      <span className={s.name}>{row.server.name}</span>
      <span className={cx(s.transport, "selectable")}>{transportLabel(row.server.transport)}</span>
      <StatusLine tone={failed && row.on ? "error" : "neutral"}>{state}</StatusLine>
      {failed && row.error && <ErrorText className={cx(s.error, "selectable")}>{row.error}</ErrorText>}
      {failed && row.stderr.length > 0 && (
        <>
          <span className={s.stderrLabel}>{t("ai.tools.stderr")}</span>
          <pre className={cx(s.stderr, "selectable")}>{row.stderr.join("\n")}</pre>
        </>
      )}
    </ListRow>
  );
}
