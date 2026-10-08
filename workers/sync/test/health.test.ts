import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { VERSION } from "../src/config";
import { KDF_PARAMS, KDF_SALT, call, setupVault } from "./helpers";

describe("GET /v1/health", () => {
  it("reports an uninitialised vault, then an initialised one", async () => {
    const before = await call("/v1/health");
    expect(before.status).toBe(200);
    expect(before.body).toEqual({ service: "hatoba-sync", version: VERSION, api: 1, initialized: false });

    await setupVault();
    const after = await call("/v1/health");
    expect(after.body.initialized).toBe(true);
  });

  it("returns 503 when D1 is unreachable or not migrated", async () => {
    const brokenDb = {
      prepare() {
        throw new Error("no such table: meta");
      },
    } as unknown as D1Database;
    const res = await call("/v1/health", { env: { DB: brokenDb } });
    expect(res.status).toBe(503);
    expect(res.body.error).toBe("database_unavailable");
  });
});

describe("GET /v1/prelogin", () => {
  it("is 404 not_initialized before setup", async () => {
    const res = await call("/v1/prelogin");
    expect(res.status).toBe(404);
    expect(res.body).toMatchObject({ error: "not_initialized" });
  });

  it("returns the salt and the KDF params string exactly as stored", async () => {
    await setupVault();
    const res = await call("/v1/prelogin");
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ kdf_salt: KDF_SALT, kdf_params: KDF_PARAMS });
    expect(typeof res.body.kdf_params).toBe("string");
  });
});

describe("transport behaviour", () => {
  it("answers unknown routes with a JSON 404", async () => {
    const res = await call("/v1/nope");
    expect(res.status).toBe(404);
    expect(res.body).toEqual({ error: "not_found" });
  });

  it("never emits CORS headers, not even for preflight requests", async () => {
    const get = await call("/v1/health", { headers: { Origin: "https://evil.example" } });
    const preflight = await call("/v1/login", {
      method: "OPTIONS",
      headers: {
        Origin: "https://evil.example",
        "Access-Control-Request-Method": "POST",
      },
    });
    for (const res of [get, preflight]) {
      expect([...res.headers.keys()].filter((name) => name.startsWith("access-control-"))).toEqual([]);
    }
    expect(preflight.status).toBe(404);
  });

  it("marks every response as non-cacheable", async () => {
    const res = await call("/v1/health");
    expect(res.headers.get("Cache-Control")).toBe("no-store");
    expect(res.headers.get("X-Content-Type-Options")).toBe("nosniff");
  });

  it("does not leak internals on unexpected failures", async () => {
    const failingDb = {
      prepare() {
        throw new Error("secret internal detail");
      },
    } as unknown as D1Database;
    const res = await call("/v1/prelogin", { env: { DB: failingDb } });
    expect(res.status).toBe(500);
    expect(res.body).toEqual({ error: "internal_error" });
    expect(env.DB).toBeDefined();
  });
});
