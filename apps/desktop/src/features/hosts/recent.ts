import { create } from "zustand";
import { errorMessage } from "@/app/errors";
import { toast } from "@/components/overlay";
import { t } from "@/i18n";
import { api } from "@/ipc/api";
import type { QuickTarget } from "@/ipc/types";
import { sameTarget } from "./quickConnect";

/** The recent quick-connect targets (HOST-12). The backend keeps the list, on this device only. */
interface RecentTargets {
  targets: QuickTarget[];
  load(): Promise<void>;
  remove(target: QuickTarget): Promise<void>;
}

export const useRecentTargets = create<RecentTargets>((set) => ({
  targets: [],
  load: async () => {
    try {
      set({ targets: await api.recent_targets_list() });
    } catch {
      /* suggestions are best effort */
    }
  },
  remove: async (target) => {
    set((s) => ({ targets: s.targets.filter((r) => !sameTarget(r, target)) }));
    try {
      set({ targets: await api.recent_target_remove(target) });
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  },
}));
