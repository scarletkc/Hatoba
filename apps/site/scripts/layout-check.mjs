// Checks that the built site's pages don't move while they load: the web font swapping in, the
// download buttons picking the visitor's platform, images arriving. It loads each page in Chrome
// with the fonts held back, as on a slow network, for each platform the pages tell apart and at
// phone width, and fails when a page's layout shift (CLS) passes BUDGET. The Site workflow runs it
// after the build.
// Usage: node scripts/layout-check.mjs (after `pnpm build`, with Chrome installed)
import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "playwright-core";
import { DIST, serve } from "./serve.mjs";

/** A tenth of the 0.1 that web.dev counts as good: a shift that small isn't visible. */
const BUDGET = 0.01;

/** The longest page of the docs, which has the most text to move. */
function longestDoc() {
  const dir = join(DIST, "docs");
  const [slug] = readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort((a, b) => statSync(join(dir, b, "index.html")).size - statSync(join(dir, a, "index.html")).size);
  return `/docs/${slug}/`;
}

const PAGES = ["/", "/download/", "/docs/", longestDoc(), "/zh/", "/ja/download/", "/404.html"];
/** The desktop systems the layout detects (data-os), and a phone, which gets none. */
const VISITORS = [
  { name: "windows", platform: "Win32" },
  { name: "macos", platform: "MacIntel" },
  { name: "linux", platform: "Linux x86_64" },
  { name: "phone", device: { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, userAgent: "Mozilla/5.0 (Linux; Android 14) Mobile" } },
];

const { origin, close } = await serve({ delay: { match: /\.woff2$/, ms: 400 } });
const browser = await chromium.launch({ channel: "chrome" });
const failures = [];
try {
  for (const visitor of VISITORS) {
    for (const path of PAGES) {
      // A context per page, so no page finds the font in the cache of the one before.
      const context = await browser.newContext(visitor.device ?? { viewport: { width: 1440, height: 900 } });
      await context.addInitScript((platform) => {
        if (platform) {
          Object.defineProperty(Navigator.prototype, "platform", { get: () => platform });
          Object.defineProperty(Navigator.prototype, "userAgentData", { get: () => ({ platform }) });
        }
        // English pages would send a Chinese or Japanese browser to its language.
        localStorage.setItem("hatoba.locale", "en");
        window.__shift = 0;
        window.__moved = [];
        new PerformanceObserver((list) => {
          for (const entry of list.getEntries()) {
            if (entry.hadRecentInput) continue;
            window.__shift += entry.value;
            // What moved, to name in a failure.
            for (const { node } of entry.sources) {
              const element = node instanceof Element ? node : node?.parentElement;
              if (element) window.__moved.push(`${element.localName}${element.id ? `#${element.id}` : ""}${element.className && typeof element.className === "string" ? `.${element.className.split(" ")[0]}` : ""} "${(node.textContent ?? "").trim().slice(0, 40)}"`);
            }
          }
        }).observe({ type: "layout-shift", buffered: true });
      }, visitor.platform);
      const page = await context.newPage();
      const response = await page.goto(origin + path, { waitUntil: "networkidle" });
      if (!response?.ok()) throw new Error(`${path} answered ${response?.status() ?? "nothing"}.`);
      await page.waitForTimeout(500);
      const { shift, moved } = await page.evaluate(() => ({ shift: window.__shift, moved: window.__moved }));
      const line = `${path} (${visitor.name}): ${shift.toFixed(4)}`;
      if (shift > BUDGET) failures.push(`${line}, moving ${[...new Set(moved)].slice(0, 5).join(", ")}`);
      console.log(line);
      await context.close();
    }
  }
} finally {
  await browser.close();
  close();
}
if (failures.length > 0) {
  console.error(`\nThese pages move by more than ${BUDGET} while they load:\n${failures.join("\n")}`);
  process.exit(1);
}
