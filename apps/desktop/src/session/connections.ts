import { create } from "zustand";
import { api } from "@/ipc/api";
import type { AppError, ConnectionsEvent, HostConnectionView, RunningForwardView } from "@/ipc/types";

/**
 * SSH-16: each saved host's open connection, which its terminal tabs and port forwards share, and the forwards on
 * it, as the backend last listed them.
 */
const useStore = create<ConnectionsEvent>(() => ({ version: -1, connections: [] }));

/** A list read before an event never replaces it. */
function apply(list: ConnectionsEvent) {
  if (list.version >= useStore.getState().version) useStore.setState(list);
}

let started = false;

/** Wired once and never removed, like the terminal's listeners. */
export function startConnectionListeners() {
  if (started) return;
  started = true;
  void api.listen("ssh://connections", apply);
  void api
    .connections_list()
    .then(apply)
    .catch(() => {});
}

export const getConnections = (): HostConnectionView[] => useStore.getState().connections;

export function useConnections(): HostConnectionView[] {
  return useStore((s) => s.connections);
}

export function useHostConnection(hostId: string | null): HostConnectionView | undefined {
  return useStore((s) => (hostId ? s.connections.find((x) => x.host_id === hostId) : undefined));
}

/** Whether the host has a connection open, which a new tab or forward joins without signing in. */
export const hostConnected = (hostId: string): boolean => getConnections().some((x) => x.host_id === hostId);

/** The forward while it runs, and the address it listens on: an edit to its rule applies at its next start. */
export function runningForward(conn: HostConnectionView | undefined, forwardId: string): RunningForwardView | null {
  return conn?.forwards.find((f) => f.forward_id === forwardId) ?? null;
}

/** Why the forward failed to start on the host's connection, until it starts or stops. */
export function forwardFailure(conn: HostConnectionView | undefined, forwardId: string): AppError | null {
  return conn?.failed.find((f) => f.forward_id === forwardId)?.error ?? null;
}
