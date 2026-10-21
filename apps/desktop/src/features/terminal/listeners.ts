import { useTransfers } from "@/features/sftp/transfers";
import { api } from "@/ipc/api";
import { pushAuthPrompt, pushHostKeyPrompt } from "./prompts";
import { routeStateEvent } from "./session";
import { watchTerminalAppearance } from "./theme";

let started = false;

/**
 * Backend → UI events for terminals, SFTP and the connection prompts. They are wired once at module
 * level (never removed) so a prompt that arrives while the lock screen is up is not lost.
 */
export function startSessionListeners() {
  if (started) return;
  started = true;
  void api.listen("ssh://state", routeStateEvent);
  void api.listen("ssh://hostkey-prompt", pushHostKeyPrompt);
  void api.listen("ssh://auth-prompt", pushAuthPrompt);
  void api.listen("transfer://progress", (ev) => useTransfers.getState().handle(ev));
  watchTerminalAppearance();
}
