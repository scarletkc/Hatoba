import { useEffect, useMemo, useRef, useState } from "react";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { Button, IconButton, Switch } from "@/components/controls";
import { EmptyState, Group, PageHeader, Section } from "@/components/layout";
import { confirm, toast } from "@/components/overlay";
import { ErrorText, ListRow, StatusLine } from "@/components/patterns";
import { writeClipboard } from "@/features/terminal/clipboard";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { ForwardInput, ForwardView, HostView, RunningForwardView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { forwardDest, forwardLabel, forwardRule, hostPort } from "./address";
import { ForwardDialog } from "./ForwardDialog";
import { forwardFailure, runningForward, useConnections } from "./connections";
import { dropForwardRule, putForwardRule, startForward, stopForward, useForwardRules } from "./forwards";
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

/**
 * The **Port Forwarding** page (FWD-01..05): every host's forwards, a section per host. Forwards are
 * separate vault items, saved the moment they change. **Start** runs one on the host's connection, which its terminal
 * tabs share (SSH-16), until it is stopped, the connection ends or Hatoba quits. `hostId` is the host whose section
 * comes into view, and that new forwards start on.
 */
export function ForwardsPage({ hostId }: { hostId: string | null }) {
  const t = useT();
  const hosts = useVaultData((st) => st.hosts);
  const { rules: items, loadFailed } = useForwardRules();
  const [dialog, setDialog] = useState<{ forward: ForwardView | null } | null>(null);
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());
  const [starting, setStarting] = useState<ReadonlySet<string>>(new Set());
  const [stopping, setStopping] = useState<ReadonlySet<string>>(new Set());
  const [failures, setFailures] = useState<Readonly<Record<string, string>>>({});
  const connections = useConnections();
  const focused = useRef<HTMLDivElement>(null);

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
            <ErrorText icon="warning-circle">{t("hosts.fwd.loadFailed")}</ErrorText>
          )}
          {sections.map(({ host, forwards }) => {
            const conn = connections.find((x) => x.host_id === host.id);
            return (
              <div key={host.id} ref={host.id === hostId ? focused : undefined} className={s.hostSection}>
                <Section title={<HostTitle host={host} />}>
                  <Group>
                    {forwards.map((f) => {
                      const rule = forwardRule(t, f);
                      const running = runningForward(conn, f.id);
                      const address = running?.address ?? null;
                      const failed = forwardFailure(conn, f.id);
                      const failure = address === null ? (failures[f.id] ?? (failed && errorMessage(t, failed))) : null;
                      const bind = hostPort(f.bind_address, f.bind_port || t("hosts.fwd.autoPort"));
                      const status = running ? (
                        <StatusLine tone="success">
                          <RunningAddress running={running} onCopy={(a) => void copy(a)} />
                        </StatusLine>
                      ) : starting.has(f.id) ? (
                        <StatusLine tone="progress">{t("terminal.fwd.starting")}</StatusLine>
                      ) : (
                        failure && <StatusLine tone="error">{failure}</StatusLine>
                      );
                      return (
                        <ListRow
                          key={f.id}
                          status={status}
                          trailing={
                            <>
                              <label className={s.auto}>
                                {t("hosts.fwd.autoStart")}
                                <Switch
                                  checked={f.auto_start}
                                  disabled={pending.has(f.id)}
                                  label={t("hosts.fwd.autoStartFor", { rule: label(f) })}
                                  onChange={(on) => void setAutoStart(f, on)}
                                />
                              </label>
                              <Button
                                size="xs"
                                className={s.toggle}
                                disabled={pending.has(f.id) || starting.has(f.id) || stopping.has(f.id)}
                                onClick={() => void (address !== null ? stop(f) : start(f))}
                              >
                                {address !== null ? t("terminal.fwd.stop") : failure ? t("btn.retry") : t("terminal.fwd.start")}
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
                                  danger
                                  disabled={pending.has(f.id)}
                                  onClick={() => void remove(f)}
                                />
                              </span>
                            </>
                          }
                        >
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
                        </ListRow>
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

/**
 * Where a running forward listens, as it started: an edit to its rule applies at its next start. This computer's
 * address copies; a remote forward's is on the server, where nothing here connects to it.
 */
function RunningAddress({ running: { kind, address }, onCopy }: { running: RunningForwardView; onCopy(address: string): void }) {
  const t = useT();
  if (kind === "remote") return <span className={s.serverAddress}>{t("terminal.fwd.onServer", { addr: address })}</span>;
  return (
    <button type="button" className={s.address} title={t("terminal.fwd.copy", { addr: address })} onClick={() => onCopy(address)}>
      {address}
    </button>
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
