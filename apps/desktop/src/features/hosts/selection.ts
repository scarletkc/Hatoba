/** The modifier keys of a click on a host, or of an arrow key that moves to one. */
export interface PickKeys {
  /** Shift: the hosts from the anchor on. */
  range: boolean;
  /** Ctrl, or ⌘ on macOS: keep the hosts already selected. */
  add: boolean;
}

export const ALONE: PickKeys = { range: false, add: false };

/**
 * The selection after picking host `id` among `hosts`, the saved hosts listed, in order. `range` selects the hosts
 * from `anchor` (the host last picked without it) to `id`; `add` keeps the hosts already selected, and on its own
 * adds `id` or takes it out again. `id` comes last, as the row the arrow keys move on from. Quick connect rows are
 * not in `hosts` and drop out, as they are only ever selected alone.
 */
export function pickHost(
  hosts: readonly string[],
  selected: readonly string[],
  anchor: string | null,
  id: string,
  keys: PickKeys,
): string[] {
  if (!keys.range && !keys.add) return [id];
  const listed = new Set(hosts);
  const kept = keys.add ? selected.filter((x) => listed.has(x) && x !== id) : [];
  if (!keys.range) return selected.includes(id) ? kept : [...kept, id];
  const from = anchor !== null && listed.has(anchor) ? hosts.indexOf(anchor) : hosts.indexOf(id);
  const to = hosts.indexOf(id);
  const range = hosts.slice(Math.min(from, to), Math.max(from, to) + 1).filter((x) => x !== id);
  const inRange = new Set(range);
  return [...kept.filter((x) => !inRange.has(x)), ...range, id];
}
