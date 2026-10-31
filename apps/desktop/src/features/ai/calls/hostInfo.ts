import { formatTelnetTarget } from "@/session/quickConnect";
import { isTelnetHost, type AnyHostView } from "@/ipc/types";
import type { ShellDialect } from "../rules/commandRules";

export interface HostInfo {
  name: string;
  target: string;
  /** This computer, for a local tab (§7.5): its approval cards say so, and so do their Always allow labels. */
  local?: boolean;
  /** The syntax of a local tab's shell (`dialectOf`), which a command's allowances are named in; POSIX when absent. */
  dialect?: ShellDialect;
}

export function hostInfo(host: AnyHostView | undefined): HostInfo | null {
  if (!host) return null;
  return {
    name: host.name,
    target: isTelnetHost(host) ? `telnet ${formatTelnetTarget(host)}` : `${host.username}@${host.address}:${host.port}`,
  };
}
