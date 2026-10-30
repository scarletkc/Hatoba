import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { RECOVERY_SESSION_TTL_MS, SESSION_TTL_MS } from "../src/config";
import { sha256Hex } from "../src/util";
import {
  AUTH_KEY,
  KDF_PARAMS,
  KDF_SALT,
  NEW_AUTH_KEY,
  NEW_RECOVERY_AUTH,
  RECOVERY_AUTH,
  call,
  changePassword,
  envelope,
  key32,
  login,
  loggedIn,
  passwordChange,
  pausedBeforeBatch,
  setupVault,
} from "./helpers";

interface SessionRow {
  token_hash: string;
  device_id: string;
  device_name: string;
  scope: string;
  created_at: number;
  last_seen: number;
  expires_at: number;
}

const sessionRows = async () =>
  (await env.DB.prepare("SELECT * FROM sessions ORDER BY created_at, device_id").all<SessionRow>()).results;

describe("POST /v1/login", () => {
  it("is 404 not_initialized before setup", async () => {
    const res = await call("/v1/login", {
      json: { auth_key: AUTH_KEY, device_id: "device-a", device_name: envelope() },
    });
    expect(res.status).toBe(404);
    expect(res.body.error).toBe("not_initialized");
  });

  it("rejects a wrong auth_key with 401 and creates no session", async () => {
    await setupVault();
    const res = await call("/v1/login", {
      json: { auth_key: key32(99), device_id: "device-a", device_name: envelope() },
    });
    expect(res.status).toBe(401);
    expect(res.body.error).toBe("invalid_credentials");
    expect(await sessionRows()).toHaveLength(0);
  });

  it("does not accept the recovery secret as a login key", async () => {
    await setupVault();
    const res = await call("/v1/login", {
      json: { auth_key: RECOVERY_AUTH, device_id: "device-a", device_name: envelope() },
    });
    expect(res.status).toBe(401);
  });

  it("issues a 30-day session token and stores only its SHA-256", async () => {
    await setupVault();
    const before = Date.now();
    const { session_token, expires_at } = await login("device-a", AUTH_KEY, "opaque-name-envelope");

    expect(session_token).toMatch(/^[A-Za-z0-9_-]{43}$/); // 32 bytes, base64url, no padding
    expect(expires_at).toBeGreaterThanOrEqual(before + SESSION_TTL_MS);
    expect(expires_at).toBeLessThanOrEqual(Date.now() + SESSION_TTL_MS);

    const rows = await sessionRows();
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      token_hash: await sha256Hex(session_token),
      device_id: "device-a",
      device_name: "opaque-name-envelope",
      scope: "full",
      expires_at,
    });
    expect(JSON.stringify(rows)).not.toContain(session_token);
  });

  it("keeps at most one session per device_id", async () => {
    await setupVault();
    const first = await login("device-a");
    const second = await login("device-a");
    const other = await login("device-b");

    expect(first.session_token).not.toBe(second.session_token);
    expect((await call("/v1/vault", { token: first.session_token })).status).toBe(401);
    expect((await call("/v1/vault", { token: second.session_token })).status).toBe(200);
    expect((await call("/v1/vault", { token: other.session_token })).status).toBe(200);
    expect(await sessionRows()).toHaveLength(2);
  });

  it("purges expired sessions of other devices on login", async () => {
    await setupVault();
    await login("stale-device");
    await env.DB.prepare("UPDATE sessions SET expires_at = ?")
      .bind(Date.now() - 1000)
      .run();
    await login("device-a");
    expect((await sessionRows()).map((row) => row.device_id)).toEqual(["device-a"]);
  });

  it.each([
    ["missing auth_key", { auth_key: undefined }],
    ["auth_key wrong length", { auth_key: btoa("nope") }],
    ["missing device_id", { device_id: undefined }],
    ["device_id with illegal characters", { device_id: "a b/c" }],
    ["device_id too long", { device_id: "x".repeat(65) }],
    ["missing device_name", { device_name: undefined }],
    ["empty device_name", { device_name: "" }],
    ["device_name over 4 KB", { device_name: "x".repeat(4097) }],
  ])("validates input: %s", async (_name, overrides) => {
    await setupVault();
    const res = await call("/v1/login", {
      json: { auth_key: AUTH_KEY, device_id: "device-a", device_name: envelope(), ...overrides },
    });
    expect(res.status).toBe(400);
  });

  it("accepts a 4 KB device_name and rejects non-JSON bodies with 415", async () => {
    await setupVault();
    const ok = await call("/v1/login", {
      json: { auth_key: AUTH_KEY, device_id: "device-a", device_name: "n".repeat(4096) },
    });
    expect(ok.status).toBe(200);
    const text = await call("/v1/login", { body: "auth_key=x", headers: { "Content-Type": "text/plain" } });
    expect(text.status).toBe(415);
  });

  it("creates no session when the password changes after the credential check", async () => {
    await setupVault();
    const owner = await login("device-a");
    const res = await call("/v1/login", {
      json: { auth_key: AUTH_KEY, device_id: "device-b", device_name: envelope() },
      env: {
        DB: pausedBeforeBatch(async () => {
          expect((await changePassword(owner.session_token, passwordChange())).status).toBe(200);
        }),
      },
    });
    expect(res.status).toBe(401);
    expect(res.body.error).toBe("invalid_credentials");
    expect((await sessionRows()).map((row) => row.device_id)).toEqual(["device-a"]);
  });

  it("keeps the device's newer session when its login with the old password fails", async () => {
    await setupVault();
    const owner = await login("device-a");
    let newer = "";
    const res = await call("/v1/login", {
      json: { auth_key: AUTH_KEY, device_id: "device-b", device_name: envelope() },
      env: {
        DB: pausedBeforeBatch(async () => {
          await changePassword(owner.session_token, passwordChange());
          newer = (await login("device-b", NEW_AUTH_KEY)).session_token;
        }),
      },
    });
    expect(res.status).toBe(401);
    expect((await call("/v1/vault", { token: newer })).status).toBe(200);
  });
});

describe("session handling", () => {
  const protectedRoutes: Array<[string, string]> = [
    ["GET", "/v1/vault"],
    ["GET", "/v1/items"],
    ["POST", "/v1/items"],
    ["PUT", "/v1/vault/password"],
    ["GET", "/v1/devices"],
    ["DELETE", "/v1/devices/device-a"],
  ];

  it.each(protectedRoutes)("%s %s requires a session (401)", async (method, path) => {
    await setupVault();
    const headerSets: Array<Record<string, string>> = [
      {},
      { Authorization: "Bearer not-a-real-token" },
      { Authorization: "Token abc" },
    ];
    for (const headers of headerSets) {
      const res = await call(path, { method, headers, json: method === "GET" || method === "DELETE" ? undefined : {} });
      expect(res.status, `${method} ${path} ${JSON.stringify(headers)}`).toBe(401);
      expect(res.headers.get("WWW-Authenticate")).toBe("Bearer");
    }
  });

  it("rejects an expired session with 401", async () => {
    const token = await loggedIn();
    await env.DB.prepare("UPDATE sessions SET expires_at = ?")
      .bind(Date.now() - 1)
      .run();
    const res = await call("/v1/vault", { token });
    expect(res.status).toBe(401);
    expect(res.body.error).toBe("invalid_session");
  });

  it("slides the expiry forward when a session is used", async () => {
    const token = await loggedIn();
    const hash = await sha256Hex(token);
    const staleSeen = Date.now() - 10 * 60_000;
    await env.DB.prepare("UPDATE sessions SET last_seen = ?, expires_at = ? WHERE token_hash = ?")
      .bind(staleSeen, Date.now() + 60_000, hash)
      .run();

    expect((await call("/v1/vault", { token })).status).toBe(200);
    const row = await env.DB.prepare("SELECT last_seen, expires_at FROM sessions").first<SessionRow>();
    expect(row!.last_seen).toBeGreaterThan(staleSeen);
    expect(row!.expires_at).toBeGreaterThanOrEqual(Date.now() - 5000 + SESSION_TTL_MS);
  });

  it("throttles renewal writes to once per minute", async () => {
    const token = await loggedIn();
    const recentSeen = Date.now() - 5_000;
    const originalExpiry = Date.now() + 1_000_000;
    await env.DB.prepare("UPDATE sessions SET last_seen = ?, expires_at = ?").bind(recentSeen, originalExpiry).run();

    expect((await call("/v1/vault", { token })).status).toBe(200);
    const row = await env.DB.prepare("SELECT last_seen, expires_at FROM sessions").first<SessionRow>();
    expect(row).toMatchObject({ last_seen: recentSeen, expires_at: originalExpiry });
  });
});

describe("POST /v1/recover", () => {
  const recover = (recoveryAuth = RECOVERY_AUTH, deviceId = "device-r") =>
    call("/v1/recover", {
      json: { recovery_auth: recoveryAuth, device_id: deviceId, device_name: envelope("r") },
    });

  it("is 404 before setup and 401 for a wrong recovery_auth", async () => {
    expect((await recover()).status).toBe(404);
    await setupVault();
    const res = await recover(key32(77));
    expect(res.status).toBe(401);
    expect(res.body.error).toBe("invalid_credentials");
    expect(await sessionRows()).toHaveLength(0);
  });

  it("does not accept the login auth_key as recovery proof", async () => {
    await setupVault();
    expect((await recover(AUTH_KEY)).status).toBe(401);
  });

  it("creates no session when the recovery code is rotated after the check", async () => {
    await setupVault();
    const owner = await login("device-a");
    const rotate = passwordChange({ recovery_vault_key: envelope("rvk2"), recovery_auth: NEW_RECOVERY_AUTH });
    const res = await call("/v1/recover", {
      json: { recovery_auth: RECOVERY_AUTH, device_id: "device-r", device_name: envelope("r") },
      env: {
        DB: pausedBeforeBatch(async () => {
          expect((await changePassword(owner.session_token, rotate)).status).toBe(200);
        }),
      },
    });
    expect(res.status).toBe(401);
    expect((await sessionRows()).map((row) => row.scope)).toEqual(["full"]);
  });

  it("still issues the session when the password changes but the recovery code does not", async () => {
    await setupVault();
    const owner = await login("device-a");
    const res = await call("/v1/recover", {
      json: { recovery_auth: RECOVERY_AUTH, device_id: "device-r", device_name: envelope("r") },
      env: {
        DB: pausedBeforeBatch(async () => {
          await changePassword(owner.session_token, passwordChange());
        }),
      },
    });
    expect(res.status).toBe(200);
    expect((await sessionRows()).map((row) => row.scope).sort()).toEqual(["full", "recovery"]);
  });

  it("returns the recovery key material and a 15 minute restricted session", async () => {
    await setupVault();
    const before = Date.now();
    const res = await recover();
    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({
      recovery_vault_key: envelope("rvk"),
      kdf_salt: KDF_SALT,
      kdf_params: KDF_PARAMS,
    });
    expect(res.body.session_token).toMatch(/^[A-Za-z0-9_-]{43}$/);
    expect(res.body.expires_at).toBeGreaterThanOrEqual(before + RECOVERY_SESSION_TTL_MS);
    expect(res.body.expires_at).toBeLessThanOrEqual(Date.now() + RECOVERY_SESSION_TTL_MS);

    const [row] = await sessionRows();
    expect(row).toMatchObject({ scope: "recovery", device_id: "device-r" });
  });

  it("issues a session that cannot call anything except PUT /v1/vault/password", async () => {
    await setupVault();
    const token = (await recover()).body.session_token as string;

    const attempts: Array<[string, string, unknown?]> = [
      ["GET", "/v1/vault"],
      ["GET", "/v1/items"],
      ["POST", "/v1/items", { changes: [] }],
      ["GET", "/v1/devices"],
      ["DELETE", "/v1/devices/device-r"],
    ];
    for (const [method, path, json] of attempts) {
      const res = await call(path, { method, token, json });
      expect(res.status, `${method} ${path}`).toBe(403);
      expect(res.body.error).toBe("insufficient_scope");
    }
  });

  it("does not slide the expiry of a recovery session", async () => {
    await setupVault();
    const token = (await recover()).body.session_token as string;
    const staleSeen = Date.now() - 10 * 60_000;
    const expiry = Date.now() + 60_000;
    await env.DB.prepare("UPDATE sessions SET last_seen = ?, expires_at = ?").bind(staleSeen, expiry).run();
    await call("/v1/vault", { token }); // 403, but must not extend the session
    const row = await env.DB.prepare("SELECT last_seen, expires_at FROM sessions").first<SessionRow>();
    expect(row).toMatchObject({ last_seen: staleSeen, expires_at: expiry });
  });

  it("rejects an expired recovery session", async () => {
    await setupVault();
    const token = (await recover()).body.session_token as string;
    await env.DB.prepare("UPDATE sessions SET expires_at = ?")
      .bind(Date.now() - 1)
      .run();
    const res = await call("/v1/vault/password", {
      method: "PUT",
      token,
      json: passwordChange(),
    });
    expect(res.status).toBe(401);
  });

  it("completes the recovery flow: new password, all sessions revoked, relogin required", async () => {
    await setupVault();
    const existing = await login("device-a");
    const token = (await recover()).body.session_token as string;

    const res = await call("/v1/vault/password", {
      method: "PUT",
      token,
      json: passwordChange({ recovery_vault_key: envelope("rvk2"), recovery_auth: NEW_RECOVERY_AUTH }),
    });
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ ok: true, relogin_required: true });

    // Every session is gone: the recovery session itself and the pre-existing device session.
    expect(await sessionRows()).toHaveLength(0);
    expect((await call("/v1/vault", { token: existing.session_token })).status).toBe(401);
    expect((await call("/v1/vault/password", { method: "PUT", token, json: passwordChange() })).status).toBe(401);

    // Old credentials are dead, new ones work.
    const oldLogin = await call("/v1/login", {
      json: { auth_key: AUTH_KEY, device_id: "device-a", device_name: envelope() },
    });
    expect(oldLogin.status).toBe(401);
    expect((await login("device-a", NEW_AUTH_KEY)).session_token).toBeTruthy();
    expect((await recover(RECOVERY_AUTH)).status).toBe(401);
    expect((await recover(NEW_RECOVERY_AUTH)).status).toBe(200);
  });
});
