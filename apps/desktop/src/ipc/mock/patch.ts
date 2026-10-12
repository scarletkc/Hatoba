/**
 * `value` with the fields `patch` sets, as Rust applies a settings patch: a field left out keeps its
 * value, and so does one set to undefined, which the IPC drops.
 */
export function applyPatch<T extends object>(value: T, patch: Partial<T> | undefined): T {
  const next = { ...value };
  for (const key of Object.keys(patch ?? {}) as (keyof T)[]) {
    const field = patch?.[key];
    if (field !== undefined) next[key] = field as T[keyof T];
  }
  return next;
}
