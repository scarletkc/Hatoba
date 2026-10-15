// Checks the built site's demo in Chrome: the home page's button opens it, the assistant's input
// holds the suggested question, and sending it gets as far as the first approval card. The Site
// workflow runs it after the build, so that a release cannot publish a demo that doesn't start.
// Usage: node scripts/demo-check.mjs (after `pnpm build`, with Chrome installed)
import { createReadStream, existsSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const DIST = fileURLToPath(new URL("../dist", import.meta.url));
const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".css": "text/css",
  ".json": "application/json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".webp": "image/webp",
  ".avif": "image/avif",
  ".woff2": "font/woff2",
  ".woff": "font/woff",
  ".ttf": "font/ttf",
};
const TIMEOUT = 30_000;

if (!existsSync(join(DIST, "demo", "index.html"))) {
  console.log("The build has no demo, so there is nothing to check.");
  process.exit(0);
}

/** dist as GitHub Pages serves it: a folder's index.html for the folder. */
const server = createServer((request, response) => {
  let path = join(DIST, decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname));
  if (relative(DIST, path).startsWith("..")) return void response.writeHead(403).end();
  if (existsSync(path) && statSync(path).isDirectory()) path = join(path, "index.html");
  if (!existsSync(path)) return void response.writeHead(404).end();
  response.writeHead(200, { "content-type": TYPES[extname(path)] ?? "application/octet-stream" });
  createReadStream(path).pipe(response);
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(undefined)));
const address = server.address();
const origin = `http://127.0.0.1:${typeof address === "object" && address ? address.port : 0}`;

const browser = await chromium.launch({ channel: "chrome" });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, locale: "en-US" });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${origin}/`);
  await page.locator("[data-demo-open]").click({ timeout: TIMEOUT });
  const frame = await (await page.waitForSelector(".window iframe")).contentFrame();
  if (!frame) throw new Error("The demo's frame has no document.");
  const input = "textarea:not(.xterm-helper-textarea)";
  await frame.waitForFunction((selector) => document.querySelector(selector)?.value.includes("502"), input, { timeout: TIMEOUT });
  await frame.locator(input).press("Enter");
  await frame.getByRole("button", { name: "Run", exact: true }).first().waitFor({ timeout: TIMEOUT });
  if (errors.length > 0) throw new Error(`The page threw:\n${errors.join("\n")}`);
  console.log("The demo opened, suggested its question, and reached the approval card.");
} finally {
  await browser.close();
  server.close();
}
