/** Settings → Appearance → AI Panel → Text Size (TERM-13), in px. Keep in step with `AI_TEXT_SIZE_*` in hatoba-core's model.rs. */
export const AI_TEXT_SIZE_MIN = 11;
export const AI_TEXT_SIZE_MAX = 24;
export const AI_TEXT_SIZE_DEFAULT = 13;

/** The `--text-scale` of the panel's conversation and input: the text size over the default, which is `--text-md`. */
export function aiTextScale(size: number | undefined): number {
  return (size ?? AI_TEXT_SIZE_DEFAULT) / AI_TEXT_SIZE_DEFAULT;
}
