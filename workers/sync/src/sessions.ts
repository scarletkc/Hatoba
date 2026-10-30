import { RECOVERY_SESSION_TTL_MS, SESSION_TOKEN_BYTES, SESSION_TTL_MS, type SessionScope } from "./config";
import { bytesToBase64Url, randomBytes, sha256Hex } from "./util";

export interface NewSession {
  token: string;
  expiresAt: number;
}

/**
 * Creates a session and returns the bearer token (shown to the client exactly once), or null when
 * `credentialHash` no longer matches `meta`: `auth_hash` for a full session, `recovery_auth_hash`
 * for a recovery session. The caller has already checked the credential, but a password change
 * may have committed since; checking again inside the insert's transaction means such a change
 * cannot be outlived by a session issued for the old credential.
 *
 * Only the SHA-256 of the token is persisted. In the same transaction, expired sessions are
 * purged and any older session of the same device and scope is replaced, so a device has at
 * most one active session.
 */
export async function createSession(
  db: D1Database,
  input: { deviceId: string; deviceName: string; scope: SessionScope; credentialHash: string },
): Promise<NewSession | null> {
  const token = bytesToBase64Url(randomBytes(SESSION_TOKEN_BYTES));
  const tokenHash = await sha256Hex(token);
  const now = Date.now();
  const expiresAt = now + (input.scope === "full" ? SESSION_TTL_MS : RECOVERY_SESSION_TTL_MS);
  const column = input.scope === "full" ? "auth_hash" : "recovery_auth_hash";
  const credentialMatches = `EXISTS (SELECT 1 FROM meta WHERE id = 1 AND ${column} = ?)`;

  const [, , inserted] = await db.batch([
    db.prepare("DELETE FROM sessions WHERE expires_at <= ?").bind(now),
    db
      .prepare(`DELETE FROM sessions WHERE device_id = ? AND scope = ? AND ${credentialMatches}`)
      .bind(input.deviceId, input.scope, input.credentialHash),
    db
      .prepare(
        `INSERT INTO sessions (token_hash, device_id, device_name, scope, created_at, last_seen, expires_at)
         SELECT ?, ?, ?, ?, ?, ?, ? WHERE ${credentialMatches}`,
      )
      .bind(tokenHash, input.deviceId, input.deviceName, input.scope, now, now, expiresAt, input.credentialHash),
  ]);

  return inserted?.meta.changes === 1 ? { token, expiresAt } : null;
}
