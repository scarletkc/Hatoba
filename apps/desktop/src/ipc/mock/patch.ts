/**
 * `value` with the fields `patch` sets, as Rust applies a settings patch: a field left out keeps its
 * value, and so does one set to undefined, which the IPC drops, or to null, unless it is one of the
 * `clearable` fields that null clears.
 */
export function applyPatch<T extends object>(
  value: T,
  patch: { [K in keyof T]?: T[K] | null },
  clearable: readonly (keyof T)[] = [],
): T {
  const next = { ...value };
  for (const key of Object.keys(patch) as (keyof T)[]) {
    const field = patch[key];
    if (field === undefined || (field === null && !clearable.includes(key))) continue;
    next[key] = field as T[keyof T];
  }
  return next;
}
