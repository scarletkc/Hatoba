type Duration = "--motion-fade" | "--motion-fast" | "--motion-base" | "--motion-slow";

/** A motion token (tokens.css) as it stands: the move and resize durations are zero with reduced motion. */
export function motion(duration: Duration, easing: "--ease-out" | "--ease-in" | "--ease-in-out" = "--ease-out") {
  const style = getComputedStyle(document.documentElement);
  const value = style.getPropertyValue(duration).trim();
  const ms = value.endsWith("ms") ? parseFloat(value) : parseFloat(value) * 1000;
  return { duration: Number.isFinite(ms) ? ms : 0, easing: style.getPropertyValue(easing).trim() || "ease-out" };
}
