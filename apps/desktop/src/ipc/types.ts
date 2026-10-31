/**
 * IPC contract between the WebView and the Rust backend (spec §10).
 *
 * The DTOs are the Rust types in `apps/desktop/src-tauri/src/dto.rs` and, for the AI assistant,
 * `crates/hatoba-assistant/src/dto.rs`, which tauri-specta exports to `bindings.ts`; this module
 * re-exports them. DTOs never carry secrets: the WebView only ever sees
 * metadata such as `has_password`, never the password itself (spec §3.2 / §10.1).
 *
 * Timestamps are Unix epoch milliseconds.
 */
import type {
  AppInfo as RustAppInfo,
  AuthPrompt,
  AuthPromptClosed,
  CliActivityEvent,
  ClosePromptEvent,
  ConnectionsEvent,
  ForwardsLostEvent,
  HostKeyPrompt,
  LaunchEvent,
  McpServerStatus,
  NotificationTarget,
  RemoteEditEvent,
  RemoteEditQuitEvent,
  SessionStateEvent,
  SyncStatus,
  TransferProgressEvent,
  TrayUpdateEvent,
  VaultLockedEvent,
  WindowChromeEvent,
  ZoomView,
} from "./bindings";

export type * from "./bindings";

/** `web` stands in for the platform until `app_info` answers; Rust never sends it. */
export type AppInfo = Omit<RustAppInfo, "platform"> & { platform: RustAppInfo["platform"] | "web" };

/**
 * Terminal output arrives on a per-session Tauri Channel as raw bytes (ArrayBuffer), §10.3.
 * Byte 0 is the frame tag: 0 = data, 1 = closed (UTF-8 reason follows), 2 = error (UTF-8 message follows).
 */
export const FRAME_DATA = 0;
export const FRAME_CLOSED = 1;
export const FRAME_ERROR = 2;
/** The backend session id, sent before connecting, so a tab can answer its prompts if it closes. */
export const FRAME_SESSION = 3;

/** The events Rust emits (§10.2), by name. */
export interface EventMap {
  "vault://locked": VaultLockedEvent;
  "sync://status": SyncStatus;
  "ssh://hostkey-prompt": HostKeyPrompt;
  "ssh://auth-prompt": AuthPrompt;
  "ssh://auth-prompt-closed": AuthPromptClosed;
  "ssh://state": SessionStateEvent;
  "transfer://progress": TransferProgressEvent;
  "sftp://edit": RemoteEditEvent;
  /** SFTP-05: Hatoba is quitting with files being edited, and waits for `sftp_edit_quit`. */
  "sftp://quit": RemoteEditQuitEvent;
  /** SSH-16: a host's connection opened or closed, or its forwards changed. */
  "ssh://connections": ConnectionsEvent;
  "ssh://forwards-lost": ForwardsLostEvent;
  /** An MCP server started, stopped, failed or relisted its tools (AI-32). */
  "ai://mcp-status": McpServerStatus;
  "window://chrome": WindowChromeEvent;
  /** TERM-13: the window moved to a screen that changes the interface zoom. */
  "window://zoom": ZoomView;
  /** Another launch handed its command line to this window (HOST-13); `launch_take` returns it. */
  "app://launch": LaunchEvent;
  /** WIN-10: the close button asks whether to hide the window to the tray or quit. */
  "window://close-prompt": ClosePromptEvent;
  /** AI-07: a click on a notification brought the window back; this is what the notification was about. */
  "app://notification-click": NotificationTarget;
  /** WIN-10: the tray menu's **Update to vX.Y.Z** was clicked. */
  "tray://update": TrayUpdateEvent;
  /** HOST-15: what other programs did through the `hatoba` command changed. */
  "cli://activity": CliActivityEvent;
}
