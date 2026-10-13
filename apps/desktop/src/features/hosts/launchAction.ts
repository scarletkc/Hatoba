import type { HostView } from "@/ipc/types";
import { parseQuickConnect, savedHostFor, type TypedTarget } from "./quickConnect";

/** What a `hatoba …` command line asks for (HOST-13); null when it only brings the window up. */
export type LaunchAction =
  | { kind: "host"; hostId: string }
  | { kind: "target"; target: TypedTarget }
  | { kind: "search"; text: string }
  | null;

/** Text that reads as ssh syntax: a user, a URL, or an option. */
const SSH_SYNTAX = /@|:\/\/|(^|\s)-/;

/**
 * Reads a command line. The name of one saved host connects that host. Anything else is read as
 * the arguments of `ssh`, with or without the word, and connects as quick connect does (HOST-12):
 * as the saved host it matches, if there is one. What cannot connect goes to the search box, which
 * says why for ssh syntax and searches the saved hosts for other words.
 */
export function launchAction(args: string[], hosts: HostView[]): LaunchAction {
  const text = args.join(" ").trim();
  if (!text) return null;
  const named = hosts.filter((h) => h.name.trim().toLowerCase() === text.toLowerCase());
  if (named.length === 1) return { kind: "host", hostId: named[0].id };
  if (named.length > 1) return { kind: "search", text };
  const command = /^ssh(\s|$)/i.test(text) ? text : `ssh ${text}`;
  const parsed = parseQuickConnect(command);
  if (parsed?.ok) {
    const saved = savedHostFor(hosts, parsed.target);
    return saved ? { kind: "host", hostId: saved.id } : { kind: "target", target: parsed.target };
  }
  return { kind: "search", text: parsed && (command === text || SSH_SYNTAX.test(text)) ? command : text };
}
