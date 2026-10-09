import { useState } from "react";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { Badge, Button, Icon, IconButton } from "@/components/controls";
import { Group, layoutStyles } from "@/components/layout";
import { PopupSelect, confirm, toast, type SelectOption } from "@/components/overlay";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { ProxyView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { proxySummary } from "./proxyLogic";
import { ProxyDialog } from "./ProxyDialog";
import { EmptyBlock } from "./sectionParts";
import { Pane, SettingRow, usePrefs } from "./shared";
import s from "./ProxiesPane.module.css";

/**
 * Settings → Proxies (SSH-13): the saved SOCKS5 and HTTP proxies, which sync, and the default
 * proxy of this device, which does not.
 */
export function ProxiesPane() {
  const t = useT();
  const proxies = useVaultData((st) => st.proxies);
  const hosts = useVaultData((st) => st.hosts);
  const [prefs, setPrefs] = usePrefs();
  const [dialog, setDialog] = useState<{ proxy: ProxyView | null } | null>(null);
  const defaultId = prefs.default_proxy_id;

  const defaultOptions: SelectOption<string | null>[] = [
    { value: null, label: t("settings.proxy.none") },
    ...proxies.map((p) => ({ value: p.id, label: p.name, hint: proxySummary(p) })),
    // Deleted on another device: shown until another choice is made.
    ...(defaultId && !proxies.some((p) => p.id === defaultId) ? [{ value: defaultId, label: t("settings.proxy.missing") }] : []),
  ];

  const remove = async (p: ProxyView) => {
    const names = p.host_ids.map((id) => hosts.find((h) => h.id === id)?.name).filter(Boolean);
    const ok = await confirm({
      title: t("settings.proxy.deleteTitle", { name: p.name }),
      body: names.length
        ? t("settings.proxy.deleteBodyHosts", { n: names.length, names: names.join(t.locale === "en" ? ", " : "、") })
        : t("settings.proxy.deleteBody"),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.proxy_delete(p.id);
      // The backend also stopped using it as this device's default.
      if (prefs.default_proxy_id === p.id) setPrefs({ default_proxy_id: null });
      await useVaultData.getState().reload();
      toast(t("settings.proxy.deleted", { name: p.name }));
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  const saved = async () => {
    setDialog(null);
    try {
      await useVaultData.getState().reload();
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  return (
    <Pane>
      <Group>
        <SettingRow label={t("settings.proxy.default")} hint={t("settings.proxy.default.hint")}>
          <PopupSelect<string | null>
            ariaLabel={t("settings.proxy.default")}
            icon="globe"
            value={defaultId}
            options={defaultOptions}
            minWidth={260}
            onChange={(default_proxy_id) => setPrefs({ default_proxy_id })}
          />
        </SettingRow>
      </Group>

      <section className={layoutStyles.section}>
        <div className={s.sectionHead}>
          <span className={layoutStyles.sectionTitle}>{t("settings.proxy.saved")}</span>
          {proxies.length > 0 && (
            <Button size="sm" icon="plus" onClick={() => setDialog({ proxy: null })}>
              {t("settings.proxy.add")}
            </Button>
          )}
        </div>
        <div className={cx(layoutStyles.group, s.clip)}>
          {proxies.length === 0 && (
            <EmptyBlock icon="globe" title={t("settings.proxy.empty.title")} body={t("settings.proxy.empty.body")}>
              <Button variant="primary" icon="plus" onClick={() => setDialog({ proxy: null })}>
                {t("settings.proxy.add")}
              </Button>
            </EmptyBlock>
          )}
          {proxies.map((p) => (
            <div key={p.id} className={s.row}>
              <button type="button" className={s.main} title={t("settings.proxy.edit", { name: p.name })} onClick={() => setDialog({ proxy: p })}>
                <span className={s.icon} aria-hidden>
                  <Icon name="globe" />
                </span>
                <span className={s.text}>
                  <span className={s.nameLine}>
                    <span className={s.name}>{p.name}</span>
                    {p.id === defaultId && <Badge>{t("settings.proxy.isDefault")}</Badge>}
                  </span>
                  <span className={s.meta}>
                    <span className={s.where}>{proxySummary(p)}</span>
                    {p.host_ids.length > 0 && (
                      <>
                        <span className={s.dot}>·</span>
                        <span>{t("settings.proxy.hosts", { n: p.host_ids.length })}</span>
                      </>
                    )}
                  </span>
                </span>
                <Icon name="caret-right" className={s.caret} />
              </button>
              <IconButton icon="trash" label={t("settings.proxy.delete", { name: p.name })} className={s.delete} onClick={() => void remove(p)} />
            </div>
          ))}
        </div>
        <div className={s.sectionHint}>{t("settings.proxy.hint")}</div>
      </section>

      {dialog && <ProxyDialog proxy={dialog.proxy} onClose={() => setDialog(null)} onSaved={() => void saved()} />}
    </Pane>
  );
}
