import { join } from "node:path";
import type { APIRoute } from "astro";
import sharp from "sharp";
import { localeStaticPaths, type Locale } from "../../i18n";
import { repoRoot } from "../../lib/docs";
import { appIconPng } from "../../lib/icons";

export function getStaticPaths() {
  return localeStaticPaths();
}

const W = 1200;
const H = 630;
const SHOT = { x: 120, y: 104, w: 960, h: 600, radius: 14 };
/** The top of the sea's wave, and the period and height of the wave, as in app-icon.svg. */
const SURFACE = 468;
const PERIOD = 448;
const AMPLITUDE = 36;

const FOLDER: Record<Locale, string> = { en: "docs/media", zh: "apps/site/src/assets/screenshots/zh-CN", ja: "apps/site/src/assets/screenshots/ja" };

function wave(close: boolean): string {
  const start = -PERIOD;
  let d = `M${start} ${SURFACE}`;
  for (let x = start; x < W; x += PERIOD) {
    const half = PERIOD / 2;
    d += `C${x + 40.5} ${SURFACE} ${x + 75.5} ${SURFACE + 8.8} ${x + 112} ${SURFACE + 18}`;
    d += `S${x + 183.5} ${SURFACE + AMPLITUDE} ${x + half} ${SURFACE + AMPLITUDE}`;
    d += `S${x + 299.5} ${SURFACE + 27.2} ${x + 336} ${SURFACE + 18}`;
    d += `S${x + 407.5} ${SURFACE} ${x + PERIOD} ${SURFACE}`;
  }
  return close ? `${d}V${H}H${start}Z` : d;
}

const sky = `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}">
  <defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#0d1735"/><stop offset="1" stop-color="#24408a"/></linearGradient>
  <filter id="blur" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="24"/></filter></defs>
  <rect width="${W}" height="${H}" fill="url(#sky)"/>
  <rect x="${SHOT.x}" y="${SHOT.y + 24}" width="${SHOT.w}" height="${SHOT.h}" rx="${SHOT.radius}" fill="#050c28" opacity=".6" filter="url(#blur)"/>
</svg>`;

const corners = `<svg xmlns="http://www.w3.org/2000/svg" width="${SHOT.w}" height="${SHOT.h}"><rect width="${SHOT.w}" height="${SHOT.h}" rx="${SHOT.radius}" fill="#fff"/></svg>`;

const sea = `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${W}" height="${H}">
  <defs>
    <linearGradient id="water" x1="0" y1="${SURFACE}" x2="0" y2="${H}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#22a6e8" stop-opacity=".92"/><stop offset=".3" stop-color="#1a84d2" stop-opacity=".97"/><stop offset="1" stop-color="#0d4590"/>
    </linearGradient>
    <linearGradient id="fade" x1="0" y1="${SURFACE}" x2="0" y2="${H}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fff" stop-opacity=".22"/><stop offset="1" stop-color="#fff" stop-opacity=".03"/>
    </linearGradient>
    <g id="r" stroke="#fff" stroke-width="5"><circle r="56" fill="#000"/><circle r="40.3" fill="none"/><circle r="24.6" fill="none"/><circle r="9" fill="none"/></g>
    <pattern id="rings" width="112" height="56" patternUnits="userSpaceOnUse" patternTransform="translate(0 ${SURFACE + 4}) scale(.5)">
      <use xlink:href="#r" x="56" y="-28"/><use xlink:href="#r" x="0" y="0"/><use xlink:href="#r" x="112" y="0"/><use xlink:href="#r" x="56" y="28"/>
      <use xlink:href="#r" x="0" y="56"/><use xlink:href="#r" x="112" y="56"/><use xlink:href="#r" x="56" y="84"/>
    </pattern>
    <mask id="lines" maskUnits="userSpaceOnUse" x="0" y="0" width="${W}" height="${H}"><rect width="${W}" height="${H}" fill="url(#rings)"/></mask>
    <clipPath id="below"><path d="${wave(true)}"/></clipPath>
  </defs>
  <g clip-path="url(#below)">
    <rect width="${W}" height="${H}" fill="url(#water)"/>
    <rect width="${W}" height="${H}" fill="url(#fade)" mask="url(#lines)"/>
  </g>
  <path d="${wave(false)}" fill="none" stroke="#fff" stroke-opacity=".5" stroke-width="1.5"/>
</svg>`;

/** The image that links to the site show: the app standing in the sea of its icon, as on the home page. */
export const GET: APIRoute = async ({ props }) => {
  const root = repoRoot();
  const screenshot = await sharp(join(root, FOLDER[props.locale as Locale], "ai-assistant.png"))
    .resize(SHOT.w, SHOT.h)
    .composite([{ input: Buffer.from(corners), blend: "dest-in" }])
    .png()
    .toBuffer();
  const icon = Buffer.from(await appIconPng(64));
  const image = await sharp(Buffer.from(sky))
    .composite([
      { input: screenshot, left: SHOT.x, top: SHOT.y },
      { input: Buffer.from(sea), left: 0, top: 0 },
      { input: icon, left: (W - 64) / 2, top: 20 },
    ])
    .png()
    .toBuffer();
  return new Response(new Uint8Array(image), { headers: { "content-type": "image/png" } });
};
