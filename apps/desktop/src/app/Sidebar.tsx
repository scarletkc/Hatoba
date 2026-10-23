import { useEffect } from "react";
import { Button, Icon } from "@/components/controls";
import { openExternal } from "@/features/sync/external";
import { formatRelative, useT } from "@/i18n";
import type { SyncStatus } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { REPO_URL, bugReportUrl } from "@/lib/github";
import { useVaultData } from "./data";
import { useStarPrompt, useStarPromptDue } from "./star";
import { useApp, type Page } from "./store";
import { useTabs } from "./tabs";
import { useOfferedUpdate } from "./update";
import st from "./Sidebar.module.css";

type Key = "hosts" | "forwards" | "keys";

function pageKey(page: Page): Key | null {
  switch (page.kind) {
    case "hosts":
    case "host-edit":
      return "hosts";
    case "keys":
      return "keys";
    case "forwards":
      return "forwards";
    case "sync":
      return null;
  }
}

/**
 * The home tab's navigation: the places that hold data, and the sync status, which opens Cloud Sync. Groups, tags
 * and favorites narrow the hosts page itself; Lock and Settings are in the title bar.
 */
export function Sidebar() {
  const t = useT();
  const { page, navigate, sync } = useApp();
  const hosts = useVaultData((s) => s.hosts);
  const keys = useVaultData((s) => s.keys);
  const active = pageKey(page);

  const item = (key: Key, icon: string, label: string, count: number | null, target: Page) => (
    <button
      key={key}
      type="button"
      className={cx(st.item, active === key && st.itemActive)}
      aria-current={active === key ? "page" : undefined}
      onClick={() => navigate(target)}
    >
      <Icon name={icon} className={st.itemIcon} />
      <span className={st.itemLabel}>{label}</span>
      {count !== null && <span className={st.itemCount}>{count}</span>}
    </button>
  );

  return (
    <nav className={st.sidebar} aria-label="Hatoba">
      <div className={st.list}>
        {item("hosts", "hard-drives", t("hosts.title"), hosts.length, { kind: "hosts", group: null })}
        {item("forwards", "arrows-left-right", t("sidebar.forwards"), null, { kind: "forwards", hostId: null })}
        {item("keys", "key", t("sidebar.keys"), keys.length, { kind: "keys" })}
      </div>

      <StarPrompt />
      <div className={st.footer}>
        <SyncFooter status={sync} active={page.kind === "sync"} onClick={() => navigate({ kind: "sync" })} />
      </div>
    </nav>
  );
}

/** Asks for a GitHub star once it is due, on the home tab, and not while the update dot shows (spec §9). */
function StarPrompt() {
  const t = useT();
  const info = useApp((st) => st.info);
  const due = useStarPromptDue(useTabs((s) => s.connectedOnce));
  const home = useTabs((s) => s.active === "home");
  const updateAvailable = useOfferedUpdate() !== null;
  useEffect(() => {
    void useStarPrompt.getState().load();
  }, []);

  if (!home || updateAvailable || !due) return null;
  const { finish } = useStarPrompt.getState();
  const open = (url: string) => {
    finish();
    void openExternal(url);
  };
  return (
    <section className={st.star} aria-label={t("sidebar.star.title")}>
      <div className={st.starHead}>
        <Icon name="star" fill className={st.starIcon} />
        <span className={st.starTitle}>{t("sidebar.star.title")}</span>
        <button type="button" className={st.starClose} title={t("btn.close")} aria-label={t("btn.close")} onClick={finish}>
          <Icon name="x" />
        </button>
      </div>
      <p className={st.starBody}>{t("sidebar.star.body")}</p>
      <div className={st.starActions}>
        <Button size="sm" variant="primary" onClick={() => open(REPO_URL)}>
          {t("sidebar.star.star")}
        </Button>
        <Button size="sm" onClick={() => open(bugReportUrl(info))}>
          {t("sidebar.star.report")}
        </Button>
      </div>
    </section>
  );
}

function SyncFooter({ status, active, onClick }: { status: SyncStatus | null; active: boolean; onClick: () => void }) {
  const t = useT();
  const view = syncFooterView(t, status);
  return (
    <button type="button" className={cx(st.syncButton, active && st.syncButtonActive)} aria-current={active ? "page" : undefined} onClick={onClick}>
      <Icon name={view.icon} className={cx(st.syncIcon, view.spin && st.spin)} color={view.color} />
      <span className={st.syncText}>
        <div className={st.syncTitle}>{view.title}</div>
        <div className={st.syncSub}>{view.sub}</div>
      </span>
    </button>
  );
}

export function syncFooterView(t: ReturnType<typeof useT>, s: SyncStatus | null) {
  if (!s || s.kind === "none")
    return { icon: "cloud", color: "var(--fg2)", title: t("sync.footer.none"), sub: t("sync.footer.none_sub"), spin: false };
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
      return { icon: "cloud-warning", color: "var(--orange)", title: t("sync.footer.auth"), sub: t("sync.footer.auth_sub"), spin: false };
    case "error":
      return { icon: "cloud-warning", color: "var(--red)", title: t("sync.footer.error"), sub: t("sync.footer.error_sub"), spin: false };
    case "paused":
      return {
        icon: "cloud-warning",
        color: "var(--orange)",
        title: t(s.worker_update?.kind === "app_required" ? "sync.footer.appUpdate" : "sync.footer.workerUpdate"),
        sub: s.pending > 0 ? t("sync.footer.paused_sub", { n: s.pending }) : t("sync.footer.paused_sub_none"),
        spin: false,
      };
    case "off":
      return { icon: "cloud", color: "var(--fg2)", title: t("sync.footer.none"), sub: t("sync.footer.none_sub"), spin: false };
    default:
      if (s.conflicts > 0)
        return { icon: "warning-circle", color: "var(--orange)", title: t("sync.footer.conflict", { n: s.conflicts }), sub: t("sync.footer.conflict_sub"), spin: false };
      return {
        icon: "cloud-check",
        color: "var(--green)",
        title: t("sync.footer.synced"),
        sub: t("sync.footer.synced_sub", { time: s.last_synced_at ? formatRelative(t.locale, s.last_synced_at) : t("time.never") }),
        spin: false,
      };
  }
}
