import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { errorMessage } from "@/app/errors";
import { Icon, Spinner } from "@/components/controls";
import { toast, useOverlaysLocked, type MenuAnchor } from "@/components/overlay";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { ForwardView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { writeClipboard } from "./clipboard";
import {
  markForwardFailed,
  markForwardRunning,
  markForwardStopped,
  useForwardRuns,
  type ForwardRun,
} from "./forwards";
import s from "./ForwardsPopover.module.css";

const hostPort = (host: string, port: number | string) => `${host.includes(":") ? `[${host}]` : host}:${port}`;

/**
 * The status bar's port forwarding popover (FWD-01/02): this host's saved forwards with their state
 * on this session, and start / stop / retry for each.
 */
export function ForwardsPopover({
  anchor,
  sessionId,
  hostId,
  onClose,
  onManage,
}: {
  anchor: MenuAnchor;
  sessionId: string;
  hostId: string;
  /** `restoreFocus` is true when closed from the keyboard, so typing can continue in the terminal. */
  onClose: (restoreFocus: boolean) => void;
  onManage: () => void;
}) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  const runs = useForwardRuns(sessionId);
  const [saved, setSaved] = useState<ForwardView[] | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [busy, setBusy] = useState<ReadonlySet<string>>(new Set());
  const [pos, setPos] = useState({ left: anchor.x, top: anchor.y });
  const locked = useOverlaysLocked();

  useEffect(() => {
    if (locked) onClose(false);
  }, [locked, onClose]);

  // Always read the list fresh: forwards are edited in the host editor, not here.
  useEffect(() => {
    let live = true;
    api
      .forwards_list(hostId)
      .then((list) => live && setSaved(list))
      .catch(() => live && setLoadFailed(true));
    return () => {
      live = false;
    };
  }, [hostId]);

  // Same placement rule as `Menu`: below the button, kept inside the window; re-measured as rows appear.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    let left = anchor.alignRight ? anchor.x - r.width : anchor.x;
    let top = anchor.y;
    left = Math.max(8, Math.min(left, window.innerWidth - r.width - 8));
    if (top + r.height > window.innerHeight - 8) top = Math.max(8, anchor.y - r.height - 8);
    setPos({ left, top });
  }, [anchor.x, anchor.y, anchor.alignRight, saved, loadFailed, runs]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const target = e.target as Element;
      // The status bar button toggles the popover itself; closing here would make its click reopen it.
      if (ref.current && !ref.current.contains(target) && !target.closest?.("[data-forwards-trigger]")) onClose(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
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

  const setBusyFor = (id: string, on: boolean) =>
    setBusy((b) => {
      const next = new Set(b);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });

  const start = async (f: ForwardView) => {
    setBusyFor(f.id, true);
    try {
      markForwardRunning(sessionId, f.id, await api.forward_start(sessionId, f.id));
    } catch (err) {
      markForwardFailed(sessionId, f.id, errorMessage(t, err));
    } finally {
      setBusyFor(f.id, false);
    }
  };

  const stop = async (f: ForwardView) => {
    setBusyFor(f.id, true);
    try {
      await api.forward_stop(sessionId, f.id);
      markForwardStopped(sessionId, f.id);
    } catch (err) {
      toast(errorMessage(t, err), "error");
    } finally {
      setBusyFor(f.id, false);
    }
  };

  const copy = async (address: string) => {
    try {
      await writeClipboard(address);
      toast(t("terminal.fwd.copied", { addr: address }), "success");
    } catch {
      toast(t("terminal.clipboardDenied"), "error");
    }
  };

  if (locked) return null;
  return createPortal(
    <div ref={ref} role="dialog" aria-label={t("terminal.fwd")} className={s.popover} style={pos}>
      <div className={s.header}>{t("terminal.fwd")}</div>
      {!saved && !loadFailed && (
        <div className={s.note}>
          <Spinner />
        </div>
      )}
      {loadFailed && <div className={cx(s.note, s.noteError)}>{t("terminal.fwd.loadFailed")}</div>}
      {saved?.length === 0 && <div className={s.note}>{t("terminal.fwd.none")}</div>}
      {saved?.map((f) => (
        <ForwardRow
          key={f.id}
          forward={f}
          run={runs[f.id]}
          busy={busy.has(f.id)}
          onStart={() => void start(f)}
          onStop={() => void stop(f)}
          onCopy={(address) => void copy(address)}
        />
      ))}
      <div className={s.separator} />
      <button
        type="button"
        className={s.manage}
        onClick={() => {
          onClose(false);
          onManage();
        }}
      >
        <Icon name="pencil-simple" />
        {t("terminal.fwd.manage")}
      </button>
    </div>,
    document.body,
  );
}

function ForwardRow({
  forward: f,
  run,
  busy,
  onStart,
  onStop,
  onCopy,
}: {
  forward: ForwardView;
  run: ForwardRun | undefined;
  busy: boolean;
  onStart(): void;
  onStop(): void;
  onCopy(address: string): void;
}) {
  const t = useT();
  const state = run?.state ?? "stopped";
  const running = state === "running" && run?.localPort != null;
  const failed = state === "failed";
  const local = running ? `localhost:${run.localPort}` : hostPort(f.bind_address, f.bind_port || t("terminal.fwd.autoPort"));

  return (
    <div className={s.row}>
      <span className={cx(s.state, running && s.stateRunning, failed && s.stateFailed)}>
        <Icon name={running ? "check-circle" : failed ? "warning-circle" : "circle"} fill={running || failed} />
      </span>
      <div className={s.body}>
        {running ? (
          <button type="button" className={cx(s.address, s.addressCopy)} title={t("terminal.fwd.copy", { addr: local })} onClick={() => onCopy(local)}>
            {local}
          </button>
        ) : (
          <span className={s.address}>{local}</span>
        )}
        <span className={s.dest}>→ {hostPort(f.dest_host, f.dest_port)}</span>
        {failed && run?.error && <span className={s.error}>{run.error}</span>}
      </div>
      <button type="button" className={s.action} disabled={busy} onClick={running ? onStop : onStart}>
        {busy ? <Spinner size={12} /> : running ? t("terminal.fwd.stop") : failed ? t("btn.retry") : t("terminal.fwd.start")}
      </button>
    </div>
  );
}
