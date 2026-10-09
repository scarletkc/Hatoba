import { create } from "zustand";
import { api } from "@/ipc/api";
import type { AuthPrompt, HostKeyPrompt } from "@/ipc/types";

/** A request for a one-off secret (SSH-03): never stored anywhere, only handed to `ssh_connect`. */
export interface SecretRequest {
  kind: "password" | "passphrase";
  /** Host display name. */
  hostName: string;
  /** `user@address:port`. */
  target: string;
  /** The previously supplied passphrase was rejected. */
  wrong?: boolean;
  /** The login password of a quick connection (HOST-12), not of a saved host. */
  oneOff?: boolean;
}

export type PromptItem =
  | { key: string; kind: "hostkey"; prompt: HostKeyPrompt }
  | { key: string; kind: "auth"; prompt: AuthPrompt }
  | { key: string; kind: "secret"; request: SecretRequest; resolve: (value: string | null) => void }
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
  if (prompt.password) {
    // A quick connection's login password is asked like an "ask every time" one (SSH-03).
    const request: SecretRequest = { kind: "password", hostName: prompt.target, target: prompt.target, oneOff: true };
    const resolve = (value: string | null) =>
      void api.auth_prompt_respond(prompt.request_id, value === null ? null : [value]).catch(() => {});
    push({ kind: "secret", request, resolve });
    return;
  }
  push({ kind: "auth", prompt });
}

/** Resolves with the typed secret, or `null` when the user cancels. */
export function askSecret(request: SecretRequest): Promise<string | null> {
  return new Promise((resolve) => push({ kind: "secret", request, resolve }));
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

export function answerUsername(item: PromptItem & { kind: "username" }, value: string | null) {
  remove(item.key);
  item.resolve(value);
}

export function answerSecret(item: PromptItem & { kind: "secret" }, value: string | null) {
  remove(item.key);
  item.resolve(value);
}
