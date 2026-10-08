/**
 * Compile-time check that the hand-written IPC contract (`types.ts`) matches the Rust DTOs exported
 * by tauri-specta (`bindings.ts`, regenerate with `cargo test -p hatoba-desktop export_bindings`).
 * Responses/events must be assignable Rust → UI; inputs UI → Rust. `tsc` fails on any drift.
 */
import type * as Rust from "./bindings";
import type * as Ui from "./types";

type Assignable<A, B> = [A] extends [B] ? true : false;
type Check<T extends true> = T;

/** Rust → UI: command results and events. */
export type ResponsesMatch = [
  Check<Assignable<Rust.AppError, Ui.AppError>>,
  Check<Assignable<Rust.AppInfo, Ui.AppInfo>>,
  Check<Assignable<Rust.UpdateCheck, Ui.UpdateCheck>>,
  Check<Assignable<Rust.VaultStatus, Ui.VaultStatus>>,
  Check<Assignable<Rust.HostView, Ui.HostView>>,
  Check<Assignable<Rust.GroupView, Ui.GroupView>>,
  Check<Assignable<Rust.TagCount, Ui.TagCount>>,
  Check<Assignable<Rust.SshConfigCandidate, Ui.SshConfigCandidate>>,
  Check<Assignable<Rust.ImportResult, Ui.ImportResult>>,
  Check<Assignable<Rust.ProbeResult, Ui.ProbeResult>>,
  Check<Assignable<Rust.KeyView, Ui.KeyView>>,
  Check<Assignable<Rust.SessionStateEvent, Ui.SessionStateEvent>>,
  Check<Assignable<Rust.HostKeyPrompt, Ui.HostKeyPrompt>>,
  Check<Assignable<Rust.AuthPrompt, Ui.AuthPrompt>>,
  Check<Assignable<Rust.TestResult, Ui.TestResult>>,
  Check<Assignable<Rust.FileEntry, Ui.FileEntry>>,
  Check<Assignable<Rust.TransferProgressEvent, Ui.TransferProgressEvent>>,
  Check<Assignable<Rust.SyncStatus, Ui.SyncStatus>>,
  Check<Assignable<Rust.SyncTestResult, Ui.SyncTestResult>>,
  Check<Assignable<Rust.DeviceView, Ui.DeviceView>>,
  Check<Assignable<Rust.ConflictView, Ui.ConflictView>>,
  Check<Assignable<Rust.SettingsView, Ui.SettingsView>>,
  Check<Assignable<Rust.LocalPrefs, Ui.LocalPrefs>>,
  Check<Assignable<Rust.VaultLockedEvent, Ui.VaultLockedEvent>>,
  Check<Assignable<Rust.ForwardView, Ui.ForwardView>>,
  Check<Assignable<Rust.ForwardStatusEvent, Ui.ForwardStatusEvent>>,
];

/** UI → Rust: command arguments. */
export type InputsMatch = [
  Check<Assignable<Ui.HostInput, Rust.HostInput>>,
  Check<Assignable<Ui.GroupInput, Rust.GroupInput>>,
  Check<Assignable<Ui.KeyImportInput, Rust.KeyImportInput>>,
  Check<Assignable<Ui.KeyGenerateInput, Rust.KeyGenerateInput>>,
  Check<Assignable<Ui.ConnectOptions, Rust.ConnectOptions>>,
  Check<Assignable<Ui.SyncConfigInput, Rust.SyncConfigInput>>,
  Check<Assignable<Ui.SettingsView, Rust.SettingsView>>,
  Check<Assignable<Ui.LocalPrefs, Rust.LocalPrefs>>,
  Check<Assignable<Ui.ForwardInput, Rust.ForwardInput>>,
];
