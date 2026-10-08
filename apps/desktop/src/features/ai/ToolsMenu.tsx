import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useApp } from "@/app/store";
import { Icon, Spinner, Switch } from "@/components/controls";
import { useT, type MessageKey } from "@/i18n";
import type { McpServerState } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { setMcpServerOff } from "./actions";
import { loadMcp, toolsMenuRows, toolsSummary, transportLabel, useMcp, type ToolsMenuRow } from "./mcp";
import s from "./ToolsMenu.module.css";

const STATE_LABEL: Record<Exclude<McpServerState, "running">, MessageKey> = {
  stopped: "ai.tools.stopped",
  starting: "ai.tools.starting",
  failed: "ai.tools.failed",
};

/**
 * AI-30: the input area's tools menu. It lists the MCP servers enabled on this device with their
 * state, and switches each one off for this conversation until the app quits.
 */
export function ToolsButton({ slotId, off }: { slotId: string; off: string[] }) {
  const t = useT();
  const ref = useRef<HTMLButtonElement>(null);
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  const servers = useMcp((st) => st.servers);
  const status = useMcp((st) => st.status);
  const rows = useMemo(() => (servers ? toolsMenuRows(servers, status, off) : []), [servers, status, off]);
  const sum = toolsSummary(rows);

  useEffect(() => {
    if (useMcp.getState().servers === null) void loadMcp();
  }, []);
  useEffect(() => setAnchor(null), [slotId]);

  const toggle = () => {
    if (anchor) return setAnchor(null);
    void loadMcp();
    if (ref.current) setAnchor(ref.current.getBoundingClientRect());
  };
  const label = sum.total > 0 ? t("ai.tools.summary", { on: sum.on, total: sum.total }) : t("ai.tools");

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
        data-tools-trigger
        onClick={toggle}
      >
        <Icon name="plug" size={13} />
        {sum.total > 0 && <span className={s.count}>{sum.on}</span>}
        {sum.failed && <span className={s.failedDot} />}
      </button>
      {anchor && (
        <ToolsPopover
          anchor={anchor}
          slotId={slotId}
          rows={rows}
          onClose={(restoreFocus) => {
            setAnchor(null);
            if (restoreFocus) ref.current?.focus();
          }}
        />
      )}
    </>
  );
}

function ToolsPopover({ anchor, slotId, rows, onClose }: { anchor: DOMRect; slotId: string; rows: ToolsMenuRow[]; onClose: (restoreFocus: boolean) => void }) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  const loaded = useMcp((st) => st.servers !== null);
  const failed = useMcp((st) => st.failed);
  const [pos, setPos] = useState({ left: anchor.left, top: anchor.top });

  // Above the button (the input area is at the bottom of the panel), kept inside the window.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const left = Math.max(8, Math.min(anchor.left, window.innerWidth - r.width - 8));
    let top = anchor.top - r.height - 6;
    if (top < 8) top = Math.min(anchor.bottom + 6, window.innerHeight - r.height - 8);
    setPos({ left, top: Math.max(8, top) });
  }, [anchor, rows, loaded, failed]);

  useEffect(() => {
    ref.current?.focus({ preventScroll: true });
  }, []);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const target = e.target as Element;
      // The button toggles the menu itself; closing here would make its click reopen it.
      if (ref.current && !ref.current.contains(target) && !target.closest?.("[data-tools-trigger]")) onClose(false);
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

  return createPortal(
    <div ref={ref} role="dialog" aria-label={t("ai.tools.title")} tabIndex={-1} className={s.popover} style={pos}>
      <div className={s.header}>{t("ai.tools.title")}</div>
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
        className={s.manage}
        onClick={() => {
          onClose(false);
          useApp.getState().openSettings(true, "ai");
        }}
      >
        <Icon name="gear-six" />
        {t("ai.tools.manage")}
      </button>
    </div>,
    document.body,
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
