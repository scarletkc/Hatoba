import { useCallback, useEffect, useState } from "react";
import { Button, Icon, LinkButton, Switch, TextField } from "@/components/controls";
import { Group, Row, RowValue, Section, StatusTile } from "@/components/layout";
import { confirm, toast } from "@/components/overlay";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { formatRelative, useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { DeviceView, SyncStatus, WorkerUpdate } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { DEPLOY_GUIDE_URL, openExternal } from "./external";
import { ChangePasswordDialog, RecoveryCodeDialog } from "./SyncDialogs";
import { countsLine, deviceIcon, isStale, refreshSyncStatus, type T } from "./syncUtils";
import { UpgradeDialog } from "./UpgradeDialog";
import s from "./Sync.module.css";

/** Icon, tint and copy for each sync state; titles reuse the sidebar footer strings. */
function present(t: T, st: SyncStatus) {
  if (st.state === "syncing")
    return {
      icon: "arrows-clockwise",
      color: "var(--accent)",
      spin: true,
      title: t("sync.footer.syncing"),
      sub: st.pending > 0 ? t("sync.footer.syncing_sub", { n: st.pending }) : t("sync.footer.syncing_sub_idle"),
    };
  if (st.state === "offline")
    return {
      icon: "cloud-slash",
      color: "var(--fg2)",
      title: t("sync.footer.offline"),
      sub: [st.pending > 0 ? t("sync.footer.offline_sub", { n: st.pending }) : null, t("sync.footer.offline_sub_none")]
        .filter(Boolean)
        .join(" · "),
    };
  if (st.state === "auth_failed")
    return { icon: "lock-key", color: "var(--orange)", title: t("sync.footer.auth"), sub: t("sync.footer.auth_sub") };
  if (st.state === "error")
    return {
      icon: "cloud-warning",
      color: "var(--orange)",
      title: t("sync.footer.error"),
      sub: st.message ?? t("sync.footer.error_sub"),
    };
  if (st.state === "paused")
    return {
      icon: "cloud-warning",
      color: "var(--orange)",
      title: t(st.worker_update?.kind === "app_required" ? "sync.footer.appUpdate" : "sync.footer.workerUpdate"),
      sub: st.pending > 0 ? t("sync.footer.paused_sub", { n: st.pending }) : t("sync.footer.paused_sub_none"),
    };
  const last = st.last_synced_at ? t("sync.sub.last", { time: formatRelative(t.locale, st.last_synced_at) }) : t("sync.sub.never");
  const sub = st.counts ? `${last} · ${countsLine(t, st.counts)}` : last;
  if (st.conflicts > 0)
    return { icon: "warning-circle", color: "var(--orange)", title: t("sync.footer.conflict", { n: st.conflicts }), sub };
  return { icon: "cloud-check", color: "var(--green)", title: t("sync.footer.synced"), sub };
}

export function SyncStatusView({ status, onReview }: { status: SyncStatus; onReview: () => void }) {
  const t = useT();
  const view = present(t, status);
  const [busy, setBusy] = useState(false);
  const syncing = status.state === "syncing" || busy;
  // The Worker version "Update Worker" deploys; the dialog stays open after the notice goes.
  const [upgradeTo, setUpgradeTo] = useState<string | null>(null);

  // §6.7 Upgrades: the Worker's version is read again whenever this page opens.
  useEffect(() => {
    void api.sync_check_worker().catch(() => {});
  }, []);

  const syncNow = async () => {
    setBusy(true);
    try {
      await api.sync_now();
      await refreshSyncStatus();
    } catch (e) {
      toast(errorMessage(t, e), "error");
    } finally {
      setBusy(false);
    }
  };

  const setAuto = async (enabled: boolean) => {
    try {
      await api.sync_set_auto(enabled);
      await refreshSyncStatus();
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  return (
    <div className={s.page}>
      <div className={s.scroll}>
        <div className={s.column}>
          <div className={s.header}>
            <div className={view.spin ? s.spin : undefined}>
              <StatusTile icon={view.icon} color={view.color} />
            </div>
            <div className={s.headerText}>
              <div className={s.title}>{view.title}</div>
              <div className={s.sub}>{view.sub}</div>
            </div>
            <Button icon="arrows-clockwise" busy={syncing} disabled={status.state === "auth_failed"} onClick={() => void syncNow()}>
              {t("sync.syncNow")}
            </Button>
          </div>

          {status.state === "auth_failed" && <ReloginForm />}

          {status.worker_update && <WorkerUpdateNotice update={status.worker_update} onUpdate={setUpgradeTo} />}
          {upgradeTo && <UpgradeDialog bundled={upgradeTo} onClose={() => setUpgradeTo(null)} />}

          {status.conflicts > 0 && status.state !== "syncing" && (
            <div className={s.banner} role="status">
              <Icon name="warning-circle" className={s.bannerIcon} />
              <span className={s.bannerText}>{t("sync.conflictBanner", { n: status.conflicts })}</span>
              <LinkButton onClick={onReview}>{t("sync.conflictBanner.view")}</LinkButton>
            </div>
          )}

          <Section title={t("sync.sec.connection")}>
            <Group>
              <Row label={t("sync.row.method")}>
                <RowValue>{t(status.kind === "d1" ? "sync.method.d1" : "sync.method.worker")}</RowValue>
              </Row>
              {status.endpoint && (
                <Row label={t("sync.row.endpoint")}>
                  <RowValue mono>{status.endpoint}</RowValue>
                </Row>
              )}
              {status.database && (
                <Row label={t("sync.row.database")}>
                  <RowValue>{status.database}</RowValue>
                </Row>
              )}
              <Row label={t("sync.row.auto")}>
                <Switch checked={status.auto_sync} label={t("sync.row.auto")} onChange={(v) => void setAuto(v)} />
              </Row>
            </Group>
          </Section>

          <Devices status={status} />
          <SecurityNote />
        </div>
      </div>
    </div>
  );
}

/** §6.7 Upgrades: "Worker update available" (dismissible), "Worker update required", or a Worker that needs a newer Hatoba. */
function WorkerUpdateNotice({ update, onUpdate }: { update: WorkerUpdate; onUpdate: (bundled: string) => void }) {
  const t = useT();
  const openSettings = useApp((st) => st.openSettings);
  const { bundled } = update;
  const available = update.kind === "available";

  const dismiss = async () => {
    try {
      await api.sync_dismiss_worker_update();
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  const text =
    update.kind === "available"
      ? t("sync.wu.available", { version: update.version, bundled: bundled ?? "" })
      : update.kind === "required"
        ? t("sync.wu.required", { version: update.version })
        : t("sync.wu.app", { version: update.version });

  return (
    <div className={cx(s.banner, available && s.bannerInfo)} role="status">
      <Icon name={available ? "arrow-circle-up" : "warning-circle"} className={s.bannerIcon} />
      <span className={s.bannerText}>{text}</span>
      {update.kind === "app_required" ? (
        <LinkButton onClick={() => openSettings(true, "about")}>{t("sync.wu.checkApp")}</LinkButton>
      ) : bundled ? (
        <LinkButton onClick={() => onUpdate(bundled)}>{t("sync.wu.update")}</LinkButton>
      ) : (
        <LinkButton onClick={() => void openExternal(`${DEPLOY_GUIDE_URL}#upgrade`)}>{t("sync.up.guide")}</LinkButton>
      )}
      {available && (
        <LinkButton tone="muted" onClick={() => void dismiss()}>
          {t("sync.wu.dismiss")}
        </LinkButton>
      )}
    </div>
  );
}

/** auth_failed: the session expired or was revoked; the master password logs this device in again. */
function ReloginForm() {
  const t = useT();
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const login = async () => {
    if (!password || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.sync_login(password);
      await refreshSyncStatus();
    } catch (e) {
      setError(errorMessage(t, e));
      setBusy(false);
    }
  };

  return (
    <div className={s.relogin}>
      <div className={s.reloginRow}>
        <TextField
          large
          secret
          value={password}
          invalid={!!error}
          placeholder={t("sync.auth.password")}
          aria-label={t("sync.auth.password")}
          autoComplete="current-password"
          disabled={busy}
          onChange={(e) => {
            setPassword(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => e.key === "Enter" && void login()}
        />
        <Button variant="primary" busy={busy} disabled={!password} onClick={() => void login()}>
          {t("sync.auth.login")}
        </Button>
      </div>
      {error && (
        <div className={s.reloginError} role="alert">
          {error}
        </div>
      )}
    </div>
  );
}

function Devices({ status }: { status: SyncStatus }) {
  const t = useT();
  const [devices, setDevices] = useState<DeviceView[] | null>(null);

  const load = useCallback(async () => {
    try {
      setDevices(await api.sync_devices());
    } catch {
      setDevices((d) => d ?? []);
    }
  }, []);
  // Re-read after each sync round: `last_seen` and newly signed-in devices change.
  useEffect(() => {
    void load();
  }, [load, status.last_synced_at, status.state]);

  const revoke = async (d: DeviceView) => {
    const ok = await confirm({
      title: t("sync.device.remove.title", { name: d.name }),
      body: t("sync.device.remove.body"),
      confirmLabel: t("sync.device.remove.confirm"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.sync_revoke_device(d.device_id);
      await load();
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  if (!devices) return null;
  return (
    <Section title={`${t("sync.sec.devices")} · ${devices.length}`}>
      <Group>
        {devices.map((d) => {
          const stale = isStale(d);
          return (
            <div key={d.device_id} className={s.device}>
              <Icon name={deviceIcon(d)} className={s.deviceIcon} />
              <div className={s.deviceText}>
                <div className={s.deviceName}>
                  <span className={s.deviceNameText}>{d.name}</span>
                  {d.current && <span className={s.badge}>{t("sync.device.thisPc")}</span>}
                </div>
                <div className={s.deviceMeta}>
                  {d.platform} · {formatRelative(t.locale, d.last_seen)}
                </div>
              </div>
              {stale && <span className={s.deviceMeta}>{t("sync.device.stale")}</span>}
              {!d.current && (
                <LinkButton
                  tone={stale ? undefined : "muted"}
                  className={cx(s.removeLink, !stale && s.removeQuiet)}
                  aria-label={t("sync.device.removeAria", { name: d.name })}
                  onClick={() => void revoke(d)}
                >
                  {t("btn.remove")}
                </LinkButton>
              )}
            </div>
          );
        })}
      </Group>
    </Section>
  );
}

function SecurityNote() {
  const t = useT();
  const [dialog, setDialog] = useState<"recovery" | "password" | null>(null);

  const disconnect = async () => {
    const ok = await confirm({
      title: t("sync.disconnect.title"),
      body: t("sync.disconnect.body"),
      confirmLabel: t("sync.disconnect.confirm"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.sync_disconnect();
      await refreshSyncStatus();
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  return (
    <div className={s.security}>
      <Icon name="shield-check" className={s.securityIcon} />
      <div className={s.securityText}>
        <span className={s.securityTitle}>{t("sync.security.title")}</span> {t("sync.security.body")}
        <div className={s.securityLinks}>
          <LinkButton className={s.securityLink} onClick={() => setDialog("recovery")}>
            {t("sync.link.recovery")}
          </LinkButton>
          <LinkButton className={s.securityLink} onClick={() => setDialog("password")}>
            {t("sync.link.password")}
          </LinkButton>
          <LinkButton tone="muted" className={s.securityLink} onClick={() => void disconnect()}>
            {t("sync.link.disconnect")}
          </LinkButton>
        </div>
      </div>
      {dialog === "recovery" && <RecoveryCodeDialog onClose={() => setDialog(null)} />}
      {dialog === "password" && <ChangePasswordDialog onClose={() => setDialog(null)} />}
    </div>
  );
}
