import { create } from "zustand";
import { api } from "@/ipc/api";
import type { AuthBanner, AuthPrompt, AuthPromptClosed, HostKeyPrompt, HostView } from "@/ipc/types";

/**
 * A request for a one-off secret (SSH-03): never stored anywhere, only handed to `ssh_connect`, or
 * a sudo password for `hatoba exec --sudo` and `hatoba cp --sudo` (HOST-15).
 */
export interface SecretRequest {
  kind: "password" | "passphrase" | "sudo";
  /** Host display name. */
  hostName: string;
  /** `user@address:port`. */
  target: string;
  /** The previously supplied passphrase or sudo password was rejected. */
  wrong?: boolean;
  /** The login password of a quick connection (HOST-12), not of a saved host. */
  oneOff?: boolean;
}

export type PromptItem =
  | { key: string; kind: "hostkey"; prompt: HostKeyPrompt }
  | { key: string; kind: "auth"; prompt: AuthPrompt }
  /** A message the server sent while signing in, and holds the login after (SSH-08). */
  | { key: string; kind: "banner"; banner: AuthBanner }
  | {
      key: string;
      kind: "secret";
      request: SecretRequest;
      resolve: (value: string | null) => void;
      /** The backend session asking, for a quick connection's login password. */
      sessionId?: string | null;
      /** The `ssh://auth-prompt` this answers. */
      requestId?: string;
    }
  | { key: string; kind: "username"; target: string; resolve: (value: string | null) => void };

interface PromptState {
  /** Pending prompts, oldest first; the dialog host shows `items[0]`. */
  items: PromptItem[];
}

export const usePrompts = create<PromptState>(() => ({ items: [] }));

let seq = 0;

type NewPrompt<T = PromptItem> = T extends PromptItem ? Omit<T, "key"> : never;

function push(item: NewPrompt) {
  usePrompts.setState((s) => ({ items: [...s.items, { ...item, key: `p${++seq}` } as PromptItem] }));
}

function remove(key: string) {
  usePrompts.setState((s) => ({ items: s.items.filter((i) => i.key !== key) }));
}

export function pushHostKeyPrompt(prompt: HostKeyPrompt) {
  push({ kind: "hostkey", prompt });
}

export function pushAuthPrompt(prompt: AuthPrompt) {
  if (prompt.sudo) {
    const request: SecretRequest = { kind: "sudo", hostName: prompt.name, target: prompt.target, wrong: prompt.wrong };
    const resolve = (value: string | null) =>
      void api.auth_prompt_respond(prompt.request_id, value === null ? null : [value]).catch(() => {});
    push({ kind: "secret", request, resolve, requestId: prompt.request_id });
    return;
  }
  if (prompt.password) {
    // A quick connection's login password is asked like an "ask every time" one (SSH-03).
    const request: SecretRequest = { kind: "password", hostName: prompt.target, target: prompt.target, oneOff: true };
    const resolve = (value: string | null) =>
      void api.auth_prompt_respond(prompt.request_id, value === null ? null : [value]).catch(() => {});
    push({ kind: "secret", request, resolve, sessionId: prompt.session_id, requestId: prompt.request_id });
    return;
  }
  push({ kind: "auth", prompt });
}

/** How long the server holds the login after a banner before the banner shows in a dialog. */
const HELD_MS = 1000;

/** Banners not shown yet, by request id: one the server answers within `HELD_MS` never shows. */
const held = new Map<string, { banner: AuthBanner; timer: number }>();

/**
 * A message the server sent while signing in, which shows in a dialog once the server has held the
 * login for `HELD_MS` after it, as Tailscale SSH's check mode does until the user signs in at its link.
 */
export function pushAuthBanner(banner: AuthBanner) {
  const timer = window.setTimeout(() => {
    held.delete(banner.request_id);
    push({ kind: "banner", banner });
  }, HELD_MS);
  held.set(banner.request_id, { banner, timer });
}

/** The tab of session `sessionId` closed while connecting: refuse its host key, cancel its logins. */
export function cancelSessionPrompts(sessionId: string) {
  for (const [id, { banner, timer }] of held) {
    if (banner.session_id !== sessionId) continue;
    window.clearTimeout(timer);
    held.delete(id);
    stopWaiting(id);
  }
  for (const item of usePrompts.getState().items) {
    if (item.kind === "hostkey" && item.prompt.session_id === sessionId) answerHostKey(item, false);
    else if (item.kind === "auth" && item.prompt.session_id === sessionId) answerAuth(item, null);
    else if (item.kind === "banner" && item.banner.session_id === sessionId) answerBanner(item);
    else if (item.kind === "secret" && item.sessionId === sessionId) answerSecret(item, null);
  }
}

/** The backend no longer waits for this prompt, so it closes unanswered. */
export function closeAuthPrompt({ request_id }: AuthPromptClosed) {
  const pending = held.get(request_id);
  if (pending) {
    window.clearTimeout(pending.timer);
    held.delete(request_id);
  }
  const waited = (i: PromptItem) =>
    (i.kind === "auth" && i.prompt.request_id === request_id) ||
    (i.kind === "banner" && i.banner.request_id === request_id) ||
    (i.kind === "secret" && i.requestId === request_id);
  usePrompts.setState((s) => ({ items: s.items.filter((i) => !waited(i)) }));
}

/** Resolves with the typed secret, or `null` when the user cancels. */
export function askSecret(request: SecretRequest): Promise<string | null> {
  return new Promise((resolve) => push({ kind: "secret", request, resolve }));
}

/** SSH-03: the password of an "ask every time" host, for this connection only; `null` when cancelled. */
export function askPassword(host: HostView): Promise<string | null> {
  return askSecret({
    kind: "password",
    hostName: host.name,
    target: `${host.username}@${host.address}:${host.port}`,
  });
}

/** Resolves with the user name to log in with, or `null` when the user cancels (HOST-12). */
export function askUsername(target: string): Promise<string | null> {
  return new Promise((resolve) => push({ kind: "username", target, resolve }));
}

/** Answer the host-key prompt. Anything but an explicit accept rejects the key (SSH-04). */
export function answerHostKey(item: PromptItem & { kind: "hostkey" }, accept: boolean) {
  remove(item.key);
  void api.hostkey_respond(item.prompt.request_id, accept).catch(() => {
    /* the request may have timed out on the backend */
  });
}

export function answerAuth(item: PromptItem & { kind: "auth" }, answers: string[] | null) {
  remove(item.key);
  void api.auth_prompt_respond(item.prompt.request_id, answers).catch(() => {});
}

/** The user stopped waiting on the server after its banner: the connection is cancelled. */
export function answerBanner(item: PromptItem & { kind: "banner" }) {
  remove(item.key);
  stopWaiting(item.banner.request_id);
}

/** Cancels the connection that waits on the server after banner `requestId`. */
export function stopWaiting(requestId: string) {
  void api.auth_prompt_respond(requestId, null).catch(() => {});
}

export function answerUsername(item: PromptItem & { kind: "username" }, value: string | null) {
  remove(item.key);
  item.resolve(value);
}

export function answerSecret(item: PromptItem & { kind: "secret" }, value: string | null) {
  remove(item.key);
  item.resolve(value);
}
