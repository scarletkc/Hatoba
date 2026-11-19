import type { HatobaApi } from "../api";
import type { KeyView, ProxyView } from "../types";
import * as D from "./data";
import { delay, fail, type MockDeps } from "./shared";

type KeysProxiesApi = Pick<
  HatobaApi,
  | "proxies_list"
  | "proxy_save"
  | "proxy_delete"
  | "keys_list"
  | "key_import"
  | "key_generate"
  | "key_rename"
  | "key_delete"
  | "key_public"
  | "key_deploy"
>;

export function createKeysProxiesMock(ctx: MockDeps): KeysProxiesApi {
  const { state } = ctx;

  function proxyViews(): ProxyView[] {
    return state.proxies
      .map((p) => ({
        ...p,
        host_ids: [...state.hosts, ...state.telnetHosts]
          .filter((h) => h.proxy_mode === "proxy" && h.proxy_id === p.id)
          .map((h) => h.id),
      }))
      .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
  }

  function keyViews(): KeyView[] {
    return state.keys.map((k) => ({
      ...k,
      used_by: state.hosts.filter((h) => h.auth_kind === "key" && h.key_id === k.id).map((h) => h.id),
    }));
  }

  return {
    proxies_list: async () => {
      ctx.needUnlocked();
      return proxyViews();
    },
    proxy_save: async (input) => {
      ctx.needUnlocked();
      if (!input.name.trim()) fail("invalid_input", "name is required", { field: "name" });
      const address = input.address.trim().replace(/^\[(.*)\]$/, "$1");
      if (!address) fail("invalid_input", "address is required", { field: "address" });
      if (/\s/.test(address)) fail("invalid_input", "address is not a valid host name or IP", { field: "address" });
      if (!input.port) fail("invalid_input", "port must be between 1 and 65535", { field: "port" });
      const username = input.username.trim();
      if (input.kind === "http" && username.includes(":"))
        fail("invalid_input", "an HTTP proxy username can't contain a colon", { field: "username" });
      const existing = input.id ? state.proxies.find((p) => p.id === input.id) : undefined;
      if (input.id && !existing) fail("not_found");
      const view: ProxyView = {
        id: existing?.id ?? ctx.id("p"),
        name: input.name.trim(),
        kind: input.kind,
        address,
        port: input.port,
        username,
        has_password: !!username && (input.password !== null ? input.password.length > 0 : !!existing?.has_password),
        host_ids: [],
        updated_at: Date.now(),
      };
      state.proxies = existing ? state.proxies.map((p) => (p.id === view.id ? view : p)) : [...state.proxies, view];
      ctx.touch();
      return proxyViews().find((p) => p.id === view.id)!;
    },
    proxy_delete: async (pid) => {
      if (!state.proxies.some((p) => p.id === pid)) fail("not_found");
      state.proxies = state.proxies.filter((p) => p.id !== pid);
      state.hosts = state.hosts.map((h) =>
        h.proxy_mode === "proxy" && h.proxy_id === pid ? { ...h, proxy_mode: "device_default", proxy_id: null } : h,
      );
      state.telnetHosts = state.telnetHosts.map((h) =>
        h.proxy_mode === "proxy" && h.proxy_id === pid ? { ...h, proxy_mode: "device_default", proxy_id: null } : h,
      );
      if (state.prefs.default_proxy_id === pid || state.prefs.app_proxy_id === pid) {
        state.prefs = {
          ...state.prefs,
          default_proxy_id: state.prefs.default_proxy_id === pid ? null : state.prefs.default_proxy_id,
          app_proxy_id: state.prefs.app_proxy_id === pid ? null : state.prefs.app_proxy_id,
        };
        ctx.storePrefs();
      }
      ctx.touch();
    },

    keys_list: async () => {
      ctx.needUnlocked();
      return keyViews();
    },
    key_import: async (input) => {
      await delay(300);
      const text = input.private_key ?? "";
      if (!input.path && !text.includes("PRIVATE KEY") && !text.includes("PuTTY-User-Key-File"))
        fail("key_parse", "unsupported format", { key_kind: "unsupported_format" });
      if (text.includes("ENCRYPTED") && !input.passphrase)
        fail("key_parse", "passphrase required", { key_kind: "passphrase_required" });
      const k: KeyView = {
        ...D.KEYS[1],
        id: ctx.id("k"),
        name: input.name || "imported-key",
        created_at: Date.now(),
        updated_at: Date.now(),
        has_passphrase: !!input.passphrase,
        used_by: [],
      };
      state.keys = [...state.keys, k];
      ctx.touch();
      return k;
    },
    key_generate: async (input) => {
      await delay(input.algorithm === "rsa" ? 1500 : 300);
      const k: KeyView = {
        id: ctx.id("k"),
        name: input.name,
        algorithm: input.algorithm,
        bits: input.algorithm === "rsa" ? 4096 : 256,
        public_key: `${input.algorithm === "rsa" ? "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQ" : "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI"}${Math.random().toString(36).slice(2, 30)} ${input.comment}`,
        fingerprint: `SHA256:${btoa(String(Math.random())).replace(/=/g, "").slice(0, 43)}`,
        comment: input.comment,
        has_passphrase: !!input.passphrase,
        created_at: Date.now(),
        updated_at: Date.now(),
        used_by: [],
      };
      state.keys = [...state.keys, k];
      ctx.touch();
      return k;
    },
    key_rename: async (kid, name) => {
      state.keys = state.keys.map((k) => (k.id === kid ? { ...k, name, updated_at: Date.now() } : k));
      ctx.touch();
      return keyViews().find((k) => k.id === kid)!;
    },
    key_delete: async (kid) => {
      state.keys = state.keys.filter((k) => k.id !== kid);
      state.hosts = state.hosts.map((h) => (h.key_id === kid ? { ...h, key_id: null, auth_kind: "ask" } : h));
      ctx.touch();
    },
    key_public: async (kid) => state.keys.find((k) => k.id === kid)?.public_key ?? fail("not_found"),
    key_deploy: async () => {
      await delay(900);
    },
  };
}
