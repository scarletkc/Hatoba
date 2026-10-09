import { Icon } from "@/components/controls";
import { cx } from "@/lib/cx";
import s from "./OsIcon.module.css";

/** The OS ids the backend reports (HOST-11) and the names shown on hover. */
const OS_NAMES = new Map([
  ["ubuntu", "Ubuntu"],
  ["debian", "Debian"],
  ["raspbian", "Raspbian"],
  ["freebsd", "FreeBSD"],
  ["netbsd", "NetBSD"],
  ["windows", "Windows"],
]);

function Glyph({ os }: { os: string | null }) {
  if (os === "windows") return <Icon name="windows-logo" fill className={s.windows} />;
  if (os !== null && OS_NAMES.has(os)) return <span className={cx(s.logo, s[os])} />;
  return <Icon name="hard-drives" className={s.server} />;
}

/**
 * HOST-11: the logo of the host's OS, or a generic server when it is unknown, with a status
 * badge (a CSS color) in the bottom-right corner.
 */
export function OsIcon({ os, badge }: { os: string | null; badge: string }) {
  const name = os === null ? undefined : OS_NAMES.get(os);
  return (
    <span className={s.root} role={name ? "img" : undefined} aria-label={name} aria-hidden={name ? undefined : true} title={name}>
      <span className={s.glyph}>
        <Glyph os={os} />
      </span>
      <span className={s.badge} style={{ background: badge }} />
    </span>
  );
}
