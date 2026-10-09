import { describe, expect, it } from "vitest";
import {
  AUTH_KEY,
  RECOVERY_AUTH,
  SETUP_TOKEN,
  call,
  envelope,
  freshIp,
  key32,
  setupPayload,
  setupVault,
} from "./helpers";

const loginAttempt = (ip: string, authKey = key32(99), extra: Parameters<typeof call>[1] = {}) =>
  call("/v1/login", {
    ip,
    json: { auth_key: authKey, device_id: "device-a", device_name: envelope() },
    ...extra,
  });

/** A limiter that records its keys and allows or denies according to `allow`. */
const fakeLimiter = (allow: () => boolean | Promise<boolean>) => {
  const keys: string[] = [];
  const limiter = {
    async limit({ key }: { key: string }) {
      keys.push(key);
      return { success: await allow() };
    },
  } satisfies RateLimit;
  return { limiter, keys };
};

/** The binding's period in wrangler.toml. */
const WINDOW_MS = 60_000;
const windowNow = () => Math.floor(Date.now() / WINDOW_MS);

/**
 * Runs `attempts` until one run lands in a single rate-limit window, and passes on that run's
 * outcome. The local binding counts in fixed windows aligned to the clock, on the minute for a
 * 60 s period, and forgets every count when a window ends, so a run that crosses into the next
 * window proves nothing and is repeated there. Each run takes fresh IPs, because the crossing run
 * left counts in the new window. Date.now() before the first attempt and after the last one
 * brackets the binding's own reads of the clock.
 */
async function inOneWindow(attempts: () => Promise<void>): Promise<void> {
  const window = windowNow();
  const failure = await attempts().then(
    () => null,
    (error: unknown) => ({ error }),
  );
  if (windowNow() !== window) return inOneWindow(attempts);
  if (failure) throw failure.error;
}

describe("rate limiting (Workers Rate Limiting binding)", () => {
  it("allows 10 attempts per minute per IP and then answers 429", async () => {
    await setupVault();
    await inOneWindow(async () => {
      const ip = freshIp();
      for (let i = 0; i < 10; i++) {
        expect((await loginAttempt(ip)).status, `attempt ${i + 1}`).toBe(401);
      }
      const limited = await loginAttempt(ip);
      expect(limited.status).toBe(429);
      expect(limited.body).toMatchObject({ error: "rate_limited" });
      expect(limited.headers.get("Retry-After")).toBe("60");

      // Once limited, even the correct key is refused for this IP ...
      expect((await loginAttempt(ip, AUTH_KEY)).status).toBe(429);
      // ... while other IPs are unaffected.
      expect((await loginAttempt(freshIp(), AUTH_KEY)).status).toBe(200);
    });
  });

  it("limits /v1/recover and /v1/setup independently per endpoint", async () => {
    await setupVault();
    await inOneWindow(async () => {
      const ip = freshIp();
      for (let i = 0; i < 10; i++) {
        await call("/v1/recover", { ip, json: { recovery_auth: key32(98), device_id: "d", device_name: envelope() } });
      }
      const recoverLimited = await call("/v1/recover", {
        ip,
        json: { recovery_auth: RECOVERY_AUTH, device_id: "d", device_name: envelope() },
      });
      expect(recoverLimited.status).toBe(429);
      // same IP, different endpoint: its own budget
      expect((await loginAttempt(ip, AUTH_KEY)).status).toBe(200);

      const setupIp = freshIp();
      for (let i = 0; i < 10; i++) {
        expect((await call("/v1/setup", { ip: setupIp, token: "wrong", json: setupPayload() })).status).toBe(401);
      }
      expect((await call("/v1/setup", { ip: setupIp, token: "wrong", json: setupPayload() })).status).toBe(429);
    });
  });

  it("does not rate limit the other endpoints", async () => {
    // A window ending midway would reset a limit and hide it.
    await inOneWindow(async () => {
      const ip = freshIp();
      for (let i = 0; i < 15; i++) expect((await call("/v1/health", { ip })).status).toBe(200);
      for (let i = 0; i < 15; i++) expect((await call("/v1/prelogin", { ip })).status).toBe(404);
    });
  });

  it("keys the limiter by endpoint and CF-Connecting-IP", async () => {
    await setupVault();
    const { limiter, keys } = fakeLimiter(() => true);
    await loginAttempt("203.0.113.9", AUTH_KEY, { env: { AUTH_LIMITER: limiter } });
    await call("/v1/recover", {
      ip: "203.0.113.10",
      env: { AUTH_LIMITER: limiter },
      json: { recovery_auth: RECOVERY_AUTH, device_id: "d", device_name: envelope() },
    });
    await call("/v1/setup", { ip: "203.0.113.11", env: { AUTH_LIMITER: limiter }, token: "x", json: {} });
    expect(keys).toEqual(["login:203.0.113.9", "recover:203.0.113.10", "setup:203.0.113.11"]);
  });

  it("rejects before touching credentials when the limiter denies, even with a valid setup token", async () => {
    const { limiter } = fakeLimiter(() => false);
    const res = await call("/v1/setup", {
      token: SETUP_TOKEN,
      json: setupPayload(),
      env: { AUTH_LIMITER: limiter },
    });
    expect(res.status).toBe(429);
    expect((await call("/v1/health")).body.initialized).toBe(false);
  });

  it("skips limiting when the binding is absent (local development)", async () => {
    await setupVault();
    const ip = freshIp();
    for (let i = 0; i < 15; i++) {
      const res = await loginAttempt(ip, key32(99), { env: { AUTH_LIMITER: undefined } });
      expect(res.status).toBe(401);
    }
  });

  it("fails open when the limiter itself errors", async () => {
    await setupVault();
    const { limiter } = fakeLimiter(() => {
      throw new Error("limiter down");
    });
    const res = await loginAttempt(freshIp(), AUTH_KEY, { env: { AUTH_LIMITER: limiter } });
    expect(res.status).toBe(200);
  });
});
