import { Icon } from "@hatoba/ui";
import { formatRelative, useT } from "@/i18n";
import type { SyncStatus } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { useApp, type Page } from "./store";
import st from "./Sidebar.module.css";

type Key = "hosts" | "forwards" | "keys" | "sync";

function pageKey(page: Page): Key {
  switch (page.kind) {
    case "hosts":
    case "host-edit":
    case "telnet-edit":
      return "hosts";
    case "keys":
      return "keys";
    case "forwards":
      return "forwards";
    case "sync":
      return "sync";
  }
}

/**
 * The home tab's navigation rail, as in Microsoft Store and Teams: the places that hold data, each an icon over its
 * name, and Cloud Sync at the bottom, whose icon is the sync status. Groups, tags and favorites narrow the hosts page
 * itself; Lock and Settings are in the title bar.
 */
export function Sidebar() {
  const t = useT();
  const { page, navigate, sync } = useApp();
  const active = pageKey(page);
  const status = syncFooterView(t, sync);

  const item = (
    key: Key,
    icon: string,
    label: string,
    target: Page,
    extra?: { color?: string; spin?: boolean; title?: string },
  ) => (
    <button
      key={key}
      type="button"
      className={cx(st.item, active === key && st.itemActive)}
      aria-current={active === key ? "page" : undefined}
      title={extra?.title ?? label}
      onClick={() => navigate(target)}
    >
      <Icon name={icon} className={cx(st.itemIcon, extra?.spin && st.spin)} color={extra?.color} />
      <span className={st.itemLabel}>{label}</span>
    </button>
  );

  return (
    <nav className={st.rail} aria-label="Hatoba">
      <div className={st.list}>
        {item("hosts", "hard-drives", t("hosts.title"), { kind: "hosts", group: null })}
        {item("forwards", "arrows-left-right", t("rail.forwards"), { kind: "forwards", hostId: null })}
        {item("keys", "key", t("sidebar.keys"), { kind: "keys" })}
      </div>
      <div className={st.foot}>
        {item(
          "sync",
          status.icon,
          t("rail.sync"),
          { kind: "sync" },
          {
            color: status.color,
            spin: status.spin,
            title: `${status.title}
${status.sub}`,
          },
        )}
      </div>
    </nav>
  );
}

export function syncFooterView(t: ReturnType<typeof useT>, s: SyncStatus | null) {
  if (!s || s.kind === "none")
    return {
      icon: "cloud",
      color: "var(--fg2)",
      title: t("sync.footer.none"),
      sub: t("sync.footer.none_sub"),
      spin: false,
    };
  switch (s.state) {
    case "syncing":
      return {
        icon: "arrows-clockwise",
        color: "var(--accent)",
        title: t("sync.footer.syncing"),
        sub: s.pending > 0 ? t("sync.footer.syncing_sub", { n: s.pending }) : t("sync.footer.syncing_sub_idle"),
        spin: true,
      };
    case "offline":
      return {
        icon: "cloud-slash",
        color: "var(--fg2)",
        title: t("sync.footer.offline"),
        sub: s.pending > 0 ? t("sync.footer.offline_sub", { n: s.pending }) : t("sync.footer.offline_sub_none"),
        spin: false,
      };
    case "auth_failed":
      return {
        icon: "cloud-warning",
        color: "var(--orange)",
        title: t("sync.footer.auth"),
        sub: t("sync.footer.auth_sub"),
        spin: false,
      };
    case "error":
      return {
        icon: "cloud-warning",
        color: "var(--red)",
        title: t("sync.footer.error"),
        sub: t("sync.footer.error_sub"),
        spin: false,
      };
    case "paused":
      return {
        icon: "cloud-warning",
        color: "var(--orange)",
        title: t(s.worker_update?.kind === "app_required" ? "sync.footer.appUpdate" : "sync.footer.workerUpdate"),
        sub: s.pending > 0 ? t("sync.footer.paused_sub", { n: s.pending }) : t("sync.footer.paused_sub_none"),
        spin: false,
      };
    case "off":
      return {
        icon: "cloud",
        color: "var(--fg2)",
        title: t("sync.footer.none"),
        sub: t("sync.footer.none_sub"),
        spin: false,
      };
    default:
      if (s.conflicts > 0)
        return {
          icon: "warning-circle",
          color: "var(--orange)",
          title: t("sync.footer.conflict", { n: s.conflicts }),
          sub: t("sync.footer.conflict_sub"),
          spin: false,
        };
      return {
        icon: "cloud-check",
        color: "var(--green)",
        title: t("sync.footer.synced"),
        sub: t("sync.footer.synced_sub", {
          time: s.last_synced_at ? formatRelative(t.locale, s.last_synced_at) : t("time.never"),
        }),
        spin: false,
      };
  }
}
