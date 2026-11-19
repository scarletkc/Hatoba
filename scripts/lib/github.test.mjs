import assert from "node:assert/strict";
import { createServer } from "node:http";
import { test } from "node:test";
import { githubClient } from "./github.mjs";

test("githubClient sends auth/headers, handles 404/204, paginates, and reports HTTP errors", async () => {
  const seen = [];
  const server = createServer((req, res) => {
    seen.push({ method: req.method, url: req.url, headers: req.headers });
    if (req.url === "/missing") {
      res.writeHead(404, { "content-type": "application/json" });
      res.end(JSON.stringify({ message: "Not Found" }));
      return;
    }
    if (req.url === "/empty") {
      res.writeHead(204).end();
      return;
    }
    if (req.url === "/boom") {
      res.writeHead(500, { "content-type": "text/plain" });
      res.end("server error");
      return;
    }
    if (req.url.startsWith("/items?")) {
      const page = Number(new URL(req.url, "http://localhost").searchParams.get("page"));
      const batch = page === 1 ? Array.from({ length: 100 }, (_, i) => i + 1) : [101];
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify(batch));
      return;
    }
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ ok: true }));
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    const client = githubClient("tok", {
      userAgent: "hatoba-test",
      baseUrl: `http://127.0.0.1:${server.address().port}`,
    });
    assert.deepEqual(await client.request("POST", "/ping", { hello: "world" }), { ok: true });
    assert.equal(seen[0].headers.authorization, "Bearer tok");
    assert.equal(seen[0].headers["user-agent"], "hatoba-test");
    assert.equal(seen[0].headers["content-type"], "application/json");
    assert.equal(await client.request("GET", "/missing", undefined, { missingOk: true }), null);
    assert.equal(await client.request("DELETE", "/empty"), null);
    await assert.rejects(client.request("GET", "/boom"), /GET \/boom: HTTP 500 server error/);
    const all = await client.all("/items");
    assert.equal(all.length, 101);
  } finally {
    server.close();
  }
});
