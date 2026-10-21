import { create } from "zustand";
import { hostById, useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { toast } from "@/components/overlay";
import { collectCredentials, editSessionHost } from "@/features/terminal/connect";
import { askSecret } from "@/features/terminal/prompts";
import { needsPassphrase } from "@/features/terminal/session";
import { t } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { TunnelLostEvent, TunnelsEvent, TunnelView } from "@/ipc/types";
import { windowFocused } from "@/lib/native";

/**
 * FWD-05: the forwards running without a terminal, as the backend last listed them. Each saved host has at most one
 * connection for them, which the backend opens with the first and closes with the last.
 */
const useStore = create<TunnelsEvent>(() => ({ version: -1, tunnels: [] }));

/** A list read before an event never replaces it. */
function apply(list: TunnelsEvent) {
  if (list.version >= useStore.getState().version) useStore.setState(list);
}

let started = false;

/** Wired once, like the terminal's listeners, so a connection lost behind the lock screen still notifies. */
export function startTunnelListeners() {
  if (started) return;
  started = true;
  void api.listen("ssh://tunnels", apply);
  void api.listen("ssh://tunnel-lost", (ev) => void notifyLost(ev));
  // The click brought the window back; the host editor starts the forwards again.
  void api.listen("app://notification-click", ({ host_id }) => {
    if (host_id && useApp.getState().phase === "unlocked") editSessionHost(host_id);
  });
  void api
    .tunnels_list()
    .then(apply)
    .catch(() => {});
}

/** A toast while the window has the focus, else a system notification, which opens the host editor. */
async function notifyLost({ host_id, host_name, error }: TunnelLostEvent) {
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

export function useTunnels(): TunnelView[] {
  return useStore((s) => s.tunnels);
}

export function useHostTunnel(hostId: string | null): TunnelView | undefined {
  return useStore((s) => (hostId ? s.tunnels.find((x) => x.host_id === hostId) : undefined));
}

/** The forward's local port while it runs without a terminal. */
export function tunnelPort(tunnel: TunnelView | undefined, forwardId: string): number | null {
  return tunnel?.forwards.find((f) => f.forward_id === forwardId)?.local_port ?? null;
}

/**
 * Starts a saved forward without a terminal and resolves to its local port, or to null when the user cancels a
 * question. The host's first one connects, asking what its sign-in needs as a terminal does (SSH-03).
 */
export async function startTunnelForward(hostId: string, forwardId: string): Promise<number | null> {
  startTunnelListeners();
  const host = hostById(hostId);
  const connected = useStore.getState().tunnels.some((x) => x.host_id === hostId);
  const creds = host && !connected ? await collectCredentials(host) : { password: null, passphrase: null };
  if (!creds) return null;
  let passphrase = creds.passphrase;
  for (let round = 0; ; round++) {
    try {
      const port = await api.tunnel_forward_start(hostId, forwardId, { password: creds.password, passphrase });
      // HOST-06: the host was connected to.
      if (!connected) void useVaultData.getState().reloadHosts().catch(() => {});
      return port;
    } catch (e) {
      const err = toAppError(e);
      if (!host || !needsPassphrase(err) || round >= 3) throw err;
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

export function stopTunnelForward(hostId: string, forwardId: string): Promise<void> {
  return api.tunnel_forward_stop(hostId, forwardId);
}
