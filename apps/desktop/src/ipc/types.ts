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
  /** Another sync setup (flow A or C, §6.6) is still running. */
  | "sync_setup_busy"
  /** Sync is already on for this device, so it cannot join a cloud vault (flow C). */
  | "sync_already_configured"
  | "cloudflare_token"
  | "cloudflare_permission"
  | "cloudflare"
  | "subdomain_required"
  | "subdomain_unavailable"
  | "worker_name_taken"
  | "worker_not_found"
  | "worker_newer"
  | "no_worker_bundle"
  /** AI assistant (§13): a model provider, search provider or fetched page failed; see `http_status`. */
  | "ai"
  /** A downloaded update does not carry a valid signature from the release key (spec §11). */
  | "update_signature"
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
  /** SSH-13: the proxy could not be reached. */
  | "proxy_unreachable"
  /** SSH-13: the proxy wants a username and password, or did not accept them. */
  | "proxy_auth"
  /** SSH-13: the proxy did not open the connection to the server. */
  | "proxy"
  /** SSH-13: the host or this device's default names a proxy that was deleted. */
  | "proxy_missing"
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
  /** `ai`: the HTTP status the provider answered with, when it answered. */
  http_status?: number | null;
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

/** Which proxy a host's connection goes through (SSH-13). With a jump host, the jump host's choice applies. */
export type ProxyMode = "device_default" | "direct" | "proxy";

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
  /** SSH-13: the device default, no proxy, or the saved proxy in `proxy_id`. */
  proxy_mode: ProxyMode;
  proxy_id: string | null;
  note: string;
  /** What the AI assistant is told about the host, in its system prompt (AI-37). */
  ai_notes: string;
  updated_at: number;
  /** Device-local, never synced (HOST-06). */
  last_connected_at: number | null;
  /** The server's OS from its SSH identification string, such as `ubuntu`; device-local, never synced (HOST-11). */
  os: string | null;
  /** The host's terminals show the server's resource usage; device-local, never synced, off until turned on (TERM-12). */
  show_stats: boolean;
  /** The dashboard shows the host; device-local, never synced, on until turned off (HOST-14). */
  dashboard: boolean;
  /** Environment variables the terminal asks the server to set (SSH-14). */
  env: HostEnvVar[];
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
  proxy_mode: ProxyMode;
  /** Required when `proxy_mode` is `proxy`, ignored otherwise. */
  proxy_id: string | null;
  note: string;
  /** At most 16 KB of UTF-8 (AI-37). */
  ai_notes: string;
  /** Checked as in `features/hosts/envVars.ts` (SSH-14). */
  env: HostEnvVar[];
}

/** One of a host's environment variables (SSH-14). */
export interface HostEnvVar {
  name: string;
  value: string;
}

export interface GroupView {
  id: string;
  name: string;
  parent_id: string | null;
  sort: number;
  /** The dashboard shows the group's hosts; device-local, never synced, on until turned off (HOST-14). */
  dashboard: boolean;
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
  /** The IdentityFile an import with keys reads: the first that exists, otherwise the first listed. */
  identity_file: string | null;
  /** `identity_file` exists, so importing keys would read it. */
  identity_file_found: boolean;
  proxy_jump: string | null;
  /** `ProxyCommand`, which is not imported (SSH-11): the host connects without it. */
  proxy_command: string | null;
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

// ───────────────────────── Proxies (SSH-13) ─────────────────────────

export type ProxyKind = "socks5" | "http";

export interface ProxyView {
  id: string;
  name: string;
  kind: ProxyKind;
  address: string;
  port: number;
  /** Empty when the proxy needs no sign-in. */
  username: string;
  /** A password is saved (as HOST-08: it can be replaced but never read back). */
  has_password: boolean;
  /** Hosts that name this proxy (not those that use it as the device default). */
  host_ids: string[];
  updated_at: number;
}

export interface ProxyInput {
  /** `null` creates a new proxy. */
  id: string | null;
  name: string;
  kind: ProxyKind;
  address: string;
  port: number;
  username: string;
  /** `null` keeps the saved password, an empty string removes it. */
  password: string | null;
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

/**
 * A target typed into the hosts search field (quick connect, HOST-12). It is never saved as a host;
 * once connected it joins the device-local recent list.
 */
export interface QuickTarget {
  /** DNS name or IP address, IPv6 without brackets. */
  address: string;
  port: number;
  username: string;
}

export type SessionState = "connecting" | "connected" | "disconnected" | "failed";

export interface SessionStateEvent {
  session_id: string;
  /** null for a quick-connect session (HOST-12). */
  host_id: string | null;
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
  /** The login password of a quick connection (HOST-12): asked like SSH-03 and answered with one value. */
  password: boolean;
  /** `user@host:port` of the hop that asks. */
  target: string;
}

export interface TestResult {
  ok: boolean;
  latency_ms: number | null;
  host_key_verified: boolean;
  error: AppError | null;
}

/** One reading of a server's resource usage (TERM-12). Sizes in bytes, rates in bytes per second. */
export interface ServerStatsView {
  /** Busy share of all CPUs since the previous reading, 0–100; `null` in the first one. */
  cpu_percent: number | null;
  cpus: number | null;
  /** Load averages over 1, 5 and 15 minutes. */
  load: [number | null, number | null, number | null] | null;
  mem_total: number | null;
  /** The total less what the kernel counts as available. */
  mem_used: number | null;
  /** 0 when the server has no swap. */
  swap_total: number | null;
  swap_used: number | null;
  net_rx_rate: number | null;
  net_tx_rate: number | null;
  /** The interfaces the rates count: those of the default routes, or else all but loopback. */
  net_interfaces: string[];
  /** The root filesystem. */
  disk_total: number | null;
  disk_used: number | null;
  /** Space left for unprivileged users, as `df` counts it. */
  disk_available: number | null;
  uptime_secs: number | null;
  /** Bytes received and sent over `net_interfaces` since each came up, usually at boot. */
  net_rx_total: number | null;
  net_tx_total: number | null;
  /** Every mounted filesystem that stores files, each device once, at most 32; only on the dashboard (HOST-14), refreshed every fifth reading. */
  mounts: MountView[];
}

/** One mounted filesystem's space in bytes, as `df -P` reports it. */
export interface MountView {
  /** What is mounted, such as `/dev/sda1` or `nas:/export`. */
  source: string;
  mount_point: string;
  total: number;
  used: number;
  /** Space left for unprivileged users. */
  available: number;
}

/** Why the dashboard does not sign in to a host on its own (HOST-14). */
export type DashboardHold = "password" | "passphrase" | "prompt" | "host_key_new" | "host_key_changed" | "sign_in_failed";

/** Streamed on the channel of `dashboard_watch` (HOST-14), each naming its host. */
export type DashboardEvent =
  /** Whether the host's first hop (the host, or its outermost jump host) answered the HOST-10 probe. */
  | { kind: "probe"; host_id: string; online: boolean }
  /** Waiting for a connection slot, or signing in. */
  | { kind: "connecting"; host_id: string }
  /** The host's connection settings changed since its readings were made (another server, perhaps): drop them. */
  | { kind: "reset"; host_id: string }
  /** Connected and reading, over the dashboard's own connection or a tab's. */
  | { kind: "reading"; host_id: string; os: string | null; tab: boolean }
  | { kind: "stats"; host_id: string; stats: ServerStatsView }
  /** The server does not run Linux; only its online status shows. */
  | { kind: "unsupported"; host_id: string; system: string }
  /** The connection needs the user; held until **Retry**, a change to the host, or a tab on it. */
  | { kind: "held"; host_id: string; hold: DashboardHold; error: AppError | null }
  /** Connecting or reading failed; tried again after a minute. */
  | { kind: "failed"; host_id: string; error: AppError };

/** Streamed on the channel of `ssh_stats_start` (TERM-12). `unsupported` and `ended` come last. */
export type StatsEvent =
  | { kind: "stats"; stats: ServerStatsView }
  /** The server does not run Linux; `system` is its name, such as `FreeBSD`, or empty. */
  | { kind: "unsupported"; system: string }
  /** Sampling stopped on its own: the script failed or the connection ended. */
  | { kind: "ended"; error: AppError };

/**
 * Terminal output arrives on a per-session Tauri Channel as raw bytes (ArrayBuffer), §10.3.
 * Byte 0 is the frame tag: 0 = data, 1 = closed (UTF-8 reason follows), 2 = error (UTF-8 message follows).
 */
export const FRAME_DATA = 0;
export const FRAME_CLOSED = 1;
export const FRAME_ERROR = 2;
/** The backend session id, sent before connecting, so a tab can answer its prompts if it closes. */
export const FRAME_SESSION = 3;

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

/** `paused`: the Worker's version stops sync until it or the app is updated; `worker_update` says which. */
export type SyncState = "off" | "idle" | "syncing" | "offline" | "auth_failed" | "error" | "paused";

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
  /** Worker mode: how the Worker's `/v1/health` compares with this app (§6.7, Upgrades). `null` when there is nothing to show. */
  worker_update: WorkerUpdate | null;
  /** Items the last sync round couldn't upload because they are larger than sync accepts (§6.2). They stay pending. */
  too_large: TooLargeItem[];
}

export interface TooLargeItem {
  item_id: string;
  item_type: ItemType;
  item_name: string;
}

/** `available`: sync continues and the notice can be dismissed. `required` and `app_required` pause sync. */
export type WorkerUpdateKind = "available" | "required" | "app_required";

export interface WorkerUpdate {
  kind: WorkerUpdateKind;
  /** The version the Worker reports. */
  version: string;
  /** The version "Update Worker" deploys; `null` in builds without the Worker bundle. */
  bundled: string | null;
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
  /** For `has_vault`: the workers.dev URL to join it on (flow C); `null` when no workers.dev URL reaches the Worker. */
  vault_url: string | null;
}

export type DeployStep = "verify" | "inspect" | "create_database" | "migrate" | "upload" | "setup_token" | "route" | "wait";
export type DeployStepStatus = "running" | "done" | "skipped";

export interface DeployProgress {
  step: DeployStep;
  status: DeployStepStatus;
}

/** What "Update Worker" starts from. */
export interface UpgradeDefaults {
  /** The Worker URL in the sync settings. */
  url: string;
  /** Recorded when the app deployed the Worker. */
  account_id: string | null;
  /** Recorded when the app deployed the Worker, or the first label of a workers.dev URL; `null` for a custom domain. */
  worker_name: string | null;
  /** `false`: the Worker may come from the Deploy to Cloudflare button, whose next push replaces the upgrade. */
  deployed_by_app: boolean;
}

export interface UpgradeTarget {
  account_id: string;
  worker_name: string;
}

/** Everything but `upgrade` stops the upgrade. */
export type UpgradeWorkerAction = "upgrade" | "missing" | "foreign" | "no_vault" | "newer";

/** What step 2 of an upgrade found, before anything is written. */
export interface UpgradePlan {
  worker: UpgradeWorkerAction;
  database_name: string | null;
  /** Migrations step 4 applies. */
  migrations: number;
  /** Step 7 runs: the Worker URL is its workers.dev URL. */
  route: boolean;
  /** The version the Worker URL reports, when it answers. */
  version: string | null;
  /** The version the upgrade deploys. */
  bundled: string;
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

export type ItemType =
  | "host"
  | "group"
  | "key"
  | "known_host"
  | "forward"
  | "snippet"
  | "proxy"
  | "settings"
  | "ai_provider"
  | "search_provider"
  | "ai_conversation"
  | "ai_message"
  | "skill"
  | "skill_file"
  | "mcp_server";

export interface ConflictView {
  id: number;
  item_id: string;
  item_type: ItemType;
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
}

/**
 * What `settings_save` changes. Only these fields are written, over the stored settings, so a window
 * that read the settings before a sync pull keeps what other devices changed.
 */
export interface SettingsPatch {
  terminal?: Partial<TerminalSettings>;
}

export type Language = "system" | "zh-CN" | "en" | "ja";

/** Device-local preferences: readable while locked (the unlock screen needs the language). */
export interface LocalPrefs {
  language: Language;
  appearance: "system" | "light" | "dark";
  density: "regular" | "compact";
  /** HOST-10 TCP reachability dots. */
  host_probe: boolean;
  /** Check GitHub Releases once after the first unlock of each launch. On by default (spec §11). */
  auto_update_check: boolean;
  /** AI-16: the permission mode new conversations start in on this device. */
  ai_permission_mode: AiPermissionMode;
  /** AI-16: the user confirmed the first switch to bypass on this device. */
  ai_bypass_confirmed: boolean;
  /** The AI panel is open (§9). */
  ai_panel_open: boolean;
  /** The AI panel's width in CSS pixels. */
  ai_panel_width: number;
  /** SSH-13: the proxy that hosts set to the device default connect through, on this device. */
  default_proxy_id: string | null;
  /** SEC-02: minutes without input before the vault locks; 0 never locks. */
  auto_lock_minutes: number;
  /** SEC-02: lock the vault when the computer sleeps. */
  lock_on_sleep: boolean;
  /** SEC-03: disconnect the SSH sessions when the vault locks. */
  lock_disconnects_sessions: boolean;
  /** SEC-08: seconds until a copied password is cleared from the clipboard; 0 never clears it. */
  clipboard_clear_secs: number;
  /** HOST-14: the dashboard shows cards or a table. */
  dashboard_layout: "cards" | "table";
  /** HOST-14: the table columns turned off. */
  dashboard_hidden_columns: string[];
  /** HOST-14: seconds between readings, 1 to 60. */
  dashboard_interval_secs: number;
  /** HOST-14: the use in percent past which a value stands out; 0 turns one off. */
  dashboard_cpu_alert: number;
  dashboard_mem_alert: number;
  dashboard_disk_alert: number;
  /** WIN-10: what the window's close button does on Windows and Linux. */
  close_action: CloseAction;
}

/** WIN-10: the close button asks, hides the window to the system tray, or quits. Without a tray it quits. */
export type CloseAction = "ask" | "tray" | "quit";

/** WIN-10: the tray menu's labels in the app's language, and whether the vault can be locked. */
export interface TrayMenuInput {
  open: string;
  lock: string;
  quit: string;
  can_lock: boolean;
  /** **Update to vX.Y.Z** while a check has found a newer release; null leaves the item out. */
  update: string | null;
  can_update: boolean;
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
  /** The local user name, which quick connect uses when the target names no user (HOST-12); empty when unknown. */
  local_user: string;
}

/** The `hatoba` command on this device (HOST-13). */
export interface CliCommandView {
  /** Settings → General has it on. */
  enabled: boolean;
  /** The folder added to the user's PATH on Windows, the launcher itself on Linux and macOS. */
  location: string;
  /** Whether that folder is on the PATH Hatoba started with. Only Linux reports false, until the next sign-in. */
  on_path: boolean;
}

/** What the update check found (spec §11). */
export interface UpdateCheck {
  current_version: string;
  /** A release newer than the running version, or null when there is none. */
  update: AvailableUpdate | null;
}

/** A newer release that `update_install` can download and install. */
export interface AvailableUpdate {
  version: string;
  /** The release notes in Markdown. */
  notes: string | null;
  /** When the release was published, Unix ms. */
  published_at: number | null;
  /** The release page on GitHub. */
  release_url: string;
}

/** Sent while `update_install` runs. */
export type UpdateProgress =
  /** Bytes downloaded so far, and the installer's size when the server sent it. */
  | { kind: "downloading"; downloaded: number; total: number | null }
  /** The download passed the signature check. Hatoba closes, and the installer starts it again. */
  | { kind: "installing" };

// ───────────────────────── AI assistant (§13) ─────────────────────────

/** `system_one` answers decision questions for the judge of Auto mode (AI-47) and cannot chat. */
export type AiProtocol = "chat_completions" | "responses" | "anthropic" | "system_one";
export type AiAuthHeader = "x-api-key" | "authorization";

/** A thinking level (AI-05), lowest first. Where one is optional, null is Default. */
export type AiEffort = "low" | "medium" | "high" | "xhigh" | "max";

/** A model of a provider (AI-03). Token limits are null when unknown. */
export interface AiModel {
  id: string;
  name: string;
  context_window: number | null;
  max_output_tokens: number | null;
  /** The thinking levels the model accepts, lowest first (AI-05): [] for none, null when unknown (Low to High are offered). */
  efforts: AiEffort[] | null;
  /** Whether the model supports adaptive thinking (Anthropic's model list), null when unknown. */
  adaptive_thinking: boolean | null;
}

/** AI-01. The API key never reaches the WebView: only whether one is saved. */
export interface AiProviderView {
  id: string;
  name: string;
  protocol: AiProtocol;
  base_url: string;
  has_api_key: boolean;
  /** Only used by `anthropic`. */
  auth_header: AiAuthHeader;
  models: AiModel[];
  updated_at: number;
}

export interface AiProviderInput {
  /** null creates a provider. With an id, a null `api_key` keeps (or tests with) the saved key. */
  id: string | null;
  name: string;
  protocol: AiProtocol;
  base_url: string;
  /** null keeps the saved key (as HOST-08 does for passwords); "" clears it. Sent one way to Rust. */
  api_key: string | null;
  auth_header: AiAuthHeader;
  models: AiModel[];
}

/** AI-04 Test Connection, also used for the search provider test. */
export type AiTestFailure = "auth" | "network" | "unknown_model" | "invalid_url" | "other";

export interface AiTestResult {
  ok: boolean;
  failure: AiTestFailure | null;
  status: number | null;
  /** The provider's own message, when it sent one. */
  message: string | null;
}

export type SearchKind = "brave" | "tavily" | "searxng";

/** The backend of `web_search` (AI-14). */
export interface SearchProviderView {
  id: string;
  kind: SearchKind;
  /** The instance URL for searxng, null otherwise. */
  base_url: string | null;
  has_api_key: boolean;
  updated_at: number;
}

export interface SearchProviderInput {
  id: string | null;
  kind: SearchKind;
  base_url: string | null;
  /** null keeps the saved key; "" clears it. */
  api_key: string | null;
}

export interface AiModelRef {
  provider_id: string;
  model_id: string;
}

/** The synced `Settings.ai` (§5.1). */
export interface AiSettingsView {
  default_model: AiModelRef | null;
  /** The thinking level new conversations start with (AI-05); null is Default. */
  default_effort: AiEffort | null;
  search_provider_id: string | null;
  /** While no search provider is chosen, `web_search` uses the chat provider's own search (AI-14). */
  native_search: boolean;
  /** The built-in `hatoba` skill is offered (AI-34). */
  builtin_skill_enabled: boolean;
  /** Sent with every request, at most 32 KB of UTF-8 (AI-36). */
  custom_instructions: string;
  /** Messages go with the attached terminal's screen unless the panel's + menu turns it off for the conversation (AI-46). */
  send_terminal_screen: boolean;
  /** The judge of Auto mode (AI-47); Auto is unavailable while it is null. */
  judge_model: AiModelRef | null;
}

/** What `ai_settings_save` changes, like {@link SettingsPatch}; null clears a field that can be empty. */
export type AiSettingsPatch = Partial<AiSettingsView>;

/** §13.5, from the least strict: manual approval, Auto (AI-47), bypass. */
export type AiPermissionMode = "manual" | "auto" | "bypass";

/** What the judge of Auto mode decided (AI-47). It never rejects: a call runs without the card, or the card asks. */
export type AiJudgeDecision = "run" | "ask";

/**
 * One of the judge's answers (AI-47): `probability` is that of yes. The questions are `requested`, `forbidden`,
 * `changes_anything`, `destroys_data`, `touches_secrets`, `sends_data_out`, `security_change`, `runs_unseen_code`,
 * `sensitive_file` and `url_carries_data`, in that order; a call gets those of its tool.
 */
export interface AiJudgeAnswer {
  question: string;
  probability: number;
}

/**
 * The verdict of Auto mode's judge on a call (AI-47), stored with the call's result when Auto decided it. Display
 * only: it never reaches the model. `concerns`: the questions whose answers kept the call from running. `failure`:
 * why the judge gave no usable answer (`unavailable`, `too_long`, `timeout`, `unreachable`, `refused`, `malformed`,
 * `cancelled`, or a kind a newer version added); then the decision is `ask` and there are no answers.
 */
export interface AiAutoVerdict {
  provider_id: string;
  model_id: string;
  decision: AiJudgeDecision;
  answers: AiJudgeAnswer[];
  concerns: string[];
  failure: string | null;
}

/**
 * AI-47: the judge's verdict on one sample call of **Test** beside the judge in Settings → AI. `name` is the sample's
 * stable name (`disk_space`, `restart_requested`, `delete_unrequested`, `key_sent_out`, `restart_forbidden`,
 * `pipe_to_shell`, `key_via_shell`, `restart_requested_zh`, `delete_unrequested_ja`, `docs_page`, `token_in_url`,
 * `input_read`, `answer_unrequested`, or one a newer version added);
 * `expected` is what a good judge decides.
 */
export interface AiJudgeSampleOutcome {
  name: string;
  expected: AiJudgeDecision;
  verdict: AiAutoVerdict;
}

/**
 * An AI approval saved on this device (AI-19), never synced: `run_command` calls that start with the
 * prefix `value` on a host, every call of the tool `value` (`edit_file`, `write_file`, `send_input`) on
 * a host, or `fetch_url` calls of pages on the domain `value`. A host is a saved host's id, or the
 * quick-connect target (`user@host:port`) of a tab without one.
 */
export interface AiAllowance {
  kind: "command" | "tool" | "domain";
  host_id: string | null;
  target: string | null;
  value: string;
}

export interface AiConversationView {
  id: string;
  title: string;
  /** The host the conversation last worked on. */
  host_id: string | null;
  /** The quick-connect target (HOST-12) it last worked on while `host_id` is null (AI-09), the address in lowercase and without brackets. */
  quick_target: QuickTarget | null;
  /** AI-43: with neither, the name of the server a fork's copies came from, when the fork could not tell which it is; its next message on a tab of a host or target notes the move from it. */
  origin: string | null;
  pinned: boolean;
  /** entry_id where the context sent to the model starts (AI-21); earlier entries are outside it. */
  context_start: string | null;
  /** The thinking level its last message was sent with (AI-05); null is Default. */
  effort: AiEffort | null;
  created_at: number;
  updated_at: number;
  /** The newest entry or conversation change, for sorting history (AI-23). */
  last_activity: number;
}

export type AiFinish = "stop" | "tool_calls" | "length" | "refused";
export type AiToolStatus = "ok" | "error" | "rejected" | "cancelled";

export interface AiToolCall {
  id: string;
  name: string;
  /** JSON text. */
  arguments: string;
}

export interface AiUsage {
  input_tokens: number;
  output_tokens: number;
  estimated: boolean;
}

/** A stored conversation entry (§13.7), without the provider's raw message. */
export type AiEntryView =
  | { role: "user"; entry_id: string; created_at: number; text: string }
  | {
      role: "assistant";
      entry_id: string;
      created_at: number;
      provider_id: string;
      model_id: string;
      text: string;
      reasoning: string | null;
      tool_calls: AiToolCall[];
      finish: AiFinish;
      usage: AiUsage | null;
    }
  /** `auto`: what the judge decided when Auto mode decided the call (AI-47). */
  | { role: "tool"; entry_id: string; created_at: number; tool_call_id: string; status: AiToolStatus; content: string; auto: AiAutoVerdict | null }
  | { role: "summary"; entry_id: string; created_at: number; text: string };

export interface AiConversationDetail {
  conversation: AiConversationView;
  entries: AiEntryView[];
  /** A turn of this conversation is running in Rust (its events go to the channel that started it). */
  running: boolean;
  /** The changes of its `edit_file` and `write_file` calls that this device can revert, or has reverted (AI-41). */
  file_changes: AiFileChange[];
}

/** What a request is made with. The frontend owns the tab, so it says what the turn may act on. */
export interface AiTurnContext {
  provider_id: string;
  model_id: string;
  /** The thinking level (AI-05); null is Default. Rust sends the highest level the model offers that is not above it. */
  effort: AiEffort | null;
  /** The tab's host; the next message moves the conversation to it (AI-09). */
  host_id: string | null;
  /**
   * The tab's quick-connect target when it has no saved host (HOST-12): the next message moves the
   * conversation off its saved host, and the request names the target instead.
   */
  target: QuickTarget | null;
  /** A connected terminal tab is attached; false offers no terminal tools (AI-09). */
  tab: boolean;
  /** The tab's SSH session while it is connected; the system prompt states its server's identification string (§13.1). */
  session_id: string | null;
  /** MCP servers switched off for this conversation in **MCP Tools** (AI-30, P2). */
  disabled_mcp_servers: string[];
  /**
   * The conversation's permission mode on this device (AI-16), which a `permission_mode` note tells the
   * model of when it changes (§13.5).
   */
  permission_mode: AiPermissionMode;
  /**
   * The attached terminal's screen when the message is sent (AI-46), while the tab is connected and
   * **Send Terminal Screen** is on for the conversation. Rust shows it in a `terminal_state` note of a
   * stored message.
   */
  screen: AiScreen | null;
}

/** What a `terminal_state` note shows (AI-46, §13.3). */
export interface AiScreen {
  /** The screen's last lines, or the whole screen while `alternate`. */
  text: string;
  /** A full-screen program (vim, less, …) uses the alternate screen. */
  alternate: boolean;
}

export interface AiSendInput {
  /** null starts a new conversation, stored with this first message (AI-07). */
  conversation_id: string | null;
  text: string;
  context: AiTurnContext;
}

export interface AiSendStarted {
  conversation: AiConversationView;
  user_entry: AiEntryView;
}

/** `empty`: the response had neither text nor tool calls, such as a reply of reasoning only (§13.1). */
export type AiTurnEndReason = "completed" | "length" | "refused" | "empty" | "stopped" | "error";

/** Streamed on the channel of `ai_send` / `ai_retry` for the whole turn (§13.1). */
export type AiTurnEvent =
  /** A new conversation's turn is registered under this id, which it is stored under with its message; sent
   * before the message waits for the MCP servers (AI-45), so a stop can name the turn before `ai_send` returns. */
  | { kind: "opened"; conversation_id: string }
  | { kind: "request_started" }
  | { kind: "text"; delta: string }
  | { kind: "reasoning"; delta: string }
  | { kind: "tool_call"; id: string; name: string; arguments: string }
  | { kind: "usage"; input_tokens: number; output_tokens: number; estimated: boolean }
  /** An entry was stored: the assistant response, a tool result, or a cancelled result. */
  | { kind: "entry"; entry: AiEntryView }
  /** One response finished. With `tool_calls`, Rust waits for every call's result. A stored `summary`
   * entry means `context_start` moved to it (AI-21, or automatic compaction before a request, AI-22). */
  | { kind: "done"; finish: AiFinish }
  /** The tool of an `ai_tool_run` call ended, closing its exec channel; its result may still wait for an earlier call's (§13.1). */
  | { kind: "tool_ran"; tool_call_id: string }
  | { kind: "error"; status: number | null; message: string }
  /** The provider refused the thinking level: the request went again without it, at the model's default depth (AI-05). */
  | { kind: "effort_ignored" }
  | { kind: "turn_ended"; reason: AiTurnEndReason };

/** AI-42: a side question about a conversation, asked without stopping its turn. */
export interface AiSideQuestionInput {
  conversation_id: string;
  /** The panel's number for the question, which `ai_side_stop` names, so that a stop meant for it never stops a newer one. */
  question_id: number;
  /** The question, after the attachment blocks the input area adds (spec §13.3). */
  text: string;
  /** What the next message would be sent with; a running turn's own context wins. */
  context: AiTurnContext;
  /** The call whose approval card waits (AI-17), so its result in the request says so. */
  approval_call_id: string | null;
}

/** Streamed while a side question's answer comes (AI-42). */
export type AiSideEvent =
  | { kind: "reasoning"; delta: string }
  | { kind: "text"; delta: string }
  /** The answer called tools, so it was dropped and the question went again without tools: what streamed before is discarded. */
  | { kind: "restarted" };

/** The answer to a side question (AI-42), which nothing stores. */
export interface AiSideAnswer {
  text: string;
  finish: AiFinish;
}

/** A result the frontend produced: `read_terminal`, `send_input`, or a rejection (AI-17). */
export interface AiToolResultInput {
  status: AiToolStatus;
  content: string;
  /** AI-17 Edit: the arguments the user changed the call to, so the result tells the model. */
  edited_arguments: string | null;
  /**
   * AI-47: Auto mode decided the call: it ran on the judge's `run`, or its card showed the judge's verdict. Rust
   * then stores the verdict it remembers with the result.
   */
  judged: boolean;
}

/**
 * The change an `edit_file` or `write_file` call would make, for its approval card (AI-39, AI-40). The
 * texts show CRLF line breaks as `\n`; `crlf` says the write keeps them as CRLF.
 */
export interface AiFilePreview {
  /** The path the call names. */
  path: string;
  /** The file as Hatoba read it; null when it does not exist (the call creates it) or `error` is set. */
  before: string | null;
  /** The file after the call; null when `error` is set. */
  after: string | null;
  /** Why the call cannot apply: the error result running it would give the model. */
  error: string | null;
  /** Most of the file's line breaks are CRLF, and the write keeps them. */
  crlf: boolean;
}

/** A change an `edit_file` or `write_file` call made, kept on this device so the user can revert it (AI-41). */
export interface AiFileChange {
  /** The entry id of the response that made the call. */
  response_id: string;
  tool_call_id: string;
  /** The server the call wrote on, as `user@host:port`. */
  server: string;
  /** The file's path as the call resolved it. */
  path: string;
  /** The call created the file, so a revert deletes it. */
  created: boolean;
  state: AiFileChangeState;
}

/**
 * Where a call's change stands on this device (AI-41). `unfinished`: the write failed after it may have begun,
 * or its end was never recorded, so the file may be cut short; it can be reverted like a written one. `dropped`:
 * the 32 MB limit dropped what the file held before, so it can no longer be reverted.
 */
export type AiFileChangeState = "written" | "unfinished" | "reverted" | "dropped";

/**
 * Why a revert did not happen (AI-41). `detail` names the server for `other_server`, and what failed for
 * `unfinished` (when an error ended it rather than the time) and `failed`. `symbolic_link`: a link is in the place
 * of the file the call created, and neither it nor its target is deleted.
 */
export type AiRevertFailure = "disconnected" | "other_server" | "permission_denied" | "not_text" | "symbolic_link" | "timed_out" | "unfinished" | "failed";

/**
 * The outcome of a revert (AI-41). `reverted`: the file holds what it held before the call again, or no longer
 * exists when the call created it; `wrote` is false when it was so already. `changed`: it holds neither what the
 * call wrote nor what it held before, so nothing was written; the diff from `current` (null: it does not exist)
 * to `before` (null: the call created it, so Restore anyway deletes it) is what Restore anyway does. `later`: a
 * later call of the conversation wrote the file too.
 */
export type AiFileRevert =
  | { status: "reverted"; wrote: boolean }
  | { status: "changed"; path: string; current: string | null; before: string | null; later: boolean }
  | { status: "failed"; reason: AiRevertFailure; detail: string | null };

/**
 * What `ai_tool_run` stored: the call's result. `dropped_file_changes` (AI-41): keeping the snapshot of this call's
 * change dropped older ones over the size limit, of this or another conversation.
 */
export interface AiToolRun {
  entry: AiEntryView;
  dropped_file_changes: boolean;
}

// ───────────────────────── AI assistant: skills (§13.8) ─────────────────────────

export interface SkillFileView {
  /** Relative path with forward slashes, such as `references/nginx.md`. Never `SKILL.md`. */
  path: string;
  content: string;
}

export interface SkillView {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  /** Paths of the files besides SKILL.md. */
  files: string[];
  updated_at: number;
}

export interface SkillDetail {
  skill: SkillView;
  /** SKILL.md without its frontmatter. */
  body: string;
  files: SkillFileView[];
  /** Other frontmatter fields, kept for export. They change nothing, so `allowed-tools` does not change approvals (AI-27). */
  frontmatter_keys: string[];
}

/** The built-in `hatoba` skill (AI-34), read-only. */
export interface BuiltinSkillView {
  name: string;
  description: string;
  /** `Settings.ai.builtin_skill_enabled`. */
  enabled: boolean;
  /** SKILL.md without its frontmatter, with the app's version filled in. */
  body: string;
  /** The files besides SKILL.md, sorted by path. */
  files: SkillFileView[];
}

export interface SkillInput {
  /** null creates a skill. */
  id: string | null;
  name: string;
  description: string;
  enabled: boolean;
  body: string;
  /** Every file besides SKILL.md; a saved file left out is deleted. */
  files: SkillFileView[];
}

/** Why a skill cannot be imported or saved (AI-27). */
export type SkillIssue =
  | { kind: "missing_skill_md" }
  | { kind: "invalid_frontmatter"; detail: string }
  | { kind: "invalid_name"; name: string }
  | { kind: "missing_description" }
  | { kind: "description_too_long"; chars: number }
  | { kind: "file_too_large"; path: string; size: number }
  | { kind: "unsafe_path"; path: string }
  | { kind: "too_many_files"; count: number }
  /** More than 5 MB in total. */
  | { kind: "too_large"; bytes: number };

/** What an import would save, shown before saving (AI-27). Importable when `issues` is empty. */
export interface SkillImportPreview {
  /** null when SKILL.md is missing or unreadable. */
  name: string | null;
  description: string | null;
  body: string | null;
  files: SkillFileView[];
  frontmatter_keys: string[];
  /** Files that are not UTF-8 text, which the import skips. */
  skipped: string[];
  issues: SkillIssue[];
  /** A saved skill with the same name, which the user may replace (or rename the new one). */
  existing_id: string | null;
  /** The name is the built-in skill's (AI-34): the skill can be imported only under another. */
  reserved_name: boolean;
}

// ───────────────────────── AI assistant: MCP servers (§13.9) ─────────────────────────

/** Environment and header values never reach the WebView, only their names (AI-29). */
export type McpTransportView =
  | { kind: "stdio"; command: string; args: string[]; env_keys: string[] }
  | { kind: "http"; url: string; header_keys: string[] };

export interface McpServerView {
  id: string;
  name: string;
  transport: McpTransportView;
  /** Ask even in bypass mode. Synced (AI-31). */
  always_ask: boolean;
  /** Enabled on this device; a server from another device starts enabled only for `http` (AI-29). */
  enabled: boolean;
  /** On this device, every tool of the server runs without asking in manual mode (AI-31). */
  always_allow: boolean;
  /** On this device, these tools (the server's own names) run without asking in manual mode. */
  always_allow_tools: string[];
  updated_at: number;
}

export interface McpSecretInput {
  key: string;
  /** null keeps the saved value for this key. Sent one way to Rust. */
  value: string | null;
}

export type McpTransportInput =
  | { kind: "stdio"; command: string; args: string[]; env: McpSecretInput[] }
  | { kind: "http"; url: string; headers: McpSecretInput[] };

export interface McpServerInput {
  /** null creates a server. */
  id: string | null;
  name: string;
  transport: McpTransportInput;
  always_ask: boolean;
}

export type McpServerState = "stopped" | "starting" | "running" | "failed";

/** Shown on the approval card; never changes whether a call asks (AI-31). */
export interface McpToolAnnotations {
  title: string | null;
  read_only_hint: boolean | null;
  destructive_hint: boolean | null;
  idempotent_hint: boolean | null;
  open_world_hint: boolean | null;
}

export interface McpToolView {
  /** The name offered to the model, `mcp__<server>__<tool>` after cleaning (AI-30). */
  name: string;
  /** The server's own tool name. */
  tool: string;
  description: string;
  annotations: McpToolAnnotations;
  /** Always allow on this device (per tool, or because the whole server is). */
  always_allow: boolean;
}

export interface McpServerStatus {
  server_id: string;
  state: McpServerState;
  error: string | null;
  /** The last stderr lines of a `stdio` server, kept in memory only (AI-32). */
  stderr: string[];
  /** The tools from the last successful listing. */
  tools: McpToolView[];
  /** The prompts from the last successful listing, which the panel's `/` menu offers (AI-50). */
  prompts: McpPromptView[];
  /** The resources from the last successful listing, which the panel's `@` menu offers (AI-51). */
  resources: McpResourceView[];
}

/** A server's prompt (AI-50): the `/` menu offers it as `/mcp__<server>__<prompt>`, named across the conversation's servers (`commandNames`). */
export interface McpPromptView {
  /** The prompt's name on its server. */
  name: string;
  title: string | null;
  description: string;
  arguments: McpPromptArgumentView[];
}

export interface McpPromptArgumentView {
  name: string;
  description: string;
  required: boolean;
}

/** A server's resource (AI-51), which the `@` menu offers. */
export interface McpResourceView {
  uri: string;
  name: string;
  title: string | null;
  description: string;
  mime_type: string | null;
}

/** A value for one argument of an MCP prompt (AI-50). */
export interface McpPromptArgumentInput {
  name: string;
  value: string;
}

/** The MCP tool behind a name the model called, for the approval card (AI-31). */
export interface McpToolInfo {
  server_id: string;
  server_name: string;
  always_ask: boolean;
  tool: McpToolView;
}

/** What the approval card of a `call_mcp_tool` call needs (AI-45). */
export interface McpProxyInfo {
  /** The tool the call names, as `mcp_tool_info` gives a direct call's; null when the conversation has no tool of that name or its server was deleted. */
  tool: McpToolInfo | null;
  /** False when the call cannot run: it then gets its error result without asking. */
  runs: boolean;
}

export interface McpImportPreview {
  servers: { name: string; transport: McpTransportView; exists: boolean }[];
  /** Entries that cannot be imported, such as an `sse` server, with the reason. */
  skipped: { name: string; reason: string }[];
}

// ───────────────────────── AI assistant: history search (AI-24) ─────────────────────────

export interface AiSearchHit {
  conversation_id: string;
  /** The first matching entry, or null when only the title matched. */
  entry_id: string | null;
  /** Text around the first match. */
  snippet: string;
}

/** Why a file dropped on the AI panel is not attached (AI-35): the panel's own reasons. */
export type DroppedFileRefusal = "image" | "too_large" | "binary" | "unreadable";

/** A text file dropped on the AI panel in the desktop app (AI-35), named by its base name only, never its path. */
export type DroppedFile = { status: "ok"; name: string; text: string } | { status: "refused"; name: string; reason: DroppedFileRefusal };

/** One entry of a directory on the tab's host, for the panel's `@` menu (AI-51). */
export interface AiHostEntry {
  name: string;
  /** A directory, or a symbolic link to one. */
  dir: boolean;
}

/** Why a path on the tab's host cannot be listed or attached (AI-51). `wrong_kind`: a directory to read, or a file to list. */
export type AiHostRefusal = "disconnected" | "not_found" | "wrong_kind" | "not_regular" | "permission_denied" | "too_large" | "binary" | "failed";

/** A directory on the tab's host (AI-51): its entries sorted by name, or why it cannot be listed. `path` as the host resolved it. */
export type AiHostListing = { status: "ok"; path: string; entries: AiHostEntry[]; cut: boolean } | { status: "refused"; reason: AiHostRefusal };

/** A text file on the tab's host for an `@` mention (AI-51), or why it cannot be attached. */
export type AiHostFile = { status: "ok"; path: string; text: string } | { status: "refused"; reason: AiHostRefusal };

/** An enabled skill the panel's `/` menu offers as a command (AI-50), the built-in one included. */
export interface AiSkillCommand {
  name: string;
  description: string;
}

// ───────────────────────── Events (§10.2) ─────────────────────────

export interface VaultLockedEvent {
  reason: "manual" | "idle" | "sleep";
}

/** Whether the Linux main window shows its painted chrome (off maximized, fullscreen, tiled, or without a compositor). */
export interface WindowChromeEvent {
  floating: boolean;
}

export interface EventMap {
  "vault://locked": VaultLockedEvent;
  "sync://status": SyncStatus;
  "ssh://hostkey-prompt": HostKeyPrompt;
  "ssh://auth-prompt": AuthPrompt;
  "ssh://state": SessionStateEvent;
  "transfer://progress": TransferProgressEvent;
  "ssh://forward": ForwardStatusEvent;
  /** An MCP server started, stopped, failed or relisted its tools (AI-32). */
  "ai://mcp-status": McpServerStatus;
  "window://chrome": WindowChromeEvent;
  /** Another launch handed its command line to this window (HOST-13); `launch_take` returns it. */
  "app://launch": null;
  /** WIN-10: the close button asks whether to hide the window to the tray or quit. */
  "window://close-prompt": null;
  /** WIN-10: the tray menu's **Update to vX.Y.Z** was clicked. */
  "tray://update": null;
}
