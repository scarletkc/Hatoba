/**
 * Hatoba's update counter (spec §11, Privacy). An installed stable version asks
 * /update/<os>/<arch>/<bundle>/<version> for its update manifest. The Worker adds the check to that
 * day's count for the version and platform, then redirects to the latest stable release's
 * `latest.json` on GitHub, where the app reads the manifest and downloads the installer as before.
 * It keeps nothing else about the request: no address, no headers, no time of day.
 */

/** The update manifest of the latest stable release. */
export const MANIFEST = "https://github.com/scarletkc/Hatoba/releases/latest/download/latest.json";

export interface Env {
  DB: D1Database;
}

/** One update check, from the endpoint the updater fills in (tauri.conf.json). */
export interface Check {
  os: string;
  arch: string;
  bundle: string;
  version: string;
}

// The values the Tauri updater puts in the endpoint. A development build has no bundle type and
// asks with "unknown", which is not counted.
const OS = new Set(["windows", "linux", "darwin"]);
const ARCH = new Set(["x86_64", "aarch64", "i686", "armv7", "riscv64"]);
const BUNDLE = new Set(["nsis", "msi", "deb", "rpm", "appimage", "app"]);
/** Only stable versions ask here; a prerelease looks through the GitHub releases instead. */
const VERSION = /^\d{1,4}\.\d{1,4}\.\d{1,4}$/;

/** The check that `pathname` names, or `null` when it is not one the updater asks. */
export function parseCheck(pathname: string): Check | null {
  const [empty, update, os, arch, bundle, version, ...rest] = pathname.split("/");
  if (empty !== "" || update !== "update" || rest.length > 0) return null;
  if (!os || !arch || !bundle || !version) return null;
  if (!OS.has(os) || !ARCH.has(arch) || !BUNDLE.has(bundle) || !VERSION.test(version)) return null;
  return { os, arch, bundle, version };
}

/** Adds one to the count of `check` on `day` (YYYY-MM-DD, UTC). */
export async function count(db: D1Database, check: Check, day: string): Promise<void> {
  await db
    .prepare(
      `INSERT INTO checks (day, version, os, arch, bundle, count) VALUES (?1, ?2, ?3, ?4, ?5, 1)
       ON CONFLICT (day, version, os, arch, bundle) DO UPDATE SET count = count + 1`,
    )
    .bind(day, check.version, check.os, check.arch, check.bundle)
    .run();
}

export default {
  async fetch(request, env, ctx): Promise<Response> {
    const check = request.method === "GET" ? parseCheck(new URL(request.url).pathname) : null;
    if (check) {
      const day = new Date().toISOString().slice(0, 10);
      // The redirect does not wait for the count, and a failed count does not fail it.
      ctx.waitUntil(count(env.DB, check, day).catch((err: unknown) => console.error("count failed:", err)));
    }
    return new Response(null, {
      status: 302,
      headers: { Location: MANIFEST, "Cache-Control": "no-store" },
    });
  },
} satisfies ExportedHandler<Env>;
