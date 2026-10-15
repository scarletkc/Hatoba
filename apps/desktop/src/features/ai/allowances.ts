import { create } from "zustand";
import type { TabSource } from "@/app/tabs";
import { formatTarget } from "@/features/hosts/quickConnect";
import { api } from "@/ipc/api";
import type { AiAllowance } from "@/ipc/types";
import { recordedTarget } from "./place";
import type { HostRef } from "./tools";

/**
 * The target a local tab's calls are approved for (AI-19): approvals are kept on the device, so it names
 * this computer, and a quick-connect target always has a user and a port, so it is never one.
 */
export const LOCAL_TARGET = "local";

/** The host a tab's calls are approved for (AI-19). */
export function hostRefOf(source: TabSource): HostRef {
  if (source.local) return { host_id: null, target: LOCAL_TARGET };
  return source.hostId !== null ? { host_id: source.hostId, target: null } : { host_id: null, target: formatTarget(recordedTarget(source.target)) };
}

/** AI-19: the approvals saved on this device, which Rust keeps sealed; null until read after an unlock. */
export const useAllowances = create<{ list: AiAllowance[] | null }>(() => ({ list: null }));

/** Bumped by the lock, so a read that was on its way then stores nothing. */
let generation = 0;
let loading: Promise<AiAllowance[]> | null = null;

/** The saved approvals, read once after each unlock. */
export function savedAllowances(): Promise<AiAllowance[]> {
  const list = useAllowances.getState().list;
  if (list) return Promise.resolve(list);
  if (!loading) {
    const gen = generation;
    loading = Promise.resolve()
      .then(() => api.ai_allowances_list())
      .then((read) => {
        if (gen === generation) useAllowances.setState({ list: read });
        return read;
      })
      .finally(() => {
        loading = null;
      });
  }
  return loading;
}

/** Saves approvals on this device (Always allow on an approval card). */
export async function addAllowances(list: AiAllowance[]) {
  for (const a of list) {
    const gen = generation;
    const next = await api.ai_allowance_add(a);
    if (gen === generation) useAllowances.setState({ list: next });
  }
}

/** Removes a saved approval (Settings → AI). */
export async function removeAllowance(a: AiAllowance) {
  const gen = generation;
  const next = await api.ai_allowance_remove(a);
  if (gen === generation) useAllowances.setState({ list: next });
}

/** The lock: what was read goes, since it names hosts. */
export function resetAllowances() {
  generation++;
  loading = null;
  useAllowances.setState({ list: null });
}
