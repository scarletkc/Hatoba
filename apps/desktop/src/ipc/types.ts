/**
 * IPC contract between the WebView and the Rust backend (spec §10).
 *
 * These DTOs mirror the Rust types in `apps/desktop/src-tauri/src/dto.rs`, which are exported
 * with tauri-specta. DTOs never carry secrets: the WebView only ever sees metadata such as
 * `has_password`, never the password itself (spec §3.2 / §10.1).
 *
 * Timestamps are Unix epoch milliseconds.
 */

// ───────────────────────── Errors ─────────────────────────

export type ErrorCode =
  | "locked"
  | "not_initialized"
  | "already_initialized"
  | "wrong_password"
  | "wrong_recovery_code"
  | "throttled"
  | "not_found"
  | "invalid_input"
  | "key_parse"
  | "ssh"
  | "sftp"
  | "sync"
  | "sync_auth"
  | "sync_offline"
  | "remote_initialized"
  | "remote_not_initialized"
  | "cloudflare_token"
  | "cloudflare_permission"
  | "cloudflare"
  | "subdomain_required"
  | "subdomain_unavailable"
  | "worker_name_taken"
  | "no_worker_bundle"
  | "cancelled"
  | "io"
  | "internal";

export type SshErrorKind =
  | "dns"
  | "refused"
  | "timeout"
  | "unreachable"
  | "auth_failed"
  | "host_key_rejected"
  | "key_parse"
  | "disconnected"
  | "protocol"
  | "io"
  | "channel"
  | "sftp"
  | "cancelled"
  | "other";

export type KeyParseErrorKind =
  | "unsupported_format"
  | "passphrase_required"
  | "wrong_passphrase"
  | "unsupported_algorithm"
  | "invalid";

/** Every command rejects with this shape. `detail` is technical English text, safe to show. */
export interface AppError {
  code: ErrorCode;
  detail: string;
  /** `throttled`: unlock allowed again at this time. */
  retry_at?: number | null;
  /** `invalid_input`: offending field name. */
  field?: string | null;
  /** `ssh`: classified SSH failure. */
  ssh_kind?: SshErrorKind | null;
  /** `key_parse`: why the private key could not be read. */
  key_kind?: KeyParseErrorKind | null;
  /** `cloudflare_permission`: the permission the API token lacks. */
  permission?: CloudflarePermission | null;
  /** `cloudflare`: Cloudflare's numeric error code, when it sent one. */
  cf_code?: number | null;
}

export type CloudflarePermission = "workers_scripts" | "d1";

// ───────────────────────── Vault ─────────────────────────

export type VaultState = "uninitialized" | "locked" | "unlocked";

export interface VaultStatus {
  state: VaultState;
  /** Consecutive failed unlock attempts (SEC-06). */
  failed_attempts: number;
  /** When throttled, the next attempt is allowed at this time. */
  retry_at: number | null;
  sync_kind: SyncKind;
  /** Windows Hello / Touch ID is available on this device (P1). */
  biometric_available: boolean;
  /** Biometric unlock has been enrolled for this vault (P1). */
  biometric_enabled: boolean;
}

// ───────────────────────── Hosts ─────────────────────────

export type AuthKind = "password" | "key" | "agent" | "ask";

export interface HostView {
  id: string;
  name: string;
  address: string;
  port: number;
  username: string;
  auth_kind: AuthKind;
  /** A password is saved (HOST-08: it can be replaced but never read back). */
  has_password: boolean;
  key_id: string | null;
  group_id: string | null;
  tags: string[];
  favorite: boolean;
  jump_host_id: string | null;
  note: string;
  updated_at: number;
  /** Device-local, never synced (HOST-06). */
  last_connected_at: number | null;
}

export interface HostInput {
  /** `null` creates a new host. */
  id: string | null;
  name: string;
  address: string;
  port: number;
  username: string;
  auth_kind: AuthKind;
  /** Only for `auth_kind = "password"`: `null` keeps the saved password, a string replaces it. */
  password: string | null;
  key_id: string | null;
  group_id: string | null;
  tags: string[];
  favorite: boolean;
  jump_host_id: string | null;
  note: string;
}

export interface GroupView {
  id: string;
  name: string;
  parent_id: string | null;
  sort: number;
}

export interface GroupInput {
  id: string | null;
  name: string;
  parent_id: string | null;
  sort: number;
}

export interface TagCount {
  name: string;
  count: number;
}

export interface SshConfigCandidate {
  alias: string;
  address: string;
  port: number;
  username: string;
  identity_file: string | null;
  proxy_jump: string | null;
  /** A host with the same name already exists. */
  exists: boolean;
}

export interface ImportResult {
  hosts_created: number;
  keys_imported: number;
  warnings: string[];
}

export interface ProbeResult {
  id: string;
  online: boolean;
  latency_ms: number | null;
}

// ───────────────────────── Keys ─────────────────────────

export type KeyAlgorithm = "ed25519" | "ecdsa" | "rsa";

export interface KeyView {
  id: string;
  name: string;
  algorithm: KeyAlgorithm;
  bits: number;
  public_key: string;
  fingerprint: string;
  comment: string;
  has_passphrase: boolean;
  created_at: number;
  updated_at: number;
  /** IDs of hosts using this key (KEY-03 / KEY-05). */
  used_by: string[];
}

export interface KeyImportInput {
  name: string;
  /** Pasted private key text. Ignored when `path` is set. */
  private_key: string | null;
  /** File chosen with the native dialog; Rust reads it. */
  path: string | null;
  passphrase: string | null;
}

export interface KeyGenerateInput {
  name: string;
  algorithm: "ed25519" | "rsa";
  comment: string;
  passphrase: string | null;
}

// ───────────────────────── SSH / terminal ─────────────────────────

export interface ConnectOptions {
  cols: number;
  rows: number;
  /** For `auth_kind = "ask"` (SSH-03): used once, never stored. */
  password: string | null;
  /** For a passphrase-protected key whose passphrase isn't saved. */
  passphrase: string | null;
}

export type SessionState = "connecting" | "connected" | "disconnected" | "failed";

export interface SessionStateEvent {
  session_id: string;
  host_id: string;
  state: SessionState;
  /** Connection round-trip time, once connected. */
  latency_ms: number | null;
  error: AppError | null;
  /** Remote exit status for a clean `disconnected`. */
  exit_status: number | null;
}

export interface HostKeyPrompt {
  request_id: string;
  session_id: string | null;
  host: string;
  port: number;
  key_type: string;
  fingerprint: string;
  /** `new`: first connection (TOFU). `changed`: fingerprint differs from the saved one — connection is blocked (SSH-04). */
  kind: "new" | "changed";
  known_fingerprint: string | null;
  known_key_type: string | null;
}

/** keyboard-interactive / 2FA prompt (SSH-08, P1). */
export interface AuthPrompt {
  request_id: string;
  session_id: string | null;
  name: string;
  instructions: string;
  prompts: { prompt: string; echo: boolean }[];
}

export interface TestResult {
  ok: boolean;
  latency_ms: number | null;
  host_key_verified: boolean;
  error: AppError | null;
}

/**
 * Terminal output arrives on a per-session Tauri Channel as raw bytes (ArrayBuffer), §10.3.
 * Byte 0 is the frame tag: 0 = data, 1 = closed (UTF-8 reason follows), 2 = error (UTF-8 message follows).
 */
export const FRAME_DATA = 0;
export const FRAME_CLOSED = 1;
export const FRAME_ERROR = 2;

// ───────────────────────── Port forwarding (FWD-01/02) ─────────────────────────

/** A saved local (`-L`) forward. Remote / dynamic forwards are P2. */
export interface ForwardView {
  id: string;
  host_id: string;
  bind_address: string;
  /** 0 = pick a free port. */
  bind_port: number;
  dest_host: string;
  dest_port: number;
  /** FWD-02: start whenever the host connects. */
  auto_start: boolean;
}

export interface ForwardInput {
  id: string | null;
  host_id: string;
  bind_address: string;
  bind_port: number;
  dest_host: string;
  dest_port: number;
  auto_start: boolean;
}

export type ForwardState = "running" | "stopped" | "failed";

export interface ForwardStatusEvent {
  session_id: string;
  forward_id: string;
  state: ForwardState;
  local_port: number | null;
  error: string | null;
}

// ───────────────────────── SFTP ─────────────────────────

export interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  is_symlink: boolean;
  size: number;
  modified: number | null;
  /** e.g. "drwxr-xr-x" */
  permissions: string;
}

export type TransferState = "running" | "done" | "failed" | "cancelled";

export interface TransferProgressEvent {
  transfer_id: string;
  session_id: string;
  direction: "upload" | "download";
  name: string;
  bytes: number;
  total: number;
  bytes_per_sec: number;
  state: TransferState;
  error: string | null;
}

// ───────────────────────── Sync ─────────────────────────

export type SyncKind = "none" | "worker" | "d1";

export type SyncState = "off" | "idle" | "syncing" | "offline" | "auth_failed" | "error";

export interface SyncStatus {
  kind: SyncKind;
  /** Worker URL host or D1 database name. */
  endpoint: string | null;
  database: string | null;
  state: SyncState;
  last_synced_at: number | null;
  /** Local changes not yet pushed. */
  pending: number;
  /** Unreviewed automatically-resolved conflicts (§6.4). */
  conflicts: number;
  auto_sync: boolean;
  message: string | null;
  counts: { hosts: number; keys: number; groups: number } | null;
}

export type SyncConfigInput =
  | { kind: "worker"; url: string; setup_token: string | null }
  | { kind: "d1"; account_id: string; database_id: string; api_token: string };

export interface SyncTestResult {
  ok: boolean;
  /** The remote already holds a vault. */
  initialized: boolean;
  version: string | null;
  latency_ms: number | null;
  /** D1 direct mode: databases the token can see, to pick from after the token is verified. */
  databases: D1Database[] | null;
  error: AppError | null;
}

export interface D1Database {
  id: string;
  name: string;
  region: string | null;
}

// ───────────────────────── In-app deployment (§6.7) ─────────────────────────

export interface CloudflareAccount {
  id: string;
  name: string;
}

/** The API token passed step 1. Later calls name the deployment by `handle`; the token stays in Rust. */
export interface DeployStart {
  handle: string;
  /** The token belongs to the account entered with it rather than to a user. */
  account_owned: boolean;
  /** The accounts a user token reaches; empty when the account ID has to be entered. */
  accounts: CloudflareAccount[];
  /** Default names. */
  worker_name: string;
  database_name: string;
}

export interface DeployTarget {
  account_id: string;
  worker_name: string;
  database_name: string;
  /** The workers.dev subdomain to create when the account has none. */
  subdomain: string | null;
}

/** `has_vault` and `foreign` stop the deployment. */
export type DeployWorkerAction = "create" | "update" | "has_vault" | "foreign";
export type DeployDatabaseAction = "create" | "use" | "bound";

/** What step 2 found, before anything is written. */
export interface DeployPlan {
  /** `null`: the account has no workers.dev subdomain yet, so the user chooses one. */
  subdomain: string | null;
  worker: DeployWorkerAction;
  database: DeployDatabaseAction | null;
  database_name: string | null;
}

export type DeployStep = "verify" | "inspect" | "create_database" | "migrate" | "upload" | "setup_token" | "route" | "wait";
export type DeployStepStatus = "running" | "done" | "skipped";

export interface DeployProgress {
  step: DeployStep;
  status: DeployStepStatus;
}

export interface DeployOutcome {
  url: string;
  /** `false`: the Worker does not answer yet (step 8 is waiting); `deploy_check` asks again. */
  ready: boolean;
}

export interface DeviceView {
  device_id: string;
  name: string;
  platform: string;
  created_at: number;
  last_seen: number;
  current: boolean;
}

export type ConflictResolution = "local_won" | "remote_won" | "kept_both" | "modified_won";

export interface ConflictField {
  /** i18n-able field key, e.g. "port", "jump_host", "tags", "name", "state". */
  field: string;
  local: string | null;
  remote: string | null;
}

export interface ConflictView {
  id: number;
  item_id: string;
  item_type: "host" | "group" | "key" | "known_host" | "forward" | "snippet" | "settings";
  item_name: string;
  resolution: ConflictResolution;
  local_updated_at: number | null;
  remote_updated_at: number | null;
  local_deleted: boolean;
  remote_deleted: boolean;
  /** Only fields that differ; secrets are never included. */
  fields: ConflictField[];
  created_at: number;
}

// ───────────────────────── Settings ─────────────────────────

export interface TerminalSettings {
  font_family: string;
  font_size: number;
  theme: "system" | "light" | "dark";
  cursor_style: "block" | "bar" | "underline";
  scrollback: number;
  /** WIN-05: PuTTY-style right click, or a context menu. */
  right_click: "copy_paste" | "menu";
  /** TERM-04: ask before pasting text with line breaks. */
  confirm_multiline_paste: boolean;
}

/** Synced settings item (spec §5.1 `Settings`). */
export interface SettingsView {
  terminal: TerminalSettings;
  auto_lock_minutes: number;
  lock_disconnects_sessions: boolean;
}

export type Language = "system" | "zh-CN" | "en" | "ja";

/** Device-local preferences: readable while locked (the unlock screen needs the language). */
export interface LocalPrefs {
  language: Language;
  appearance: "system" | "light" | "dark";
  density: "regular" | "compact";
  /** HOST-10 TCP reachability dots. */
  host_probe: boolean;
  /** Check GitHub Releases once after unlock. Off by default (no telemetry, spec §11). */
  auto_update_check: boolean;
}

/** Device-local state of the sidebar's GitHub star prompt (spec §9). */
export interface StarPrompt {
  /** When this device first read the state, Unix ms. The prompt waits a day from then. */
  first_seen_at: number;
  /** The user starred the repository, opened the bug report form, or closed the prompt. */
  done: boolean;
}

export interface AppInfo {
  version: string;
  platform: "windows" | "macos" | "linux" | "web";
  /** Windows 11 Mica is active (WIN-07). */
  mica: boolean;
}

/** What the update check found on GitHub Releases (spec §11). */
export interface UpdateCheck {
  current_version: string;
  /** The newest stable release, or null before the first release. */
  latest_version: string | null;
  /** The release page to download the installer from. */
  release_url: string | null;
  update_available: boolean;
}

// ───────────────────────── Events (§10.2) ─────────────────────────

export interface VaultLockedEvent {
  reason: "manual" | "idle" | "sleep";
}

export interface EventMap {
  "vault://locked": VaultLockedEvent;
  "sync://status": SyncStatus;
  "ssh://hostkey-prompt": HostKeyPrompt;
  "ssh://auth-prompt": AuthPrompt;
  "ssh://state": SessionStateEvent;
  "transfer://progress": TransferProgressEvent;
  "ssh://forward": ForwardStatusEvent;
}
