import type { HostView, QuickTarget } from "@/ipc/types";

/** A destination typed into the hosts search field; `username` is null when none was given. */
export interface TypedTarget {
  address: string;
  port: number;
  username: string | null;
}

/** Why a typed ssh command cannot be quick-connected. */
export type QuickProblem = { kind: "option"; option: string } | { kind: "command" } | { kind: "port" };

/**
 * What the hosts search field makes of its input (HOST-12). `search` is the text the saved hosts are
 * filtered with: the destination without the ssh syntax around it.
 */
export type QuickParse =
  | { ok: true; target: TypedTarget; search: string }
  | { ok: false; problem: QuickProblem; search: string };

/** OpenSSH options that take an argument, so their value is not mistaken for the destination. */
const WITH_ARGUMENT = new Set("bceilmopBDEFIJLOPQRSwW".split(""));
const HOSTNAME = /^[\p{L}\p{N}_](?:[\p{L}\p{N}._-]*)$/u;
const IPV6 = /^[0-9a-f]*:[0-9a-f:.]*(?:%[\w.-]+)?$/i;

/** 1–65535, or NaN. */
function parsePort(text: string): number {
  if (!/^\d{1,5}$/.test(text)) return NaN;
  const port = Number(text);
  return port >= 1 && port <= 65535 ? port : NaN;
}

/** `[user@]host[:port]`, `[user@][v6]:port` or `ssh://[user@]host[:port]`; null when it is none of these. */
function parseDestination(token: string): { address: string; port: number | null; username: string | null } | null {
  let rest = token;
  if (/^ssh:\/\//i.test(rest)) rest = rest.slice("ssh://".length).replace(/\/$/, "");
  let username: string | null = null;
  // Like ssh, the last "@" separates the user, which may contain one itself (user@domain@host).
  const at = rest.lastIndexOf("@");
  if (at >= 0) {
    username = rest.slice(0, at);
    rest = rest.slice(at + 1);
    if (!username) return null;
  }
  let address: string;
  let portText: string | null = null;
  if (rest.startsWith("[")) {
    const close = rest.indexOf("]");
    if (close < 0) return null;
    address = rest.slice(1, close);
    const after = rest.slice(close + 1);
    if (after) {
      if (!after.startsWith(":")) return null;
      portText = after.slice(1);
    }
    if (!IPV6.test(address)) return null;
  } else {
    const colons = rest.split(":").length - 1;
    if (colons === 1) {
      [address, portText] = rest.split(":");
      if (!HOSTNAME.test(address)) return null;
    } else {
      address = rest;
      if (!(colons > 1 ? IPV6 : HOSTNAME).test(address)) return null;
    }
  }
  const port = portText ? parsePort(portText) : null;
  return { address, port, username };
}

/**
 * Reads `user@host[:port]`, `ssh://…` and `ssh [-p port] [-l user] [user@]host` (HOST-12). Plain
 * words return null: the input is only a search. So does an ssh command that is not complete yet.
 */
export function parseQuickConnect(input: string): QuickParse | null {
  const text = input.trim();
  const command = /^ssh\s/i.test(text);
  if (!command && !text.includes("@") && !/^ssh:\/\//i.test(text)) return null;
  const tokens = (command ? text.replace(/^ssh\s+/i, "") : text).split(/\s+/).filter(Boolean);

  let portOption: number | null = null;
  let userOption: string | null = null;
  let problem: QuickProblem | null = null;
  const positionals: string[] = [];
  for (let i = 0; i < tokens.length; i++) {
    const token = tokens[i];
    // Options may follow the destination, but everything after the next word is the remote command.
    if (positionals.length < 2 && token.length > 1 && token.startsWith("-")) {
      const flag = token[1];
      const attached = token.slice(2);
      const value = WITH_ARGUMENT.has(flag) ? attached || tokens[++i] : undefined;
      if (WITH_ARGUMENT.has(flag) && value === undefined) return null; // still typing the value
      if (flag === "p") portOption = parsePort(value!);
      else if (flag === "l") userOption = value!;
      else problem ??= { kind: "option", option: `-${flag}` };
      continue;
    }
    positionals.push(token);
  }
  if (positionals.length === 0) return null;
  const dest = parseDestination(positionals[0]);
  if (!dest) return null;
  // Typed without "ssh", the input must be the destination itself; anything more is a search.
  if (!command && (positionals.length > 1 || !(positionals[0].includes("@") || /^ssh:\/\//i.test(positionals[0])))) return null;
  if (positionals.length > 1) problem ??= { kind: "command" };

  // As in ssh, -l and -p win over the user and port in the destination.
  const username = userOption ?? dest.username;
  const port = portOption ?? dest.port ?? 22;
  const search = `${username ? `${username}@` : ""}${dest.address}`;
  if (!problem && Number.isNaN(port)) problem = { kind: "port" };
  if (problem) return { ok: false, problem, search };
  return { ok: true, target: { address: dest.address, port, username: username || null }, search };
}

/** The address as it goes before `:port`: an IPv6 address in brackets. */
export function bracketHost(address: string): string {
  return address.includes(":") ? `[${address}]` : address;
}

/** `user@host:port`, with an IPv6 address in brackets. */
export function formatTarget(t: { address: string; port: number; username: string | null }): string {
  return `${t.username ? `${t.username}@` : ""}${bracketHost(t.address)}:${t.port}`;
}

const bare = (address: string) => address.replace(/^\[(.*)\]$/, "$1").toLowerCase();

export function sameTarget(a: TypedTarget | QuickTarget, b: TypedTarget | QuickTarget): boolean {
  return bare(a.address) === bare(b.address) && a.port === b.port && a.username === b.username;
}

/**
 * The saved host a typed target is, so connecting uses its key and jump host: same address, port and
 * user. Without a user, the one saved host at that address and port, if there is only one.
 */
export function savedHostFor(hosts: HostView[], target: TypedTarget | QuickTarget): HostView | undefined {
  const address = bare(target.address);
  const matches = hosts.filter(
    (h) => bare(h.address) === address && h.port === target.port && (target.username === null || h.username === target.username),
  );
  return target.username === null && matches.length > 1 ? undefined : matches[0];
}
