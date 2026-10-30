import { Hono } from "hono";
import { MAX_BODY_BYTES_SMALL, MAX_DEVICE_NAME_BYTES, SECRET_KEY_BYTES } from "../config";
import type { AppEnv } from "../env";
import { invalidCredentials, notInitialized } from "../errors";
import { limitBody, rateLimit } from "../middleware";
import { createSession } from "../sessions";
import { constantTimeEqual, sha256Hex } from "../util";
import { readBase64Secret, readDevice, readJsonBody } from "../validate";

export const authRoutes = new Hono<AppEnv>();

/** Exchanges `auth_key` (derived from the master password) for a 30-day session. */
authRoutes.post("/login", rateLimit("login"), limitBody(MAX_BODY_BYTES_SMALL), async (c) => {
  const body = await readJsonBody(c);
  const authKey = readBase64Secret(body, "auth_key", SECRET_KEY_BYTES);
  const { deviceId, deviceName } = readDevice(body, MAX_DEVICE_NAME_BYTES);

  const meta = await c.env.DB.prepare("SELECT auth_hash FROM meta WHERE id = 1").first<{ auth_hash: string }>();
  if (!meta) throw notInitialized();
  const authHash = await sha256Hex(authKey);
  if (!(await constantTimeEqual(authHash, meta.auth_hash))) throw invalidCredentials();

  const session = await createSession(c.env.DB, { deviceId, deviceName, scope: "full", credentialHash: authHash });
  if (!session) throw invalidCredentials();
  return c.json({ session_token: session.token, expires_at: session.expiresAt });
});

/**
 * Account recovery with the recovery code. Returns the wrapped recovery key (so the client can
 * recover the vault key) and a 15-minute restricted session that may only call
 * `PUT /v1/vault/password`.
 */
authRoutes.post("/recover", rateLimit("recover"), limitBody(MAX_BODY_BYTES_SMALL), async (c) => {
  const body = await readJsonBody(c);
  const recoveryAuth = readBase64Secret(body, "recovery_auth", SECRET_KEY_BYTES);
  const { deviceId, deviceName } = readDevice(body, MAX_DEVICE_NAME_BYTES);

  const meta = await c.env.DB.prepare(
    "SELECT recovery_auth_hash, recovery_vault_key, kdf_salt, kdf_params FROM meta WHERE id = 1",
  ).first<{ recovery_auth_hash: string; recovery_vault_key: string; kdf_salt: string; kdf_params: string }>();
  if (!meta) throw notInitialized();
  const recoveryAuthHash = await sha256Hex(recoveryAuth);
  if (!(await constantTimeEqual(recoveryAuthHash, meta.recovery_auth_hash))) throw invalidCredentials();

  const session = await createSession(c.env.DB, {
    deviceId,
    deviceName,
    scope: "recovery",
    credentialHash: recoveryAuthHash,
  });
  if (!session) throw invalidCredentials();
  return c.json({
    recovery_vault_key: meta.recovery_vault_key,
    kdf_salt: meta.kdf_salt,
    kdf_params: meta.kdf_params,
    session_token: session.token,
    expires_at: session.expiresAt,
  });
});
