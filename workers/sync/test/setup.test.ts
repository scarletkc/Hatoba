import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { sha256Hex } from "../src/util";
import { AUTH_KEY, KDF_PARAMS, RECOVERY_AUTH, SETUP_TOKEN, call, key32, setupPayload } from "./helpers";

// `token: null` sends no Authorization header at all.
const setup = (json: unknown, token: string | null = SETUP_TOKEN) =>
  call("/v1/setup", { json, ...(token === null ? {} : { token }) });

describe("POST /v1/setup", () => {
  it("is 503 when SETUP_TOKEN is not configured", async () => {
    const res = await call("/v1/setup", { token: "anything", json: setupPayload(), env: { SETUP_TOKEN: undefined } });
    expect(res.status).toBe(503);
    expect(res.body.error).toBe("setup_token_not_configured");
  });

  it("treats an empty SETUP_TOKEN secret as not configured", async () => {
    const res = await call("/v1/setup", { token: "", json: setupPayload(), env: { SETUP_TOKEN: "" } });
    expect(res.status).toBe(503);
  });

  it("rejects a missing, malformed or wrong setup token with 401", async () => {
    for (const token of [null, "wrong-token", `${SETUP_TOKEN}x`, SETUP_TOKEN.slice(0, -1)]) {
      const res = await setup(setupPayload(), token);
      expect(res.status, String(token)).toBe(401);
      expect(res.body.error).toBe("invalid_setup_token");
    }
    const basic = await call("/v1/setup", {
      json: setupPayload(),
      headers: { Authorization: `Basic ${SETUP_TOKEN}` },
    });
    expect(basic.status).toBe(401);
    expect((await call("/v1/health")).body.initialized).toBe(false);
  });

  it("initialises the vault with the correct token", async () => {
    const res = await setup(setupPayload());
    expect(res.status).toBe(201);
    expect(res.body).toEqual({ initialized: true });
    expect((await call("/v1/health")).body.initialized).toBe(true);
  });

  it("stores only SHA-256 digests of auth_key and recovery_auth", async () => {
    await setup(setupPayload());
    const row = await env.DB.prepare("SELECT * FROM meta WHERE id = 1").first<Record<string, unknown>>();
    expect(row).toMatchObject({
      schema_version: 1,
      kdf_params: KDF_PARAMS,
      seq: 0,
      auth_hash: await sha256Hex(Uint8Array.from(atob(AUTH_KEY), (ch) => ch.charCodeAt(0))),
      recovery_auth_hash: await sha256Hex(Uint8Array.from(atob(RECOVERY_AUTH), (ch) => ch.charCodeAt(0))),
    });
    const stored = JSON.stringify(row);
    expect(stored).not.toContain(AUTH_KEY);
    expect(stored).not.toContain(RECOVERY_AUTH);
  });

  it("returns 409 once initialised and keeps the original vault", async () => {
    expect((await setup(setupPayload())).status).toBe(201);
    const again = await setup(setupPayload({ auth_key: key32(9) }));
    expect(again.status).toBe(409);
    expect(again.body.error).toBe("already_initialized");
    const row = await env.DB.prepare("SELECT auth_hash FROM meta").first<{ auth_hash: string }>();
    expect(row?.auth_hash).not.toBe(await sha256Hex(Uint8Array.from(atob(key32(9)), (ch) => ch.charCodeAt(0))));
  });

  it("lets exactly one of two racing setups win", async () => {
    const statuses = (await Promise.all([setup(setupPayload()), setup(setupPayload({ auth_key: key32(9) }))])).map(
      (res) => res.status,
    );
    expect(statuses.sort()).toEqual([201, 409]);
  });

  it("rejects non-JSON bodies with 415", async () => {
    const res = await call("/v1/setup", {
      token: SETUP_TOKEN,
      body: "schema_version=1",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
    });
    expect(res.status).toBe(415);
    expect(res.body.error).toBe("unsupported_media_type");
    const noType = await call("/v1/setup", { token: SETUP_TOKEN, body: JSON.stringify(setupPayload()) });
    expect(noType.status).toBe(415);
  });

  it("accepts a Content-Type with a charset parameter", async () => {
    const res = await call("/v1/setup", {
      token: SETUP_TOKEN,
      body: JSON.stringify(setupPayload()),
      headers: { "Content-Type": "application/json; charset=utf-8" },
    });
    expect(res.status).toBe(201);
  });

  it("rejects malformed JSON and non-object bodies with 400", async () => {
    const bad = await call("/v1/setup", {
      token: SETUP_TOKEN,
      body: "{nope",
      headers: { "Content-Type": "application/json" },
    });
    expect(bad.status).toBe(400);
    expect(bad.body.error).toBe("invalid_json");
    expect((await setup([1, 2])).status).toBe(400);
    expect((await setup(null)).status).toBe(400);
  });

  it.each([
    ["schema_version missing", { schema_version: undefined }],
    ["schema_version zero", { schema_version: 0 }],
    ["schema_version float", { schema_version: 1.5 }],
    ["schema_version string", { schema_version: "1" }],
    ["kdf_salt too short", { kdf_salt: "abc" }],
    ["kdf_salt bad characters", { kdf_salt: "not base64 at all!!" }],
    ["kdf_params not a string", { kdf_params: { alg: "argon2id" } }],
    ["kdf_params not JSON", { kdf_params: "argon2id" }],
    ["kdf_params JSON array", { kdf_params: "[1]" }],
    ["kdf_params too large", { kdf_params: JSON.stringify({ pad: "x".repeat(2000) }) }],
    ["auth_key wrong length", { auth_key: btoa("short") }],
    ["auth_key 33 bytes", { auth_key: btoa("x".repeat(33)) }],
    ["auth_key not base64", { auth_key: "!!!!" }],
    ["auth_key url-safe alphabet", { auth_key: "-".repeat(43) + "=" }],
    ["auth_key unpadded", { auth_key: key32(1).replace(/=+$/, "") }],
    ["auth_key missing", { auth_key: undefined }],
    ["recovery_auth wrong length", { recovery_auth: btoa("short") }],
    ["recovery_auth missing", { recovery_auth: undefined }],
    ["protected_vault_key empty", { protected_vault_key: "" }],
    ["protected_vault_key too large", { protected_vault_key: "x".repeat(5000) }],
    ["recovery_vault_key not a string", { recovery_vault_key: 5 }],
  ])("validates input: %s", async (_name, overrides) => {
    const res = await setup(setupPayload(overrides));
    expect(res.status).toBe(400);
    expect(res.body.error).toBe("invalid_request");
    expect((await call("/v1/health")).body.initialized).toBe(false);
  });

  it("never echoes submitted secrets in error messages", async () => {
    const res = await setup(setupPayload({ auth_key: "SUPER-SECRET-VALUE" }));
    expect(res.status).toBe(400);
    expect(JSON.stringify(res.body)).not.toContain("SUPER-SECRET-VALUE");
  });

  it("rejects bodies over the size limit with 413", async () => {
    const res = await setup(setupPayload({ kdf_params: "x".repeat(100_000) }));
    expect(res.status).toBe(413);
    expect(res.body.error).toBe("payload_too_large");
  });
});
