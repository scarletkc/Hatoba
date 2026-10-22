import { useRef, useState } from "react";
import { createPortal } from "react-dom";
import { errorMessage } from "@/app/errors";
import { Icon, LinkButton, Spinner } from "@/components/controls";
import { overlayStyles, toast, type MenuAnchor } from "@/components/overlay";
import { ListRow } from "@/components/patterns";
import { forwardDest, hostPort } from "@/features/hosts/address";
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
  // What each forward is busy with, so a stop that clears it from the connection first doesn't read as a start.
  const [busy, setBusy] = useState<Readonly<Record<string, "start" | "stop">>>({});
  const [errors, setErrors] = useState<Readonly<Record<string, string>>>({});
  // Re-measured as rows appear.
  const { pos, locked } = useStatusPopover(ref, anchor, onClose, "[data-forwards-trigger]", [saved, conn, errors]);

  const setBusyFor = (id: string, action: "start" | "stop" | null) =>
    setBusy(({ [id]: _old, ...rest }) => (action === null ? rest : { ...rest, [id]: action }));
  const setError = (id: string, message: string | null) =>
    setErrors(({ [id]: _old, ...rest }) => (message === null ? rest : { ...rest, [id]: message }));

  const start = async (f: ForwardView) => {
    setBusyFor(f.id, "start");
    setError(f.id, null);
    try {
      await startForward(hostId, f.id);
    } catch (err) {
      setError(f.id, errorMessage(t, err));
    } finally {
      setBusyFor(f.id, null);
    }
  };

  const stop = async (f: ForwardView) => {
    setBusyFor(f.id, "stop");
    try {
      await stopForward(hostId, f.id);
    } catch (err) {
      toast(errorMessage(t, err), "error");
    } finally {
      setBusyFor(f.id, null);
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
            busy={busy[f.id] ?? null}
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
  /** What the row is waiting on, if anything. */
  busy: "start" | "stop" | null;
  onStart(): void;
  onStop(): void;
  onCopy(address: string): void;
}) {
  const t = useT();
  const running = active !== null;
  const failed = error !== null;
  const starting = busy === "start" && !running;
  const kind = active?.kind ?? f.kind;
  // A remote forward listens on the server: nothing here connects to it, so it has nothing to copy.
  const remote = kind === "remote";
  const local = active?.address ?? hostPort(f.bind_address, f.bind_port || t("terminal.fwd.autoPort"));
  const dest = forwardDest(t, f, active);

  return (
    <ListRow
      size="sm"
      leading={
        // A starting forward turns its state icon into a spinner, so the row keeps its height.
        <span
          className={cx(s.state, running && s.stateRunning, failed && !starting && s.stateFailed)}
          role={starting ? "status" : undefined}
          aria-label={starting ? t("terminal.fwd.starting") : undefined}
          title={starting ? t("terminal.fwd.starting") : undefined}
        >
          {starting ? <Spinner size={14} /> : <Icon name={running ? "check-circle" : failed ? "warning-circle" : "circle"} fill={running || failed} />}
        </span>
      }
      status={failed && <span className={s.error}>{error}</span>}
      trailing={
        <LinkButton className={s.action} disabled={busy !== null} onClick={running ? onStop : onStart}>
          {running ? t("terminal.fwd.stop") : failed ? t("btn.retry") : t("terminal.fwd.start")}
        </LinkButton>
      }
    >
      {running && !remote ? (
        <button type="button" className={cx(s.address, s.addressCopy)} title={t("terminal.fwd.copy", { addr: local })} onClick={() => onCopy(local)}>
          {local}
        </button>
      ) : (
        <span className={s.address}>{remote ? t("terminal.fwd.onServer", { addr: local }) : local}</span>
      )}
      <span className={s.dest}>{kind === "dynamic" ? t("terminal.fwd.socks") : `→ ${dest}`}</span>
    </ListRow>
  );
}
