import { useEffect, useState } from "react";
import { errorMessage } from "@/app/errors";
import { Icon, IconButton, LinkButton, Switch } from "@/components/controls";
import { Group, Section } from "@/components/layout";
import { confirm, toast } from "@/components/overlay";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { ForwardInput, ForwardView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { ForwardDialog } from "./ForwardDialog";
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

/**
 * "Port Forwarding" group of the host editor (FWD-01, FWD-02). Forwards are separate vault items:
 * every change here is saved immediately and has nothing to do with the host form's Save button.
 */
export function ForwardsSection({ hostId }: { hostId: string | null }) {
  const t = useT();
  const [items, setItems] = useState<ForwardView[] | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [dialog, setDialog] = useState<{ forward: ForwardView | null } | null>(null);
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());

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
          return (
            <div key={f.id} className={s.row}>
              <span className={s.rule} title={label}>
                {hostPort(f.bind_address, f.bind_port || t("hosts.fwd.autoPort"))}
                <span className={s.arrow}>→</span>
                {hostPort(f.dest_host, f.dest_port)}
              </span>
              <label className={s.auto}>
                {t("hosts.fwd.autoStart")}
                <Switch
                  checked={f.auto_start}
                  disabled={pending.has(f.id)}
                  label={t("hosts.fwd.autoStartFor", { rule: label })}
                  onChange={(on) => void setAutoStart(f, on)}
                />
              </label>
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
