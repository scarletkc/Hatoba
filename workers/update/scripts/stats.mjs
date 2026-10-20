// Prints the update checks the Worker counted (README.md): each of the last 30 days, and the last
// 7 days by version and by platform. Reads the deployed database after `npx wrangler login`, or
// the local one of `npm run dev` with --local.
// Usage: npm run stats [-- --local]
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const WORKER = fileURLToPath(new URL("..", import.meta.url));
const WRANGLER = fileURLToPath(new URL("../node_modules/wrangler/bin/wrangler.js", import.meta.url));

const TABLES = [
  [
    "Checks per day, last 30 days (UTC)",
    "SELECT day, SUM(count) AS checks FROM checks WHERE day >= date('now', '-29 days') GROUP BY day ORDER BY day DESC",
  ],
  [
    "By version, last 7 days",
    "SELECT version, SUM(count) AS checks FROM checks WHERE day >= date('now', '-6 days') GROUP BY version ORDER BY checks DESC",
  ],
  [
    "By platform, last 7 days",
    "SELECT os, arch, bundle, SUM(count) AS checks FROM checks WHERE day >= date('now', '-6 days') GROUP BY os, arch, bundle ORDER BY checks DESC",
  ],
];

const where = process.argv.includes("--local") ? "--local" : "--remote";
const sql = TABLES.map(([, query]) => query).join("; ");
const output = execFileSync(process.execPath, [WRANGLER, "d1", "execute", "DB", where, "--json", "--command", sql], {
  cwd: WORKER,
  encoding: "utf8",
  stdio: ["ignore", "pipe", "inherit"],
});
const results = JSON.parse(output);
TABLES.forEach(([title], i) => {
  console.log(`\n${title}`);
  console.table(results[i].results);
});
