import type { HostLike } from "./order";

/** Lower-cased searchable text of one host, built once per vault reload (HOST-05). */
export interface HostIndexEntry<T extends HostLike = HostLike> {
  host: T;
  name: string;
  address: string;
  user: string;
  tags: string[];
  /** `user@address:port`, so queries like "deploy@43." or "host:2222" work. */
  full: string;
}

export function buildHostIndex<T extends HostLike>(hosts: T[]): HostIndexEntry<T>[] {
  return hosts.map((host) => ({
    host,
    name: host.name.toLowerCase(),
    address: host.address.toLowerCase(),
    user: host.username.toLowerCase(),
    tags: host.tags.map((t) => t.toLowerCase()),
    full: `${host.username}@${host.address}:${host.port}`.toLowerCase(),
  }));
}

const BOUNDARY = /[\s\-_.@:/]/;

/**
 * Score one query token against one field. Exact > prefix > word-start > substring > subsequence;
 * 0 means no match. Cheap: a handful of indexOf calls and one linear scan.
 */
function scoreField(field: string, token: string, allowFuzzy: boolean): number {
  if (!field) return 0;
  if (field === token) return 100;
  if (field.startsWith(token)) return 90;
  const at = field.indexOf(token);
  if (at > 0) {
    if (BOUNDARY.test(field[at - 1])) return 80;
    return 60 - Math.min(20, at);
  }
  if (allowFuzzy && token.length >= 2) {
    // Subsequence: every character of the token appears in order ("pat" ~ "prod-api-tokyo").
    let from = 0;
    let gaps = 0;
    let last = -1;
    for (let i = 0; i < token.length; i++) {
      const found = field.indexOf(token[i], from);
      if (found === -1) return 0;
      if (last !== -1 && found !== last + 1) gaps++;
      last = found;
      from = found + 1;
    }
    return Math.max(5, 30 - gaps * 4);
  }
  return 0;
}

function scoreToken(e: HostIndexEntry, token: string): number {
  let best = scoreField(e.name, token, true);
  best = Math.max(best, scoreField(e.address, token, true) * 0.8);
  best = Math.max(best, scoreField(e.user, token, false) * 0.7);
  for (const tag of e.tags) best = Math.max(best, scoreField(tag, token, false) * 0.85);
  if (best === 0) best = scoreField(e.full, token, false) * 0.6;
  return best;
}

/**
 * Case-insensitive token search over name, address, username and tags. Every whitespace-separated
 * token must match; results are ordered best-first and keep the incoming order for equal scores.
 */
export function searchHosts<T extends HostLike>(index: HostIndexEntry<T>[], query: string): T[] {
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return index.map((e) => e.host);
  const scored: { host: T; score: number; order: number }[] = [];
  index.forEach((e, order) => {
    let total = 0;
    for (const token of tokens) {
      const s = scoreToken(e, token);
      if (s === 0) return;
      total += s;
    }
    scored.push({ host: e.host, score: total, order });
  });
  scored.sort((a, b) => b.score - a.score || a.order - b.order);
  return scored.map((s) => s.host);
}
