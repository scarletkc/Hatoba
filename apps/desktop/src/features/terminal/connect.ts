import { hostById, useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { toast } from "@/components/overlay";
import { formatTarget, savedHostFor } from "@/features/hosts/quickConnect";
import { t } from "@/i18n";
import type { HostView, LocalShellView, QuickTarget } from "@/ipc/types";
import { startSessionListeners } from "./listeners";
import { loadLocalShells, preferredShell, shellName } from "./localShells";
import { askSecret, askUsername } from "./prompts";
import { ensureSession, getSession, type Credentials } from "./session";

const NONE: Credentials = { password: null, passphrase: null };

/**
 * Ask for whatever the host's auth mode needs before connecting. `null` means the user cancelled.
 * "Ask every time" hosts get a password prompt (SSH-03); key passphrases are asked lazily, only if
 * the backend reports it has none (see LiveSession.connect).
 */
export async function collectCredentials(host: HostView): Promise<Credentials | null> {
  if (host.auth_kind !== "ask") return NONE;
  const password = await askSecret({
    kind: "password",
    hostName: host.name,
    target: `${host.username}@${host.address}:${host.port}`,
  });
  return password === null ? null : { password, passphrase: null };
}

/** Open a new terminal tab for a host and connect (HOST-07, TERM-01: the same host may have several tabs). */
export async function connectHost(hostId: string): Promise<void> {
  startSessionListeners();
  const host = hostById(hostId);
  if (!host) {
    toast(t("terminal.hostMissing"), "error");
    return;
  }
  const creds = await collectCredentials(host);
  if (!creds) return;
  const tabId = useTabs.getState().openSession({ hostId, target: null }, host.name);
  const session = ensureSession(tabId);
  // The view connects as soon as it has measured the terminal size.
  session?.setCredentials(creds);
}

/**
 * Quick connect (HOST-12): open a terminal tab on a target typed into the hosts search field, without
 * saving a host. Without a user name, log in as the local user, as ssh does; ask only when that is unknown.
 */
export async function connectTarget(typed: { address: string; port: number; username: string | null }): Promise<void> {
  startSessionListeners();
  const username = typed.username ?? (useApp.getState().info.local_user || (await askUsername(formatTarget(typed)))?.trim());
  if (!username) return;
  const target: QuickTarget = { address: typed.address, port: typed.port, username };
  useTabs.getState().openSession({ hostId: null, target }, `${username}@${target.address}`);
}

/**
 * LOCAL-01: open a local terminal tab on `shell`, or on the shell picked in Settings → Terminal, else
 * the system default.
 */
export async function openLocalTerminal(shell?: LocalShellView): Promise<void> {
  startSessionListeners();
  let chosen = shell;
  if (!chosen) {
    try {
      chosen = preferredShell(await loadLocalShells(), useApp.getState().prefs.local_shell);
    } catch (e) {
      toast(errorMessage(t, e), "error");
      return;
    }
  }
  if (!chosen) {
    toast(t("terminal.local.noShell"), "error");
    return;
  }
  const local = { id: chosen.id, name: chosen.name, kind: chosen.kind };
  useTabs.getState().openSession({ hostId: null, target: null, local }, shellName(local, t));
}

/** Retry / reconnect a tab in place (prompting again for "ask every time" hosts), or restart a local tab's shell. */
export async function reconnectSession(tabId: string): Promise<void> {
  const session = getSession(tabId);
  if (!session || session.status === "connecting") return;
  // A quick connection saved as a host since (Save as Host…) reconnects as that host.
  if (session.local) {
    await session.reconnect(async () => NONE);
    return;
  }
  const saved = session.target ? savedHostFor(useVaultData.getState().hosts, session.target) : undefined;
  if (saved) session.adoptHost(saved);
  const host = hostById(session.hostId);
  await session.reconnect(async () => (host ? collectCredentials(host) : NONE));
}

/** Disconnect (if needed) and close a session tab. */
export async function closeSessionTab(tabId: string): Promise<void> {
  const session = getSession(tabId);
  useTabs.getState().closeTab(tabId);
  await session?.dispose();
}

/** Jump to the host's edit page (from the error card / menu). */
export function editSessionHost(hostId: string | null) {
  const host = hostById(hostId);
  if (!host) return;
  useApp.getState().navigate({ kind: "host-edit", hostId: host.id, groupId: host.group_id, back: { kind: "all" } });
  useTabs.getState().activate("home");
}

/** "Save as Host…" for a quick connection: a new host, filled in with its target (HOST-12). */
export function saveTargetAsHost(target: QuickTarget) {
  useApp.getState().navigate({ kind: "host-edit", hostId: null, groupId: null, back: { kind: "all" }, prefill: target });
  useTabs.getState().activate("home");
}
