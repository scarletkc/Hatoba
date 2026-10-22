// Checks the built site's demo in Chrome: the home page's button opens it, the assistant's input
// holds the suggested question, and sending it gets as far as the first approval card. The Site
// workflow runs it after the build, so that a release cannot publish a demo that doesn't start.
// Usage: node scripts/demo-check.mjs (after `pnpm build`, with Chrome installed)
import { existsSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "playwright-core";
import { DIST, serve } from "./serve.mjs";

const TIMEOUT = 30_000;

if (!existsSync(join(DIST, "demo", "index.html"))) {
  console.log("The build has no demo, so there is nothing to check.");
  process.exit(0);
}

const { origin, close } = await serve();

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
  close();
}
