import type { Channel } from "@tauri-apps/api/core";
import type { commands } from "./bindings";
import type { AppError, ErrorCode, EventMap, TermFrame } from "./types";

/** Receives raw terminal frames (see FRAME_* in types.ts). */
export type FrameHandler = (frame: Uint8Array) => void;
export type Unlisten = () => void;

type Commands = typeof commands;

/** `aiConversationPin` → `ai_conversation_pin`: the command under the name of its Rust function. */
type SnakeCase<S extends string, Done extends string = ""> = S extends `${infer C}${infer Rest}`
  ? SnakeCase<Rest, `${Done}${C extends Lowercase<C> ? C : `_${Lowercase<C>}`}`>
  : Done;

/** The channel a command streams on is a callback here; terminal frames reach it as bytes. */
type Param<P> = P extends Channel<infer T> ? ([T] extends [TermFrame] ? FrameHandler : (message: T) => void) : P;

/** A command whose Rust function returns `()` resolves to nothing. */
type Resolves<R> = R extends Promise<infer T> ? Promise<[T] extends [null] ? void : T> : never;

type Method<F> = F extends (...args: infer A) => infer R
  ? (...args: { [I in keyof A]: Param<A[I]> }) => Resolves<R>
  : never;

/**
 * Everything the UI can ask the backend to do (spec §10.1): every command tauri-specta exports in
 * `bindings.ts`, where its documentation is, under the snake_case name of its Rust function.
 * Methods reject with {@link AppError}.
 */
export type HatobaApi = { [K in keyof Commands as SnakeCase<K & string>]: Method<Commands[K]> } & {
  listen<K extends keyof EventMap>(event: K, handler: (payload: EventMap[K]) => void): Promise<Unlisten>;
};

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function isAppError(e: unknown): e is AppError {
  return typeof e === "object" && e !== null && "code" in e && "detail" in e;
}

/**
 * An error the frontend makes itself, such as the browser mock's: the fields `fields` leaves out
 * are null, as in every error Rust sends.
 */
export function appError(
  code: ErrorCode,
  detail: string,
  fields: Partial<Omit<AppError, "code" | "detail">> = {},
): AppError {
  return {
    code,
    detail,
    retry_at: null,
    field: null,
    ssh_kind: null,
    key_kind: null,
    permission: null,
    cf_code: null,
    http_status: null,
    attempts_left: null,
    ...fields,
  };
}

/** Normalise anything thrown by a command into an {@link AppError}. */
export function toAppError(e: unknown): AppError {
  if (isAppError(e)) return e;
  return appError("internal", e instanceof Error ? e.message : String(e));
}

let impl: Promise<HatobaApi> | null = null;

/** Cached promise so concurrent first calls share one backend instance. */
function load(): Promise<HatobaApi> {
  impl ??= isTauri()
    ? import("./tauri").then((m) => m.createTauriApi())
    : import("./mock").then((m) => m.createMockApi());
  return impl;
}

/**
 * The backend. Calls are forwarded lazily so the Tauri or mock implementation is chosen at runtime
 * (the mock lets the UI run in a plain browser with sample data).
 */
export const api: HatobaApi = new Proxy({} as HatobaApi, {
  get(_target, prop: string) {
    return async (...args: unknown[]) => {
      const backend = await load();
      const fn = (backend as unknown as Record<string, (...a: unknown[]) => Promise<unknown>>)[prop];
      if (typeof fn !== "function") throw toAppError(new Error(`unknown command ${prop}`));
      try {
        return await fn.apply(backend, args);
      } catch (e) {
        throw toAppError(e);
      }
    };
  },
});
