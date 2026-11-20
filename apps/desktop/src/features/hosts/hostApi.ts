import type { Page } from "@/app/store";
import { api } from "@/ipc/api";
import { isTelnetHost, type AnyHostView } from "@/ipc/types";
import { connectHost, connectTelnetHost } from "@/session/connect";

export interface HostActions {
  editRoute: (back: string | null) => Page;
  connect: () => Promise<void> | void;
  setFavorite: (favorite: boolean) => Promise<void>;
  duplicate: () => Promise<AnyHostView>;
  remove: () => Promise<void>;
  addressText: () => string;
  copyPassword: () => Promise<void>;
}

/** Dispatches host actions and formatting to the SSH or Telnet backend API for `host`. */
export function hostApi(host: AnyHostView): HostActions {
  if (isTelnetHost(host)) {
    return {
      editRoute: (back) => ({ kind: "telnet-edit", hostId: host.id, groupId: null, back }),
      connect: () => connectTelnetHost(host.id),
      setFavorite: (favorite) => api.telnet_host_set_favorite(host.id, favorite),
      duplicate: () => api.telnet_host_duplicate(host.id),
      remove: () => api.telnet_host_delete(host.id),
      addressText: () => `${host.address}${host.port !== 23 ? ` ${host.port}` : ""}`,
      copyPassword: () => api.telnet_host_copy_password(host.id),
    };
  }
  return {
    editRoute: (back) => ({ kind: "host-edit", hostId: host.id, groupId: null, back }),
    connect: () => connectHost(host.id),
    setFavorite: (favorite) => api.host_set_favorite(host.id, favorite),
    duplicate: () => api.host_duplicate(host.id),
    remove: () => api.host_delete(host.id),
    addressText: () =>
      `${host.username ? `${host.username}@` : ""}${host.address}${host.port !== 22 ? ` -p ${host.port}` : ""}`,
    copyPassword: () => api.host_copy_password(host.id),
  };
}
