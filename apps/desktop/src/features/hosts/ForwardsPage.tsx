import { useEffect, useMemo, useRef, useState } from "react";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { Button, Icon, IconButton, Switch } from "@/components/controls";
import { EmptyState, Group, PageHeader, Section } from "@/components/layout";
import { confirm, toast } from "@/components/overlay";
import { writeClipboard } from "@/features/terminal/clipboard";
import { markForwardStopped, useAllForwardRuns } from "@/features/terminal/forwards";
import { hostSessionIds } from "@/features/terminal/session";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { ForwardInput, ForwardView, HostView, TunnelView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { hostPort, reachableAddress } from "./address";
import { ForwardDialog } from "./ForwardDialog";
import { startTunnelForward, stopTunnelForward, tunnelPort, useTunnels } from "./tunnels";
import s from "./ForwardsPage.module.css";

const toInput = (f: ForwardView, patch: Partial<ForwardInput> = {}): ForwardInput => ({
  id: f.id,
  host_id: f.host_id,
  bind_address: f.bind_address,
  bind_port: f.bind_port,
  dest_host: f.dest_host,
  dest_port: f.dest_port,
  auto_start: f.auto_start,
  ...patch,
});

/** Where a forward runs on this device: without a terminal (FWD-05), or on one of the host's tabs. */
type Run = { localPort: number; sessionId: string | null };

const toggled = (set: ReadonlySet<string>, id: string, on: boolean): ReadonlySet<string> => {
  const next = new Set(set);
  if (on) next.add(id);
  else next.delete(id);
  return next;
};

/**
 * The **Port Forwarding** page (FWD-01, FWD-02, FWD-05): every host's forwards, a section per host. Forwards are
 * separate vault items, saved the moment they change. **Start** runs one without a terminal, until it is stopped
 * or Hatoba quits. `hostId` is the host whose section comes into view, and that new forwards start on.
 */
export function ForwardsPage({ hostId }: { hostId: string | null }) {
  const t = useT();
  const hosts = useVaultData((st) => st.hosts);
  const [items, setItems] = useState<ForwardView[] | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [dialog, setDialog] = useState<{ forward: ForwardView | null } | null>(null);
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());
  const [starting, setStarting] = useState<ReadonlySet<string>>(new Set());
  const [failures, setFailures] = useState<Readonly<Record<string, string>>>({});
  const tunnels = useTunnels();
  const bySession = useAllForwardRuns();
  const focused = useRef<HTMLDivElement>(null);

  // Read again when the hosts are: a sync may have brought forwards from another device.
  useEffect(() => {
    let live = true;
    setLoadFailed(false);
    api
      .forwards_list_all()
      .then((list) => live && setItems(list))
      .catch(() => live && setLoadFailed(true));
    return () => {
      live = false;
    };
  }, [hosts]);

  const sections = useMemo(() => {
    const byHost = new Map<string, ForwardView[]>();
    for (const f of items ?? []) byHost.set(f.host_id, [...(byHost.get(f.host_id) ?? []), f]);
    return hosts
      .filter((h) => byHost.has(h.id))
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((host) => ({ host, forwards: byHost.get(host.id)! }));
  }, [hosts, items]);
  const count = sections.reduce((n, x) => n + x.forwards.length, 0);

  const ready = items !== null;
  useEffect(() => {
    if (ready) focused.current?.scrollIntoView({ block: "start" });
  }, [ready, hostId]);

  const markPending = (id: string, on: boolean) => setPending((p) => toggled(p, id, on));
  const markStarting = (id: string, on: boolean) => setStarting((p) => toggled(p, id, on));
  const setFailure = (id: string, message: string | null) =>
    setFailures(({ [id]: _old, ...rest }) => (message === null ? rest : { ...rest, [id]: message }));

  const rule = (f: ForwardView) =>
    `${hostPort(f.bind_address, f.bind_port || t("hosts.fwd.autoPort"))} → ${hostPort(f.dest_host, f.dest_port)}`;

  const upsert = (saved: ForwardView) =>
    setItems((list) => {
      const rest = list ?? [];
      return rest.some((x) => x.id === saved.id) ? rest.map((x) => (x.id === saved.id ? saved : x)) : [...rest, saved];
    });

  const runOf = (f: ForwardView, tunnel: TunnelView | undefined): Run | null => {
    const port = tunnelPort(tunnel, f.id);
    if (port !== null) return { localPort: port, sessionId: null };
    for (const sessionId of hostSessionIds(f.host_id)) {
      const run = bySession[sessionId]?.[f.id];
      if (run?.state === "running" && run.localPort !== null) return { localPort: run.localPort, sessionId };
    }
    return null;
  };

  const setAutoStart = async (f: ForwardView, auto_start: boolean) => {
    markPending(f.id, true);
    upsert({ ...f, auto_start }); // optimistic; rolled back below if the save fails
    try {
      upsert(await api.forward_save(toInput(f, { auto_start })));
    } catch (err) {
      upsert(f);
      toast(errorMessage(t, err), "error");
    } finally {
      markPending(f.id, false);
    }
  };

  const remove = async (f: ForwardView) => {
    const ok = await confirm({
      title: t("hosts.fwd.removeTitle", { rule: rule(f) }),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    markPending(f.id, true);
    try {
      await api.forward_delete(f.id);
      setItems((list) => (list ?? []).filter((x) => x.id !== f.id));
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
      await startTunnelForward(f.host_id, f.id);
    } catch (err) {
      setFailure(f.id, errorMessage(t, err));
    } finally {
      markStarting(f.id, false);
    }
  };

  const stop = async (f: ForwardView, run: Run) => {
    markStarting(f.id, true);
    try {
      if (run.sessionId === null) await stopTunnelForward(f.host_id, f.id);
      else {
        await api.forward_stop(run.sessionId, f.id);
        markForwardStopped(run.sessionId, f.id);
      }
    } catch (err) {
      toast(errorMessage(t, err), "error");
    } finally {
      markStarting(f.id, false);
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

  const title = t("sidebar.forwards");
  const addButton = (
    <Button variant="primary" icon="plus" disabled={!ready || hosts.length === 0} onClick={() => setDialog({ forward: null })}>
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
        upsert(saved);
        setDialog(null);
      }}
    />
  );

  if (ready && count === 0 && !loadFailed) {
    return (
      <div className={s.page}>
        <PageHeader title={title} count={t("hosts.fwd.count", { n: 0 })} />
        <div className={s.emptyWrap}>
          <EmptyState icon="arrows-left-right" title={t("hosts.fwd.empty")} body={t("hosts.fwd.emptyBody")} actions={addButton} />
        </div>
        {dialogs}
      </div>
    );
  }

  return (
    <div className={s.page}>
      <PageHeader title={title} count={ready ? t("hosts.fwd.count", { n: count }) : undefined}>
        {addButton}
      </PageHeader>
      <div className={s.scroll}>
        <div className={s.column}>
          {loadFailed && (
            <span className={s.error}>
              <Icon name="warning-circle" size={14} />
              {t("hosts.fwd.loadFailed")}
            </span>
          )}
          {sections.map(({ host, forwards }) => {
            const tunnel = tunnels.find((x) => x.host_id === host.id);
            return (
              <div key={host.id} ref={host.id === hostId ? focused : undefined} className={s.hostSection}>
                <Section title={<HostTitle host={host} />}>
                  <Group>
                    {forwards.map((f) => {
                      const label = rule(f);
                      const run = runOf(f, tunnel);
                      const failure = run ? null : failures[f.id];
                      const address = run && reachableAddress(f.bind_address, run.localPort);
                      return (
                        <div key={f.id} className={s.row}>
                          <div className={s.body}>
                            <span className={s.rule} title={label}>
                              {hostPort(f.bind_address, f.bind_port || t("hosts.fwd.autoPort"))}
                              <span className={s.arrow}>→</span>
                              {hostPort(f.dest_host, f.dest_port)}
                            </span>
                            {address && (
                              <span className={s.status}>
                                <Icon name="check-circle" fill size={13} className={s.running} />
                                <button
                                  type="button"
                                  className={s.address}
                                  title={t("terminal.fwd.copy", { addr: address })}
                                  onClick={() => void copy(address)}
                                >
                                  {address}
                                </button>
                                {run.sessionId !== null && <span className={s.where}>{t("hosts.fwd.inTab")}</span>}
                              </span>
                            )}
                            {failure && (
                              <span className={cx(s.status, s.failed)}>
                                <Icon name="warning-circle" fill size={13} />
                                <span className={s.failure}>{failure}</span>
                              </span>
                            )}
                          </div>
                          <label className={s.auto}>
                            {t("hosts.fwd.autoStart")}
                            <Switch
                              checked={f.auto_start}
                              disabled={pending.has(f.id)}
                              label={t("hosts.fwd.autoStartFor", { rule: label })}
                              onChange={(on) => void setAutoStart(f, on)}
                            />
                          </label>
                          <Button
                            size="xs"
                            className={s.toggle}
                            busy={starting.has(f.id)}
                            disabled={pending.has(f.id)}
                            onClick={() => void (run ? stop(f, run) : start(f))}
                          >
                            {run ? t("terminal.fwd.stop") : failure ? t("btn.retry") : t("terminal.fwd.start")}
                          </Button>
                          <span className={s.actions}>
                            <IconButton
                              icon="pencil-simple"
                              label={t("btn.edit")}
                              disabled={pending.has(f.id)}
                              onClick={() => setDialog({ forward: f })}
                            />
                            <IconButton
                              icon="trash"
                              label={t("btn.delete")}
                              className={s.remove}
                              disabled={pending.has(f.id)}
                              onClick={() => void remove(f)}
                            />
                          </span>
                        </div>
                      );
                    })}
                  </Group>
                </Section>
              </div>
            );
          })}
        </div>
      </div>
      {dialogs}
    </div>
  );
}

/** A section's title: the host's name and `user@address:port`. */
function HostTitle({ host }: { host: HostView }) {
  return (
    <span className={s.hostTitle}>
      <span className={s.hostName}>{host.name}</span>
      <span className={s.hostTarget}>
        {host.username ? `${host.username}@` : ""}
        {hostPort(host.address, host.port)}
      </span>
    </span>
  );
}
