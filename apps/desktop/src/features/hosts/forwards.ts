import { useMemo } from "react";
import { create } from "zustand";
import { hostById, useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { toast } from "@/components/overlay";
import { collectCredentials } from "@/features/terminal/connect";
import { askPassword, askSecret } from "@/features/terminal/prompts";
import { needsPassphrase, needsPassword } from "@/features/terminal/session";
import { t } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { ForwardsLostEvent, ForwardView } from "@/ipc/types";
import { windowFocused } from "@/lib/native";
import { hostConnected, startConnectionListeners } from "./connections";

/** Every host's saved forwards (FWD-05); `null` until read. */
const useRules = create<{ rules: ForwardView[] | null; loadFailed: boolean }>(() => ({ rules: null, loadFailed: false }));
/** Grows with each read and each change, so a read that began before a change never undoes it. */
let stamp = 0;

export async function loadForwardRules(): Promise<void> {
  const mine = ++stamp;
  try {
    const rules = await api.forwards_list_all();
    if (mine === stamp) useRules.setState({ rules, loadFailed: false });
  } catch {
    if (mine === stamp) useRules.setState({ loadFailed: true });
  }
}

/** A rule was saved. */
export function putForwardRule(saved: ForwardView) {
  stamp++;
  useRules.setState(({ rules }) => {
    const rest = rules ?? [];
    return { rules: rest.some((x) => x.id === saved.id) ? rest.map((x) => (x.id === saved.id ? saved : x)) : [...rest, saved] };
  });
}

/** A rule was deleted. */
export function dropForwardRule(id: string) {
  stamp++;
  useRules.setState(({ rules }) => ({ rules: rules && rules.filter((x) => x.id !== id) }));
}

export const getForwardRules = (): ForwardView[] | null => useRules.getState().rules;

export function useForwardRules(): { rules: ForwardView[] | null; loadFailed: boolean } {
  const rules = useRules((s) => s.rules);
  const loadFailed = useRules((s) => s.loadFailed);
  return { rules, loadFailed };
}

/** The host's saved forwards; `null` until read. */
export function useHostForwardRules(hostId: string | null): ForwardView[] | null {
  const rules = useRules((s) => s.rules);
  return useMemo(() => (hostId && rules ? rules.filter((f) => f.host_id === hostId) : null), [rules, hostId]);
}

let started = false;

/** Wired once and never removed, like the terminal's listeners. */
export function startForwardListeners() {
  if (started) return;
  started = true;
  startConnectionListeners();
  void api.listen("ssh://forwards-lost", (ev) => void notifyLost(ev));
  // The click brought the window back; the forwards start again on the Port Forwarding page.
  void api.listen("app://notification-click", ({ host_id }) => {
    if (host_id && useApp.getState().phase === "unlocked") openForwards(host_id);
  });
  void loadForwardRules();
  // Read again when the hosts are: a sync may have brought forwards from another device.
  useVaultData.subscribe((s, prev) => {
    if (s.hosts !== prev.hosts) void loadForwardRules();
  });
}

/** The Port Forwarding page in the home tab, with the host's section in view. */
export function openForwards(hostId: string | null) {
  useApp.getState().navigate({ kind: "forwards", hostId });
  useTabs.getState().activate("home");
}

/** A toast while the window has the focus, else a system notification, which opens the host's forwards. */
async function notifyLost({ host_id, host_name, error }: ForwardsLostEvent) {
  const reason = errorMessage(t, error);
  if (await windowFocused()) {
    toast(t("hosts.fwd.lostToast", { host: host_name, reason }), "error");
    return;
  }
  await api
    .notification_show({
      title: host_name,
      body: t("hosts.fwd.lost", { reason }),
      target: { tab: null, conversation_id: null, host_id },
    })
    .catch(() => {});
}

/**
 * Starts a saved forward on the host's connection (SSH-16) and resolves to its local port, or to null when the user
 * cancels a question. With nothing open on the host, it connects, asking what its sign-in needs as a terminal does
 * (SSH-03).
 */
export async function startForward(hostId: string, forwardId: string): Promise<number | null> {
  startForwardListeners();
  const host = hostById(hostId);
  const connected = hostConnected(hostId);
  const creds = host ? await collectCredentials(host) : { password: null, passphrase: null };
  if (!creds) return null;
  let { password, passphrase } = creds;
  for (let round = 0; ; round++) {
    try {
      const port = await api.forward_start(hostId, forwardId, { password, passphrase });
      // HOST-06: the host was connected to.
      if (!connected) void useVaultData.getState().reloadHosts().catch(() => {});
      return port;
    } catch (e) {
      const err = toAppError(e);
      if (!host || round >= 3) throw err;
      if (needsPassword(err) && password === null && host.auth_kind === "ask") {
        // The host's connection closed after its password was skipped.
        password = await askPassword(host);
        if (password === null) return null;
        continue;
      }
      if (!needsPassphrase(err)) throw err;
      // The backend has no (or a wrong) saved passphrase: ask once, use it, forget it.
      const answer = await askSecret({
        kind: "passphrase",
        hostName: host.name,
        target: `${host.username}@${host.address}:${host.port}`,
        wrong: err.key_kind === "wrong_passphrase",
      });
      if (answer === null) return null;
      passphrase = answer;
    }
  }
}

export function stopForward(hostId: string, forwardId: string): Promise<void> {
  return api.forward_stop(hostId, forwardId);
}
