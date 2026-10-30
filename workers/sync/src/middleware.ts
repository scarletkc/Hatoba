import type { MiddlewareHandler } from "hono";
import { bodyLimit } from "hono/body-limit";
import { SESSION_TOUCH_INTERVAL_MS, SESSION_TTL_MS, type SessionScope } from "./config";
import type { AppEnv } from "./env";
import { ApiError, invalidSession } from "./errors";
import { sha256Hex } from "./util";

/** Returns the token of an `Authorization: Bearer <token>` header, or null. */
export function bearerToken(header: string | undefined): string | null {
  const match = /^Bearer ([\x21-\x7e]{1,512})$/i.exec(header?.trim() ?? "");
  return match?.[1] ?? null;
}

/** Rejects request bodies larger than `maxBytes` with 413 (checked before the body is parsed). */
export const limitBody = (maxBytes: number): MiddlewareHandler<AppEnv> =>
  bodyLimit({
    maxSize: maxBytes,
    onError: () => {
      throw new ApiError(413, "payload_too_large", `Request body must be at most ${maxBytes} bytes`);
    },
  });

/**
 * Per-IP, per-endpoint rate limit using the Workers Rate Limiting binding. Skipped when the
 * binding is absent (local development). A failing limiter fails open: it is abuse protection,
 * not an authentication control, since every secret here is a 128+ bit random value.
 */
export const rateLimit =
  (endpoint: string): MiddlewareHandler<AppEnv> =>
  async (c, next) => {
    const limiter = c.env.AUTH_LIMITER;
    if (limiter) {
      const ip = c.req.header("CF-Connecting-IP") ?? "unknown";
      let allowed = true;
      try {
        allowed = (await limiter.limit({ key: `${endpoint}:${ip}` })).success;
      } catch {
        console.warn("rate limiter unavailable, allowing request");
      }
      if (!allowed) {
        throw new ApiError(429, "rate_limited", "Too many attempts, retry later", { "Retry-After": "60" });
      }
    }
    await next();
  };

interface SessionRow {
  device_id: string;
  scope: SessionScope;
  last_seen: number;
  expires_at: number;
}

/**
 * Authenticates `Authorization: Bearer <session_token>` against the sessions table.
 *
 * `allow` lists the session scopes accepted by the route; by default only full sessions.
 * Full sessions are renewed on use (sliding 30-day expiry); the write is throttled to once per
 * minute. Recovery sessions have a fixed lifetime and are never extended.
 *
 * The lookup is by SHA-256 of a 256-bit random token, so the (non constant-time) index lookup
 * reveals nothing an attacker could exploit.
 */
export const requireSession =
  (allow: readonly SessionScope[] = ["full"]): MiddlewareHandler<AppEnv> =>
  async (c, next) => {
    const token = bearerToken(c.req.header("Authorization"));
    if (!token) {
      throw new ApiError(401, "unauthorized", "Missing bearer token", { "WWW-Authenticate": "Bearer" });
    }

    const tokenHash = await sha256Hex(token);
    const row = await c.env.DB.prepare(
      "SELECT device_id, scope, last_seen, expires_at FROM sessions WHERE token_hash = ?",
    )
      .bind(tokenHash)
      .first<SessionRow>();

    const now = Date.now();
    if (!row || row.expires_at <= now) throw invalidSession();
    if (!allow.includes(row.scope)) {
      throw new ApiError(403, "insufficient_scope", "This session is not allowed to call this endpoint");
    }

    if (row.scope === "full" && now - row.last_seen >= SESSION_TOUCH_INTERVAL_MS) {
      await c.env.DB.prepare("UPDATE sessions SET last_seen = ?, expires_at = ? WHERE token_hash = ?")
        .bind(now, now + SESSION_TTL_MS, tokenHash)
        .run();
    }

    c.set("session", { tokenHash, deviceId: row.device_id, scope: row.scope });
    await next();
  };
