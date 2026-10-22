import { useEffect, useState } from "react";
import { Button, Icon, LinkButton, StatusDot } from "@/components/controls";
import { toast } from "@/components/overlay";
import { HintText } from "@/components/patterns";
import { useVaultData } from "@/app/data";
import { useApp } from "@/app/store";
import { formatDate, formatRelative, useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { HostView, KeyView } from "@/ipc/types";
import { copyText } from "./clipboard";
import { keyAlgoName } from "./keyUtils";
import s from "./KeysPage.module.css";

/** Online dots for the hosts using a key (HOST-10 probe, only when the user left probing on). */
function useProbe(hosts: HostView[]): Record<string, boolean> {
  const enabled = useApp((st) => st.prefs.host_probe);
  const [state, setState] = useState<Record<string, boolean>>({});
  const ids = hosts.map((h) => h.id).join(",");
  useEffect(() => {
    setState({});
    if (!enabled || !ids) return;
    let live = true;
    api
      .hosts_probe(ids.split(","))
      .then((results) => live && setState(Object.fromEntries(results.map((r) => [r.id, r.online]))))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [enabled, ids]);
  return state;
}

export function KeyDetail({
  keyView,
  onRename,
  onDeploy,
  onDelete,
}: {
  keyView: KeyView;
  onRename: () => void;
  onDeploy: () => void;
  onDelete: () => void;
}) {
  const t = useT();
  const allHosts = useVaultData((st) => st.hosts);
  const hosts = keyView.used_by.map((id) => allHosts.find((h) => h.id === id)).filter((h): h is HostView => !!h);
  const probe = useProbe(hosts);
  const lastUsedHost = hosts
    .filter((h) => h.last_connected_at !== null)
    .sort((a, b) => b.last_connected_at! - a.last_connected_at!)[0];

  const copyPublic = async () => {
    try {
      await copyText(keyView.public_key);
      toast(t("btn.copied"), "success");
    } catch {
      toast(t("err.internal"), "error");
    }
  };

  const editHost = (hostId: string) => useApp.getState().navigate({ kind: "host-edit", hostId, groupId: null, back: { kind: "all" } });

  return (
    <aside className={s.panel} aria-label={keyView.name}>
      <div className={s.panelScroll}>
        <div className={s.panelHead}>
          <div className={s.panelTile}>
            <Icon name="key" />
          </div>
          <div className={s.panelHeadText}>
            <div className={s.panelName} title={keyView.name}>
              {keyView.name}
            </div>
            <div className={s.panelMeta}>
              {t(keyView.has_passphrase ? "keys.meta.protected" : "keys.meta.plain", {
                algo: keyAlgoName(keyView.algorithm),
                bits: keyView.bits,
              })}
            </div>
          </div>
        </div>

        <div className={s.facts}>
          <span className={s.factLabel}>{t("keys.detail.fingerprint")}</span>
          <span className={`${s.fingerprint} selectable`}>{keyView.fingerprint}</span>
          <span className={s.factLabel}>{t("keys.detail.created")}</span>
          <span>{formatDate(keyView.created_at)}</span>
          {lastUsedHost && (
            <>
              <span className={s.factLabel}>{t("keys.detail.lastUsed")}</span>
              <span>
                {formatRelative(t.locale, lastUsedHost.last_connected_at!)} · {lastUsedHost.name}
              </span>
            </>
          )}
        </div>

        <div className={s.block}>
          <div className={s.blockHead}>
            <span className={s.blockTitle}>{t("keys.detail.publicKey")}</span>
            <LinkButton icon="copy" aria-label={t("keys.detail.copyPublic")} onClick={() => void copyPublic()}>
              {t("btn.copy")}
            </LinkButton>
          </div>
          <div className={`${s.publicKey} selectable`}>{keyView.public_key}</div>
        </div>

        <div className={s.block}>
          <div className={s.blockTitle}>{t("keys.detail.privateKey")}</div>
          <div className={s.privateBox} aria-hidden>
            {"•".repeat(26)}
            <br />
            {"•".repeat(20)}
          </div>
        </div>

        <div className={s.usedHosts}>
          <div className={s.blockTitle}>
            {t("keys.detail.usedHosts")} <span className={s.blockCount}>{hosts.length}</span>
          </div>
          {hosts.length === 0 ? (
            <HintText>{t("keys.detail.noHosts")}</HintText>
          ) : (
            hosts.map((h) => (
              <button
                key={h.id}
                type="button"
                className={s.hostRow}
                aria-label={t("keys.detail.editHost", { name: h.name })}
                onClick={() => editHost(h.id)}
              >
                <StatusDot size={6} color={h.id in probe ? (probe[h.id] ? "var(--green)" : "var(--red)") : "var(--fg3)"} />
                <span className={s.hostName}>{h.name}</span>
              </button>
            ))
          )}
        </div>
      </div>
      <div className={s.panelFoot}>
        <Button size="sm" onClick={onRename}>
          {t("btn.rename")}
        </Button>
        <Button size="sm" onClick={onDeploy}>
          {t("keys.action.deploy")}
        </Button>
        <span className={s.footSpacer} />
        <LinkButton tone="danger" className={s.deleteLink} onClick={onDelete}>
          {t("keys.action.delete")}
        </LinkButton>
      </div>
    </aside>
  );
}
