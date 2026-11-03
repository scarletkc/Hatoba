import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type SyntheticEvent,
} from "react";
import {
  Button,
  EmptyState,
  ErrorText,
  IconButton,
  Menu,
  PageHeader,
  Spinner,
  StatusDot,
  Switch,
  confirm,
  isImeEvent,
  layoutStyles,
  toast,
  useMenu,
  type MenuEntry,
} from "@hatoba/ui";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { ForwardInput, ForwardView, HostView, RunningForwardView } from "@/ipc/types";
import { forwardDest, forwardLabel, forwardRule, hostPort } from "@/lib/address";
import { writeClipboard } from "@/lib/clipboard";
import { cx } from "@/lib/cx";
import { OsIcon } from "@/lib/OsIcon";
import { isMac } from "@/lib/platform";
import { forwardFailure, runningForward, useConnections } from "@/session/connections";
import { dropForwardRule, putForwardRule, startForward, stopForward, useForwardRules } from "@/session/forwards";
import { ForwardDialog } from "./ForwardDialog";
import s from "./ForwardsPage.module.css";

const toInput = (f: ForwardView, patch: Partial<ForwardInput> = {}): ForwardInput => ({
  id: f.id,
  host_id: f.host_id,
  name: f.name,
  kind: f.kind,
  bind_address: f.bind_address,
  bind_port: f.bind_port,
  dest_host: f.dest_host,
  dest_port: f.dest_port,
  auto_start: f.auto_start,
  ...patch,
});

const toggled = (set: ReadonlySet<string>, id: string, on: boolean): ReadonlySet<string> => {
  const next = new Set(set);
  if (on) next.add(id);
  else next.delete(id);
  return next;
};

/** Keeps a double-click on a row's control from opening the rule. */
const keepClick = (e: SyntheticEvent) => e.stopPropagation();

/** A rule as a row shows it: whether it runs and where, or why it could not start. */
interface RowState {
  running: RunningForwardView | null;
  starting: boolean;
  failure: string | null;
  /** Saving, deleting, starting or stopping: its controls wait. */
  busy: boolean;
  /** Saving or deleting: its switch and edits wait too. */
  pending: boolean;
}

/**
 * The **Port Forwarding** page (FWD-01..05): every host's forwards in one list, the hosts list's columns and rows, with
 * a row per host above its forwards. Forwards are separate vault items, saved the moment they change. **Start** runs
 * one on the host's connection, which its terminal tabs share (SSH-16), until it is stopped, the connection ends or
 * Hatoba quits. `hostId` is the host whose rows come into view, and that new forwards start on.
 */
export function ForwardsPage({ hostId }: { hostId: string | null }) {
  const t = useT();
  const platform = useApp((st) => st.info.platform);
  const hosts = useVaultData((st) => st.hosts);
  const { rules: items, loadFailed } = useForwardRules();
  const [dialog, setDialog] = useState<{ forward: ForwardView | null } | null>(null);
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());
  const [starting, setStarting] = useState<ReadonlySet<string>>(new Set());
  const [stopping, setStopping] = useState<ReadonlySet<string>>(new Set());
  const [failures, setFailures] = useState<Readonly<Record<string, string>>>({});
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [ctxId, setCtxId] = useState<string | null>(null);
  const rowMenu = useMenu();
  const connections = useConnections();
  const focused = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const optionId = `${useId()}-forward-`;

  const sections = useMemo(() => {
    const byHost = new Map<string, ForwardView[]>();
    for (const f of items ?? []) byHost.set(f.host_id, [...(byHost.get(f.host_id) ?? []), f]);
    return hosts
      .filter((h) => byHost.has(h.id))
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((host) => ({ host, forwards: byHost.get(host.id)! }));
  }, [hosts, items]);
  const order = useMemo(() => sections.flatMap((x) => x.forwards), [sections]);
  const count = order.length;
  const selected = order.find((f) => f.id === selectedId) ?? null;

  const ready = items !== null;
  useEffect(() => {
    if (ready) focused.current?.scrollIntoView({ block: "start" });
  }, [ready, hostId]);

  // A rule that ran since, started from a tab or with the connection, drops the failure of an earlier start here.
  useEffect(() => {
    setFailures((all) => {
      const ran = Object.keys(all).filter((id) => connections.some((c) => runningForward(c, id)));
      if (ran.length === 0) return all;
      const rest = { ...all };
      for (const id of ran) delete rest[id];
      return rest;
    });
  }, [connections]);

  const markPending = (id: string, on: boolean) => setPending((p) => toggled(p, id, on));
  const markStarting = (id: string, on: boolean) => setStarting((p) => toggled(p, id, on));
  const markStopping = (id: string, on: boolean) => setStopping((p) => toggled(p, id, on));
  const setFailure = (id: string, message: string | null) =>
    setFailures(({ [id]: _old, ...rest }) => (message === null ? rest : { ...rest, [id]: message }));

  const label = (f: ForwardView) => forwardLabel(t, f);

  const stateOf = (f: ForwardView): RowState => {
    const conn = connections.find((x) => x.host_id === f.host_id);
    const running = runningForward(conn, f.id);
    const failed = forwardFailure(conn, f.id);
    return {
      running,
      starting: running === null && starting.has(f.id),
      failure: running === null ? (failures[f.id] ?? (failed ? errorMessage(t, failed) : null)) : null,
      busy: pending.has(f.id) || starting.has(f.id) || stopping.has(f.id),
      pending: pending.has(f.id),
    };
  };

  const setAutoStart = async (f: ForwardView, auto_start: boolean) => {
    markPending(f.id, true);
    putForwardRule({ ...f, auto_start }); // optimistic; rolled back below if the save fails
    try {
      putForwardRule(await api.forward_save(toInput(f, { auto_start })));
    } catch (err) {
      putForwardRule(f);
      toast(errorMessage(t, err), "error");
    } finally {
      markPending(f.id, false);
    }
  };

  const remove = async (f: ForwardView) => {
    const ok = await confirm({
      title: t("hosts.fwd.removeTitle", { rule: label(f) }),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    markPending(f.id, true);
    try {
      await api.forward_delete(f.id);
      dropForwardRule(f.id);
    } catch (err) {
      toast(errorMessage(t, err), "error");
    } finally {
      markPending(f.id, false);
    }
  };

  const start = async (f: ForwardView) => {
    markStarting(f.id, true);
    setFailure(f.id, null);
    try {
      await startForward(f.host_id, f.id);
    } catch (err) {
      setFailure(f.id, errorMessage(t, err));
    } finally {
      markStarting(f.id, false);
    }
  };

  const stop = async (f: ForwardView) => {
    markStopping(f.id, true);
    try {
      await stopForward(f.host_id, f.id);
    } catch (err) {
      toast(errorMessage(t, err), "error");
    } finally {
      markStopping(f.id, false);
    }
  };

  const toggle = (f: ForwardView, running: boolean) => void (running ? stop(f) : start(f));

  const copy = async (address: string) => {
    try {
      await writeClipboard(address);
      toast(t("terminal.fwd.copied", { addr: address }), "success");
    } catch {
      toast(t("terminal.clipboardDenied"), "error");
    }
  };

  const edit = (f: ForwardView) => setDialog({ forward: f });

  const move = (delta: number) => {
    if (order.length === 0) return;
    const index = order.findIndex((f) => f.id === selected?.id);
    // With none selected, ↓ starts at the first rule and ↑ at the last.
    const to = index < 0 ? (delta > 0 ? 0 : order.length - 1) : index + delta;
    const next = order[Math.max(0, Math.min(order.length - 1, to))];
    setSelectedId(next.id);
    requestAnimationFrame(() =>
      listRef.current?.querySelector(`[data-id="${CSS.escape(next.id)}"]`)?.scrollIntoView({ block: "nearest" }),
    );
  };

  // ↑/↓ move the selection, Enter edits the selected rule, and Delete deletes it, as on the keys list.
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.target !== listRef.current) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      move(e.key === "ArrowDown" ? 1 : -1);
    } else if (e.key === "Enter" && selected && !isImeEvent(e)) {
      e.preventDefault();
      edit(selected);
    } else if (selected && (e.key === "Delete" || (isMac(platform) && e.metaKey && e.key === "Backspace"))) {
      e.preventDefault();
      if (!pending.has(selected.id)) void remove(selected);
    } else if (e.key === "Escape" && selected) {
      e.preventDefault();
      setSelectedId(null);
    }
  };

  const onContext = (e: MouseEvent, f: ForwardView) => {
    e.preventDefault();
    setSelectedId(f.id);
    setCtxId(f.id);
    rowMenu.openAt({ x: e.clientX, y: e.clientY });
  };
  const ctxForward = order.find((f) => f.id === ctxId);

  const menuEntries = (f: ForwardView): MenuEntry[] => {
    const { running, failure, busy, pending: saving } = stateOf(f);
    const local = running && running.kind !== "remote" ? running.address : null;
    return [
      {
        label: running ? t("terminal.fwd.stop") : failure ? t("btn.retry") : t("terminal.fwd.start"),
        icon: running ? "stop" : failure ? "arrow-clockwise" : "play",
        disabled: busy,
        onSelect: () => toggle(f, running !== null),
      },
      ...(local
        ? [{ label: t("terminal.fwd.copy", { addr: local }), icon: "copy", onSelect: () => void copy(local) }]
        : []),
      { label: t("btn.edit"), icon: "pencil-simple", disabled: saving, onSelect: () => edit(f) },
      { kind: "separator" },
      { label: t("btn.delete"), icon: "trash", danger: true, disabled: saving, onSelect: () => void remove(f) },
    ];
  };

  const title = t("sidebar.forwards");
  const addButton = (
    <Button
      variant="primary"
      icon="plus"
      disabled={!ready || hosts.length === 0}
      onClick={() => setDialog({ forward: null })}
    >
      {t("hosts.fwd.add")}
    </Button>
  );
  const dialogs = dialog && (
    <ForwardDialog
      hostId={dialog.forward?.host_id ?? null}
      defaultHostId={hostId}
      forward={dialog.forward}
      onClose={() => setDialog(null)}
      onSaved={(saved) => {
        putForwardRule(saved);
        setDialog(null);
      }}
    />
  );

  if (ready && count === 0 && !loadFailed) {
    return (
      <div className={s.page}>
        <PageHeader title={title} count={t("hosts.fwd.count", { n: 0 })} />
        <div className={s.emptyWrap}>
          <EmptyState
            icon="arrows-left-right"
            title={t("hosts.fwd.empty")}
            body={t("hosts.fwd.emptyBody")}
            actions={addButton}
          />
        </div>
        {dialogs}
      </div>
    );
  }

  return (
    <div className={s.page} onKeyDown={onKeyDown}>
      <PageHeader title={title} count={ready ? t("hosts.fwd.count", { n: count }) : undefined}>
        {addButton}
      </PageHeader>
      <div className={s.body}>
        {ready && count > 0 && (
          <div className={cx(layoutStyles.tableHead, s.cols)}>
            <span className={s.headName}>{t("hosts.col.name")}</span>
            <span className={s.headStatus}>{t("hosts.fwd.col.status")}</span>
            <span>{t("hosts.fwd.autoStart")}</span>
            <span />
          </div>
        )}
        {/* The focus stays on the list; the active option tells a screen reader which rule is selected. */}
        <div
          ref={listRef}
          className={s.list}
          tabIndex={0}
          role="listbox"
          aria-label={title}
          aria-activedescendant={selected ? optionId + selected.id : undefined}
          onMouseDown={(e) => {
            // A press on a row selects it and leaves the keys to the list, as in the hosts list.
            if (e.button !== 0 || (e.target as Element).closest("button, a, input, textarea, select")) return;
            e.preventDefault();
            listRef.current?.focus({ preventScroll: true });
          }}
          onClick={(e) => {
            // A click beside the rows selects none.
            if (!(e.target as Element).closest('[role="option"]')) setSelectedId(null);
          }}
        >
          {loadFailed && (
            <ErrorText icon="warning-circle" className={s.loadFailed}>
              {t("hosts.fwd.loadFailed")}
            </ErrorText>
          )}
          {sections.map(({ host, forwards }) => (
            <div
              key={host.id}
              ref={host.id === hostId ? focused : undefined}
              role="group"
              aria-label={`${host.name}, ${hostTarget(host)}`}
              className={s.hostGroup}
            >
              <HostRow host={host} connected={connections.some((c) => c.host_id === host.id)} />
              {forwards.map((f) => {
                const state = stateOf(f);
                return (
                  <ForwardRow
                    key={f.id}
                    id={optionId + f.id}
                    forward={f}
                    state={state}
                    selected={f.id === selected?.id}
                    onSelect={() => setSelectedId(f.id)}
                    onEdit={() => edit(f)}
                    onRemove={() => void remove(f)}
                    onToggle={() => toggle(f, state.running !== null)}
                    onAutoStart={(on) => void setAutoStart(f, on)}
                    onCopy={(address) => void copy(address)}
                    onContext={(e) => onContext(e, f)}
                  />
                );
              })}
            </div>
          ))}
        </div>
      </div>
      {rowMenu.anchor && ctxForward && (
        <Menu anchor={rowMenu.anchor} onClose={rowMenu.close} entries={menuEntries(ctxForward)} />
      )}
      {dialogs}
    </div>
  );
}

const hostTarget = (host: HostView) => (host.username ? `${host.username}@` : "") + hostPort(host.address, host.port);

/**
 * The row above a host's forwards: its name and `user@address:port`, the badge green while it is connected. Hidden from
 * a screen reader, which hears the name and address as the group's label: a listbox's group holds only options.
 */
function HostRow({ host, connected }: { host: HostView; connected: boolean }) {
  return (
    <div className={s.hostRow} aria-hidden>
      <OsIcon os={host.os} badge={connected ? "var(--green)" : "var(--fg3)"} />
      <span className={s.hostName}>{host.name}</span>
      <span className={s.hostTarget}>{hostTarget(host)}</span>
    </div>
  );
}

/**
 * A forward's row: its name over its rule or the rule alone; its state with a dot, which is where it listens while it
 * runs or why it could not start; its **Start on connect** switch; and **Start**, **Stop** or **Retry**, with edit and
 * delete shown on the row under the pointer or selected.
 */
function ForwardRow({
  id,
  forward: f,
  state,
  selected,
  onSelect,
  onEdit,
  onRemove,
  onToggle,
  onAutoStart,
  onCopy,
  onContext,
}: {
  id: string;
  forward: ForwardView;
  state: RowState;
  selected: boolean;
  onSelect(): void;
  onEdit(): void;
  onRemove(): void;
  onToggle(): void;
  onAutoStart(on: boolean): void;
  onCopy(address: string): void;
  onContext(e: MouseEvent): void;
}) {
  const t = useT();
  const { running, starting, failure, busy, pending } = state;
  const rule = forwardRule(t, f);
  const bind = hostPort(f.bind_address, f.bind_port || t("hosts.fwd.autoPort"));
  // A narrow list keeps only the status dot, and this says what it means.
  const statusTitle = starting
    ? t("terminal.fwd.starting")
    : running
      ? running.kind === "remote"
        ? t("terminal.fwd.onServer", { addr: running.address })
        : running.address
      : (failure ?? t("hosts.fwd.stopped"));
  return (
    <div
      id={id}
      role="option"
      aria-selected={selected}
      data-id={f.id}
      className={cx(s.row, s.cols, selected && s.rowSel)}
      onClick={onSelect}
      onDoubleClick={onEdit}
      onContextMenu={onContext}
    >
      <span className={s.nameCell}>
        <span className={s.titles}>
          {f.name && (
            <span className={s.name} title={f.name}>
              {f.name}
            </span>
          )}
          <span className={cx(s.rule, f.name && s.ruleDetail)} title={rule}>
            {f.kind === "remote" ? t("terminal.fwd.onServer", { addr: bind }) : bind}
            {f.kind === "dynamic" ? (
              <span className={s.arrow}>{t("terminal.fwd.socks")}</span>
            ) : (
              <>
                <span className={s.arrow}>→</span>
                {forwardDest(t, f)}
              </>
            )}
          </span>
        </span>
      </span>
      <span className={s.status} title={statusTitle} onDoubleClick={keepClick}>
        {starting ? (
          <>
            <Spinner size={12} />
            <span className={s.statusText} role="status">
              {t("terminal.fwd.starting")}
            </span>
          </>
        ) : running ? (
          <>
            <StatusDot color="var(--green)" />
            <RunningAddress running={running} onCopy={onCopy} />
          </>
        ) : failure ? (
          <>
            <StatusDot color="var(--red)" />
            <span className={cx(s.statusText, s.error)} title={failure}>
              {failure}
            </span>
          </>
        ) : (
          <>
            <StatusDot color="var(--fg3)" />
            <span className={s.statusText}>{t("hosts.fwd.stopped")}</span>
          </>
        )}
      </span>
      <span className={s.auto} onDoubleClick={keepClick}>
        <Switch
          checked={f.auto_start}
          disabled={pending}
          label={t("hosts.fwd.autoStartFor", { rule: forwardLabel(t, f) })}
          onChange={onAutoStart}
        />
      </span>
      <span className={s.actions} onDoubleClick={keepClick}>
        <Button size="xs" className={s.toggle} disabled={busy} onClick={onToggle}>
          {running ? t("terminal.fwd.stop") : failure ? t("btn.retry") : t("terminal.fwd.start")}
        </Button>
        <IconButton
          icon="pencil-simple"
          label={t("btn.edit")}
          className={s.reveal}
          disabled={pending}
          onClick={onEdit}
        />
        <IconButton
          icon="trash"
          label={t("btn.delete")}
          danger
          className={s.reveal}
          disabled={pending}
          onClick={onRemove}
        />
      </span>
    </div>
  );
}

/**
 * Where a running forward listens, as it started: an edit to its rule applies at its next start. This computer's
 * address copies; a remote forward's is on the server, where nothing here connects to it.
 */
function RunningAddress({
  running: { kind, address },
  onCopy,
}: {
  running: RunningForwardView;
  onCopy(address: string): void;
}) {
  const t = useT();
  if (kind === "remote")
    return <span className={s.serverAddress}>{t("terminal.fwd.onServer", { addr: address })}</span>;
  return (
    <button
      type="button"
      className={s.address}
      title={t("terminal.fwd.copy", { addr: address })}
      onClick={() => onCopy(address)}
    >
      {address}
    </button>
  );
}
