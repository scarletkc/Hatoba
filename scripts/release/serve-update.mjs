// Serves a signed installer as an update from this machine, so that a test build of Hatoba can
// install it end to end. docs/development.md describes the steps.
// Usage: node scripts/release/serve-update.mjs --installer FILE --pubkey FILE [--port PORT]
//   --installer  the installer that `tauri build` wrote; its .sig must be next to it
//   --pubkey     the .pub file of the key that signed it
//   --port       the port on 127.0.0.1 to serve on (default 18765)
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { basename } from "node:path";
import { parseArgs } from "node:util";
import { verifyUpdaterSignature } from "./release.mjs";

const { values } = parseArgs({
  options: { installer: { type: "string" }, pubkey: { type: "string" }, port: { type: "string", default: "18765" } },
});
if (!values.installer || !values.pubkey) {
  console.error("Usage: node scripts/release/serve-update.mjs --installer FILE --pubkey FILE [--port PORT]");
  process.exit(1);
}
const installer = readFileSync(values.installer);
const signature = readFileSync(`${values.installer}.sig`, "utf8").trim();
const version = verifyUpdaterSignature(installer, signature, readFileSync(values.pubkey, "utf8"));
if (!version) throw new Error(`${values.installer}.sig names no version; sign it with \`tauri build\` or \`tauri signer sign --app-version\`.`);

const name = basename(values.installer);
const path = `/${encodeURIComponent(name)}`;
const origin = `http://127.0.0.1:${values.port}`;
const latest = JSON.stringify({
  version,
  notes: `## Local test update\n\nServed from \`${name}\` by \`scripts/release/serve-update.mjs\`.`,
  pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
  platforms: { "windows-x86_64": { signature, url: `${origin}${path}` } },
});

createServer((req, res) => {
  console.log(`${req.method} ${req.url}`);
  if (req.url === "/latest.json") {
    res.writeHead(200, { "Content-Type": "application/json" }).end(latest);
  } else if (req.url === path) {
    res.writeHead(200, { "Content-Type": "application/octet-stream", "Content-Length": installer.length }).end(installer);
  } else {
    res.writeHead(404).end();
  }
}).listen(Number(values.port), "127.0.0.1", () => console.log(`Serving Hatoba ${version} at ${origin}/latest.json`));
