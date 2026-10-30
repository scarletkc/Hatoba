import { env, exports } from "cloudflare:workers";
import app from "../src/index";
import type { Bindings } from "../src/env";

export const SETUP_TOKEN = env.SETUP_TOKEN as string;

/** Test key material: 32 bytes of a single repeated value, as standard base64. */
export const key32 = (fill: number): string => btoa(String.fromCharCode(...new Uint8Array(32).fill(fill)));
export const AUTH_KEY = key32(1);
export const RECOVERY_AUTH = key32(2);
export const NEW_AUTH_KEY = key32(3);
export const NEW_RECOVERY_AUTH = key32(4);

export const KDF_PARAMS = JSON.stringify({ alg: "argon2id", v: 1, m: 65536, t: 3, p: 4 });
export const KDF_SALT = "c2FsdHNhbHRzYWx0c2FsdA==";

export const envelope = (tag = "x") => JSON.stringify({ v: 1, n: "bm9uY2Vub25jZW5v", c: `ciphertext-${tag}` });

let ipCounter = 0;
/** A fresh client IP per call so tests never share a rate-limit bucket. */
export const freshIp = (): string => {
  ipCounter++;
  return `10.${(ipCounter >> 8) & 255}.${ipCounter & 255}.7`;
};

export interface CallOptions {
  method?: string;
  token?: string;
  json?: unknown;
  body?: string;
  headers?: Record<string, string>;
  ip?: string;
  /** Run against a custom environment instead of the Worker under test. */
  env?: Partial<Bindings>;
}

export interface ApiResponse {
  status: number;
  headers: Headers;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  body: any;
}

export async function call(path: string, opts: CallOptions = {}): Promise<ApiResponse> {
  const headers = new Headers(opts.headers);
  headers.set("CF-Connecting-IP", opts.ip ?? freshIp());
  if (opts.token) headers.set("Authorization", `Bearer ${opts.token}`);
  let body = opts.body;
  if (opts.json !== undefined) {
    body = JSON.stringify(opts.json);
    if (!headers.has("Content-Type")) headers.set("Content-Type", "application/json");
  }
  const request = new Request(`https://sync.test${path}`, {
    method: opts.method ?? (body === undefined ? "GET" : "POST"),
    headers,
    body,
  });
  const response = opts.env
    ? await app.fetch(request, { DB: env.DB, ...opts.env } satisfies Bindings)
    : await exports.default.fetch(request);
  const text = await response.text();
  const isJson = response.headers.get("Content-Type")?.includes("application/json");
  return { status: response.status, headers: response.headers, body: isJson ? JSON.parse(text) : text };
}

/**
 * `env.DB` for one request (pass it as `call(..., { env: { DB } })`) whose first `batch()` waits
 * for `meanwhile`, so another request runs after the handler's checks and before its writes.
 */
export function pausedBeforeBatch(meanwhile: () => Promise<void>): D1Database {
  let pending: (() => Promise<void>) | null = meanwhile;
  return {
    prepare: (query: string) => env.DB.prepare(query),
    batch: async (statements: D1PreparedStatement[]) => {
      const run = pending;
      pending = null;
      if (run) await run();
      return env.DB.batch(statements);
    },
  } as unknown as D1Database;
}

export const changePassword = (token: string, json: unknown) =>
  call("/v1/vault/password", { method: "PUT", token, json });

export const setupPayload = (overrides: Record<string, unknown> = {}) => ({
  schema_version: 1,
  kdf_salt: KDF_SALT,
  kdf_params: KDF_PARAMS,
  auth_key: AUTH_KEY,
  protected_vault_key: envelope("pvk"),
  recovery_vault_key: envelope("rvk"),
  recovery_auth: RECOVERY_AUTH,
  ...overrides,
});

export async function setupVault(overrides: Record<string, unknown> = {}): Promise<void> {
  const res = await call("/v1/setup", { token: SETUP_TOKEN, json: setupPayload(overrides) });
  if (res.status !== 201) throw new Error(`setup failed: ${res.status} ${JSON.stringify(res.body)}`);
}

export async function login(deviceId = "device-a", authKey = AUTH_KEY, deviceName = envelope("name-a")) {
  const res = await call("/v1/login", {
    json: { auth_key: authKey, device_id: deviceId, device_name: deviceName },
  });
  if (res.status !== 200) throw new Error(`login failed: ${res.status} ${JSON.stringify(res.body)}`);
  return res.body as { session_token: string; expires_at: number };
}

/** Sets up the vault and returns a logged-in session token. */
export async function loggedIn(deviceId = "device-a"): Promise<string> {
  await setupVault();
  return (await login(deviceId)).session_token;
}

export function passwordChange(overrides: Record<string, unknown> = {}) {
  return {
    kdf_salt: "bmV3c2FsdG5ld3NhbHRuZXc=",
    kdf_params: JSON.stringify({ alg: "argon2id", v: 1, m: 65536, t: 3, p: 4, rev: 2 }),
    auth_key: NEW_AUTH_KEY,
    protected_vault_key: envelope("pvk2"),
    ...overrides,
  };
}

export const change = (id: string, baseRevision: number, tag = "v", extra: Record<string, unknown> = {}) => ({
  id,
  base_revision: baseRevision,
  deleted: false,
  envelope: envelope(tag),
  ...extra,
});

export const push = (token: string, changes: unknown[]) => call("/v1/items", { token, json: { changes } });
