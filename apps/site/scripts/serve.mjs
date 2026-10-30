// Serves the built site (dist) on a free local port as GitHub Pages does, for the checks that open
// it in Chrome. `delay` holds back responses whose path matches it, as a slow network would.
import { createReadStream, existsSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

export const DIST = fileURLToPath(new URL("../dist", import.meta.url));

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

/** Starts the server and returns its origin and a function that stops it. */
export async function serve({ delay } = {}) {
  const server = createServer((request, response) => {
    const pathname = decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname);
    let path = join(DIST, pathname);
    if (relative(DIST, path).startsWith("..")) return void response.writeHead(403).end();
    // A folder's index.html for the folder.
    if (existsSync(path) && statSync(path).isDirectory()) path = join(path, "index.html");
    if (!existsSync(path)) return void response.writeHead(404).end();
    const send = () => {
      response.writeHead(200, { "content-type": TYPES[extname(path)] ?? "application/octet-stream" });
      createReadStream(path).pipe(response);
    };
    if (delay && delay.match.test(pathname)) setTimeout(send, delay.ms);
    else send();
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(undefined)));
  const address = server.address();
  const origin = `http://127.0.0.1:${typeof address === "object" && address ? address.port : 0}`;
  return { origin, close: () => server.close() };
}
