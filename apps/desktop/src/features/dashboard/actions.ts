import { create } from "zustand";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useTabs } from "@/app/tabs";
import { toast } from "@/components/overlay";
import { connectHost } from "@/features/terminal/connect";
import { t } from "@/i18n";
import { api } from "@/ipc/api";
import type { GroupView, HostView } from "@/ipc/types";
import { leftOutBy, type DashboardSort } from "./logic";

/** Dashboard view state that survives leaving the page and coming back (HOST-14). */
export const useDashboardUi = create<{
  sort: DashboardSort;
  showLeftOut: boolean;
  setSort(sort: DashboardSort): void;
  setShowLeftOut(show: boolean): void;
}>((set) => ({
  sort: "list",
  showLeftOut: false,
  setSort: (sort) => set({ sort }),
  setShowLeftOut: (showLeftOut) => set({ showLeftOut }),
}));

/** **Open Terminal**: brings the host's tab to the front, or opens one (which takes over the dashboard's connection). */
export function openTerminal(hostId: string) {
  const tabs = useTabs.getState();
  const open = tabs.tabs.filter((tab) => tab.hostId === hostId);
  const tab = open.find((x) => x.status === "connected") ?? open[open.length - 1];
  if (tab) tabs.activate(tab.id);
  else void connectHost(hostId);
}

/** Leaves a host or a group out of the dashboard on this device, or brings it back. */
export async function setShownOnDashboard(id: string, shown: boolean) {
  try {
    await api.dashboard_set_shown(id, shown);
    await useVaultData.getState().reload();
  } catch (e) {
    toast(errorMessage(t, e), "error");
  }
}

/**
 * The switch a host's menu offers: **Include in Dashboard** for a host left out on its own, **Include Group in
 * Dashboard** for one whose group is left out (its own switch is on), else **Leave Out of Dashboard**.
 */
export function dashboardSwitch(host: HostView, groups: ReadonlyMap<string, GroupView>) {
  switch (leftOutBy(host, groups)) {
    case "host":
      return { key: "dashboard.include", icon: "eye", run: () => void setShownOnDashboard(host.id, true) } as const;
    case "group":
      return { key: "dashboard.includeGroup", icon: "eye", run: () => void setShownOnDashboard(host.group_id!, true) } as const;
    case null:
      return { key: "dashboard.leaveOut", icon: "eye-slash", run: () => void setShownOnDashboard(host.id, false) } as const;
  }
}

export function retryHost(hostId: string) {
  void api.dashboard_retry(hostId).catch((e) => toast(errorMessage(t, e), "error"));
}
