import { Channel } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { FrameHandler, HatobaApi } from "./api";
import { commands } from "./bindings";

/** The commands whose channel streams terminal frames. */
type FrameCommand = {
  [K in keyof HatobaApi]: FrameHandler extends Parameters<HatobaApi[K]>[number] ? K : never;
}[keyof HatobaApi];

/** Every command that streams terminal frames, so a new one fails the type check until it is here. */
const FRAME_COMMANDS: Record<FrameCommand, true> = { ssh_connect: true, ssh_connect_target: true, local_open: true };

/** `aiConversationPin` → `ai_conversation_pin`, as `HatobaApi` names the command. */
const snakeCase = (name: string) => name.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);

/** The channel behind a command's callback. Terminal frames arrive as an ArrayBuffer or an array of numbers. */
function channelFor(callback: (message: never) => void, frames: boolean): Channel<unknown> {
  const channel = new Channel<unknown>();
  channel.onmessage = frames
    ? (msg) =>
        (callback as FrameHandler)(msg instanceof ArrayBuffer ? new Uint8Array(msg) : Uint8Array.from(msg as number[]))
    : (callback as (message: unknown) => void);
  return channel;
}

/**
 * Tauri implementation of {@link HatobaApi}, on the command functions tauri-specta generates: each
 * callback argument becomes the channel the command streams on.
 */
export function createTauriApi(): HatobaApi {
  const api: Record<string, unknown> = {
    listen: (event: string, handler: (payload: unknown) => void) => tauriListen(event, (e) => handler(e.payload)),
  };
  for (const [name, command] of Object.entries(commands)) {
    const key = snakeCase(name);
    const frames = key in FRAME_COMMANDS;
    api[key] = (...args: unknown[]) =>
      (command as (...a: unknown[]) => Promise<unknown>)(
        ...args.map((arg) => (typeof arg === "function" ? channelFor(arg as (message: never) => void, frames) : arg)),
      );
  }
  return api as HatobaApi;
}
