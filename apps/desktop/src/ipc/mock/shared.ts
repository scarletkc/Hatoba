import { appError } from "../api";
import type { AppError, EventMap, GroupView, HostView, KeyView, LocalPrefs, ProxyView, TelnetHostView } from "../types";

export type MockEmit = <K extends keyof EventMap>(event: K, payload: EventMap[K]) => void;

export interface MockState {
  hosts: HostView[];
  telnetHosts: TelnetHostView[];
  groups: GroupView[];
  keys: KeyView[];
  proxies: ProxyView[];
  prefs: LocalPrefs;
}

export interface MockDeps {
  state: MockState;
  emit: MockEmit;
  needUnlocked(): void;
  id(prefix: string): string;
  touch(): void;
  placeNewHost(after?: string): number;
  storePrefs(): void;
}

export function fail(code: AppError["code"], detail: string = code, extra: Partial<AppError> = {}): never {
  throw appError(code, detail, extra);
}

export const delay = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

export function frame(tag: number, payload: Uint8Array): Uint8Array {
  const out = new Uint8Array(payload.length + 1);
  out[0] = tag;
  out.set(payload, 1);
  return out;
}
