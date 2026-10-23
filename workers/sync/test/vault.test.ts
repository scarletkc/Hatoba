import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { sha256Hex } from "../src/util";
import {
  AUTH_KEY,
  KDF_PARAMS,
  KDF_SALT,
  NEW_AUTH_KEY,
  NEW_RECOVERY_AUTH,
  RECOVERY_AUTH,
  call,
  change,
  envelope,
  login,
  passwordChange,
  push,
  setupVault,
} from "./helpers";

const changePassword = (token: string, json: unknown) => call("/v1/vault/password", { method: "PUT", token, json });

describe("GET /v1/vault", () => {
  it("returns the key material, KDF info and current seq", async () => {
    await setupVault();
    const { session_token } = await login();
    await push(session_token, [change("item-1", 0), change("item-2", 0)]);

    const res = await call("/v1/vault", { token: session_token });
    expect(res.status).toBe(200);
    expect(res.body).toEqual({
      schema_version: 1,
      kdf_salt: KDF_SALT,
      kdf_params: KDF_PARAMS,
      protected_vault_key: envelope("pvk"),
      recovery_vault_key: envelope("rvk"),
      seq: 2,
    });
    expect(JSON.stringify(res.body)).not.toContain("auth_hash");
  });
});

describe("PUT /v1/vault/password", () => {
  it("updates meta and revokes every other session but keeps the caller's", async () => {
    await setupVault();
    const caller = await login("device-a");
    const other1 = await login("device-b");
    const other2 = await login("device-c");

    const res = await changePassword(caller.session_token, passwordChange());
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ ok: true, relogin_required: false });

    expect((await call("/v1/vault", { token: other1.session_token })).status).toBe(401);
    expect((await call("/v1/vault", { token: other2.session_token })).status).toBe(401);
    const vault = await call("/v1/vault", { token: caller.session_token });
    expect(vault.status).toBe(200);
    expect(vault.body).toMatchObject({
      kdf_salt: "bmV3c2FsdG5ld3NhbHRuZXc=",
      protected_vault_key: envelope("pvk2"),
      // not provided, so the recovery material is untouched
      recovery_vault_key: envelope("rvk"),
    });
    expect(JSON.parse(vault.body.kdf_params)).toMatchObject({ rev: 2 });

    const remaining = await env.DB.prepare("SELECT device_id FROM sessions").all<{ device_id: string }>();
    expect(remaining.results.map((r) => r.device_id)).toEqual(["device-a"]);
  });

  it("switches the login credential and serves the new salt from prelogin", async () => {
    await setupVault();
    const caller = await login();
    await changePassword(caller.session_token, passwordChange());

    const oldKey = await call("/v1/login", {
      json: { auth_key: AUTH_KEY, device_id: "device-z", device_name: envelope() },
    });
    expect(oldKey.status).toBe(401);
    expect((await login("device-z", NEW_AUTH_KEY)).session_token).toBeTruthy();

    const prelogin = await call("/v1/prelogin");
    expect(prelogin.body.kdf_salt).toBe("bmV3c2FsdG5ld3NhbHRuZXc=");
    expect(JSON.parse(prelogin.body.kdf_params)).toMatchObject({ rev: 2 });

    const row = await env.DB.prepare("SELECT auth_hash, recovery_auth_hash FROM meta").first<Record<string, string>>();
    expect(row!.auth_hash).toBe(await sha256Hex(Uint8Array.from(atob(NEW_AUTH_KEY), (c) => c.charCodeAt(0))));
    // recovery secret not rotated
    expect(
      (await call("/v1/recover", { json: { recovery_auth: RECOVERY_AUTH, device_id: "d", device_name: envelope() } }))
        .status,
    ).toBe(200);
  });

  it("rotates the recovery material when recovery_vault_key and recovery_auth are sent", async () => {
    await setupVault();
    const caller = await login();
    const res = await changePassword(
      caller.session_token,
      passwordChange({ recovery_vault_key: envelope("rvk2"), recovery_auth: NEW_RECOVERY_AUTH }),
    );
    expect(res.status).toBe(200);

    expect((await call("/v1/vault", { token: caller.session_token })).body.recovery_vault_key).toBe(envelope("rvk2"));
    const attempt = (recoveryAuth: string) =>
      call("/v1/recover", { json: { recovery_auth: recoveryAuth, device_id: "d", device_name: envelope() } });
    expect((await attempt(RECOVERY_AUTH)).status).toBe(401);
    expect((await attempt(NEW_RECOVERY_AUTH)).status).toBe(200);
  });

  it("also revokes outstanding recovery sessions", async () => {
    await setupVault();
    const caller = await login();
    const recovery = (
      await call("/v1/recover", { json: { recovery_auth: RECOVERY_AUTH, device_id: "dr", device_name: envelope() } })
    ).body.session_token as string;
    await changePassword(caller.session_token, passwordChange());
    expect((await changePassword(recovery, passwordChange())).status).toBe(401);
  });

  it("leaves items untouched", async () => {
    await setupVault();
    const caller = await login();
    await push(caller.session_token, [change("item-1", 0, "keep")]);
    await changePassword(caller.session_token, passwordChange());
    const pulled = await call("/v1/items", { token: caller.session_token });
    expect(pulled.body.items[0]).toMatchObject({ id: "item-1", envelope: envelope("keep") });
  });

  it.each([
    ["recovery_vault_key without recovery_auth", { recovery_vault_key: envelope("x") }],
    ["recovery_auth without recovery_vault_key", { recovery_auth: NEW_RECOVERY_AUTH }],
    ["recovery_auth of the wrong length", { recovery_vault_key: envelope("x"), recovery_auth: btoa("short") }],
    ["auth_key of the wrong length", { auth_key: btoa("short") }],
    ["kdf_salt missing", { kdf_salt: undefined }],
    ["kdf_params not JSON", { kdf_params: "nope" }],
    ["protected_vault_key missing", { protected_vault_key: undefined }],
  ])("rejects invalid input and changes nothing: %s", async (_name, overrides) => {
    await setupVault();
    const caller = await login();
    const other = await login("device-b");
    const res = await changePassword(caller.session_token, passwordChange(overrides));
    expect(res.status).toBe(400);
    // no partial update: the other session survives and the old password still works
    expect((await call("/v1/vault", { token: other.session_token })).status).toBe(200);
    expect((await login("device-c", AUTH_KEY)).session_token).toBeTruthy();
  });

  it("rejects non-JSON bodies with 415", async () => {
    await setupVault();
    const caller = await login();
    const res = await call("/v1/vault/password", {
      method: "PUT",
      token: caller.session_token,
      body: "x",
      headers: { "Content-Type": "text/plain" },
    });
    expect(res.status).toBe(415);
  });
});
