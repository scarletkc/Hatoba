/** Tunable limits and lifetimes. All durations are in milliseconds. */

export const SERVICE_NAME = "hatoba-sync";
export const API_VERSION = 1;
/** Keep in sync with package.json (enforced by a test). */
export const VERSION = "0.1.1";

const MINUTE = 60_000;
const DAY = 24 * 60 * MINUTE;

/** Full sessions live 30 days and are renewed on use (sliding window). */
export const SESSION_TTL_MS = 30 * DAY;
/** Restricted sessions issued by /v1/recover may only change the password. */
export const RECOVERY_SESSION_TTL_MS = 15 * MINUTE;
/** Sliding renewal writes to D1 at most once per interval per session. */
export const SESSION_TOUCH_INTERVAL_MS = MINUTE;

export const MAX_CHANGES_PER_PUSH = 100;
export const MAX_ENVELOPE_BYTES = 64 * 1024;
export const MAX_DEVICE_NAME_BYTES = 4 * 1024;
/** Wrapped vault keys are small envelopes; this is a generous sanity bound. */
export const MAX_KEY_BLOB_BYTES = 4 * 1024;
export const MAX_KDF_PARAMS_BYTES = 1024;

export const DEFAULT_PULL_LIMIT = 500;
export const MAX_PULL_LIMIT = 1000;

/** Request body ceilings (bytes). A push is at most 100 envelopes of 64 KB plus JSON overhead. */
export const MAX_BODY_BYTES_SMALL = 32 * 1024;
export const MAX_BODY_BYTES_PUSH = 8 * 1024 * 1024;

/** `auth_key` / `recovery_auth` are 32-byte values sent as standard base64. */
export const SECRET_KEY_BYTES = 32;
export const SESSION_TOKEN_BYTES = 32;

export type SessionScope = "full" | "recovery";
