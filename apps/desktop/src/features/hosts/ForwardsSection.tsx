import { useEffect, useState } from "react";
import { errorMessage } from "@/app/errors";
import { Button, Icon, IconButton, LinkButton, Switch } from "@/components/controls";
import { Group, Section } from "@/components/layout";
import { confirm, toast } from "@/components/overlay";
import { writeClipboard } from "@/features/terminal/clipboard";
import { markForwardStopped, useAllForwardRuns } from "@/features/terminal/forwards";
import { hostSessionIds } from "@/features/terminal/session";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { ForwardInput, ForwardView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { ForwardDialog } from "./ForwardDialog";
import { startTunnelForward, stopTunnelForward, tunnelPort, useHostTunnel } from "./tunnels";
import s from "./ForwardsSection.module.css";

/** `host:port`, with IPv6 literals bracketed. */
export function hostPort(host: string, port: number | string): string {
  return `${host.includes(":") ? `[${host}]` : host}:${port}`;
}

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

/**
 * "Port Forwarding" group of the host editor (FWD-01, FWD-02, FWD-05). Forwards are separate vault items:
 * every change here is saved immediately and has nothing to do with the host form's Save button. **Start**
 * runs a forward without a terminal, until it is stopped or Hatoba quits.
 */
export function ForwardsSection({ hostId }: { hostId: string | null }) {
  const t = useT();
  const [items, setItems] = useState<ForwardView[] | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [dialog, setDialog] = useState<{ forward: ForwardView | null } | null>(null);
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());
  const [starting, setStarting] = useState<ReadonlySet<string>>(new Set());
  const [failures, setFailures] = useState<Readonly<Record<string, string>>>({});
  const tunnel = useHostTunnel(hostId);
  const bySession = useAllForwardRuns();

  useEffect(() => {
    if (!hostId) return;
    let live = true;
    setItems(null);
    setLoadFailed(false);
    api
      .forwards_list(hostId)
      .then((list) => live && setItems(list))
      .catch(() => live && setLoadFailed(true));
    return () => {
      live = false;
    };
  }, [hostId]);

  const markPending = (id: string, on: boolean) =>
    setPending((p) => {
      const next = new Set(p);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });

  const markStarting = (id: string, on: boolean) =>
    setStarting((p) => {
      const next = new Set(p);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });

  const setFailure = (id: string, message: string | null) =>
    setFailures(({ [id]: _old, ...rest }) => (message === null ? rest : { ...rest, [id]: message }));

  const runOf = (f: ForwardView): Run | null => {
    const port = tunnelPort(tunnel, f.id);
    if (port !== null) return { localPort: port, sessionId: null };
    for (const sessionId of hostId ? hostSessionIds(hostId) : []) {
      const run = bySession[sessionId]?.[f.id];
      if (run?.state === "running" && run.localPort !== null) return { localPort: run.localPort, sessionId };
    }
    return null;
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

  const rule = (f: ForwardView) =>
    `${hostPort(f.bind_address, f.bind_port || t("hosts.fwd.autoPort"))} → ${hostPort(f.dest_host, f.dest_port)}`;

  const upsert = (saved: ForwardView) =>
    setItems((list) => {
      const rest = list ?? [];
      return rest.some((x) => x.id === saved.id) ? rest.map((x) => (x.id === saved.id ? saved : x)) : [...rest, saved];
    });

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

  const title = t("hosts.edit.sec.fwd");

  // A new host has no id to attach forwards to yet.
  if (!hostId) {
    return (
      <Section title={title}>
        <Group>
          <div className={s.footRow}>
            <span className={s.muted}>{t("hosts.fwd.newHint")}</span>
          </div>
        </Group>
      </Section>
    );
  }

  const addLink = (
    <LinkButton icon="plus" onClick={() => setDialog({ forward: null })} disabled={!items}>
      {t("hosts.fwd.add")}
    </LinkButton>
  );

  return (
    <Section title={title}>
      <Group>
        {items?.map((f) => {
          const label = rule(f);
          const run = runOf(f);
          const failure = run ? null : failures[f.id];
          const address = run && `localhost:${run.localPort}`;
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
        <div className={cx(s.footRow, items && items.length === 0 && s.footSplit)}>
          {loadFailed ? (
            <span className={s.error}>
              <Icon name="warning-circle" size={14} />
              {t("hosts.fwd.loadFailed")}
            </span>
          ) : (
            items?.length === 0 && <span className={s.muted}>{t("hosts.fwd.empty")}</span>
          )}
          {addLink}
        </div>
      </Group>
      {dialog && (
        <ForwardDialog
          hostId={hostId}
          forward={dialog.forward}
          onClose={() => setDialog(null)}
          onSaved={(saved) => {
            upsert(saved);
            setDialog(null);
          }}
        />
      )}
    </Section>
  );
}
