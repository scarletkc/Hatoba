import { useRef, useState } from "react";
import { createPortal } from "react-dom";
import { errorMessage } from "@/app/errors";
import { Icon, Spinner } from "@/components/controls";
import { overlayStyles, toast, type MenuAnchor } from "@/components/overlay";
import { hostPort } from "@/features/hosts/address";
import { forwardFailure, runningForward, useHostConnection } from "@/features/hosts/connections";
import { startForward, stopForward, useHostForwardRules } from "@/features/hosts/forwards";
import { useT } from "@/i18n";
import type { ForwardView, RunningForwardView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { writeClipboard } from "./clipboard";
import { useStatusPopover } from "./popover";
import s from "./ForwardsPopover.module.css";


/**
 * The status bar's port forwarding popover (FWD-01/02): this host's saved forwards, which run on the connection the
 * tab shares (SSH-16), with start / stop / retry for each.
 */
export function ForwardsPopover({
  anchor,
  hostId,
  onClose,
  onManage,
}: {
  anchor: MenuAnchor;
  hostId: string;
  /** `restoreFocus` is true when closed from the keyboard, so typing can continue in the terminal. */
  onClose: (restoreFocus: boolean) => void;
  onManage: () => void;
}) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  const saved = useHostForwardRules(hostId);
  const conn = useHostConnection(hostId);
  const [busy, setBusy] = useState<ReadonlySet<string>>(new Set());
  const [errors, setErrors] = useState<Readonly<Record<string, string>>>({});
  // Re-measured as rows appear.
  const { pos, locked } = useStatusPopover(ref, anchor, onClose, "[data-forwards-trigger]", [saved, conn, errors]);

  const setBusyFor = (id: string, on: boolean) =>
    setBusy((b) => {
      const next = new Set(b);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  const setError = (id: string, message: string | null) =>
    setErrors(({ [id]: _old, ...rest }) => (message === null ? rest : { ...rest, [id]: message }));

  const start = async (f: ForwardView) => {
    setBusyFor(f.id, true);
    setError(f.id, null);
    try {
      await startForward(hostId, f.id);
    } catch (err) {
      setError(f.id, errorMessage(t, err));
    } finally {
      setBusyFor(f.id, false);
    }
  };

  const stop = async (f: ForwardView) => {
    setBusyFor(f.id, true);
    try {
      await stopForward(hostId, f.id);
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
    <div ref={ref} role="dialog" aria-label={t("terminal.fwd")} className={cx(s.popover, overlayStyles.popIn)} style={pos}>
      <div className={s.header}>{t("terminal.fwd")}</div>
      {saved?.length === 0 && <div className={s.note}>{t("terminal.fwd.none")}</div>}
      {saved?.map((f) => {
        const running = runningForward(conn, f.id);
        const failure = forwardFailure(conn, f.id);
        const error = running === null ? (errors[f.id] ?? (failure && errorMessage(t, failure))) : null;
        return (
          <ForwardRow
            key={f.id}
            forward={f}
            running={running}
            error={error}
            busy={busy.has(f.id)}
            onStart={() => void start(f)}
            onStop={() => void stop(f)}
            onCopy={(address) => void copy(address)}
          />
        );
      })}
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
  running: active,
  error,
  busy,
  onStart,
  onStop,
  onCopy,
}: {
  forward: ForwardView;
  /** The forward while it runs, as it started: an edit to its rule applies at its next start. */
  running: RunningForwardView | null;
  /** Why it failed to start. */
  error: string | null;
  busy: boolean;
  onStart(): void;
  onStop(): void;
  onCopy(address: string): void;
}) {
  const t = useT();
  const running = active !== null;
  const failed = error !== null;
  const kind = active?.kind ?? f.kind;
  // A remote forward listens on the server: nothing here connects to it, so it has nothing to copy.
  const remote = kind === "remote";
  const local = active?.address ?? hostPort(f.bind_address, f.bind_port || t("terminal.fwd.autoPort"));
  const dest = active?.dest ?? hostPort(f.dest_host, f.dest_port);

  return (
    <div className={s.row}>
      <span className={cx(s.state, running && s.stateRunning, failed && s.stateFailed)}>
        <Icon name={running ? "check-circle" : failed ? "warning-circle" : "circle"} fill={running || failed} />
      </span>
      <div className={s.body}>
        {running && !remote ? (
          <button type="button" className={cx(s.address, s.addressCopy)} title={t("terminal.fwd.copy", { addr: local })} onClick={() => onCopy(local)}>
            {local}
          </button>
        ) : (
          <span className={s.address}>{remote ? t("terminal.fwd.onServer", { addr: local }) : local}</span>
        )}
        <span className={s.dest}>{kind === "dynamic" ? t("terminal.fwd.socks") : `→ ${dest}`}</span>
        {failed && <span className={s.error}>{error}</span>}
      </div>
      <button type="button" className={s.action} disabled={busy} onClick={running ? onStop : onStart}>
        {busy ? <Spinner size={12} /> : running ? t("terminal.fwd.stop") : failed ? t("btn.retry") : t("terminal.fwd.start")}
      </button>
    </div>
  );
}
