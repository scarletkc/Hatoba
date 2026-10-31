import type { HostView, TelnetHostView } from "@/ipc/types";
import {
  parseQuickConnect,
  parseTelnet,
  savedHostFor,
  savedTelnetHostFor,
  type TelnetTarget,
  type TypedTarget,
} from "@/session/quickConnect";

/**
 * What a `hatoba …` command line asks for (HOST-13); null when it only brings the window up. A saved
 * host is an SSH or a Telnet one.
 */
export type LaunchAction =
  | { kind: "host"; hostId: string }
  | { kind: "target"; target: TypedTarget }
  | { kind: "telnet"; target: TelnetTarget }
  | { kind: "search"; text: string }
  | null;

/** Text that reads as ssh syntax: a user, a URL, or an option. */
const SSH_SYNTAX = /@|:\/\/|(^|\s)-/;
/** Text that reads as telnet syntax (TELNET-05): the command, or a `telnet://` URL. */
const TELNET_SYNTAX = /^telnet(\s|:\/\/|$)/i;

/**
 * Reads a command line. The name of one saved host, SSH or Telnet, connects that host. `telnet host
 * [port]` and `telnet://host[:port]` connect over Telnet, and anything else is read as the arguments
 * of `ssh`, with or without the word; both connect as quick connect does (HOST-12, TELNET-05): as the
 * saved host they match, if there is one. What cannot connect goes to the search box, which says why
 * for ssh and telnet syntax and searches the saved hosts for other words.
 */
export function launchAction(args: string[], hosts: HostView[], telnetHosts: TelnetHostView[]): LaunchAction {
  const text = args.join(" ").trim();
  if (!text) return null;
  const named = [...hosts, ...telnetHosts].filter((h) => h.name.trim().toLowerCase() === text.toLowerCase());
  if (named.length === 1) return { kind: "host", hostId: named[0].id };
  if (named.length > 1) return { kind: "search", text };
  if (TELNET_SYNTAX.test(text)) {
    const telnet = parseTelnet(text);
    if (!telnet?.ok) return { kind: "search", text };
    const saved = savedTelnetHostFor(telnetHosts, telnet.target);
    return saved ? { kind: "host", hostId: saved.id } : { kind: "telnet", target: telnet.target };
  }
  const command = /^ssh(\s|$)/i.test(text) ? text : `ssh ${text}`;
  const parsed = parseQuickConnect(command);
  if (parsed?.ok) {
    const saved = savedHostFor(hosts, parsed.target);
    return saved ? { kind: "host", hostId: saved.id } : { kind: "target", target: parsed.target };
  }
  return { kind: "search", text: parsed && (command === text || SSH_SYNTAX.test(text)) ? command : text };
}
