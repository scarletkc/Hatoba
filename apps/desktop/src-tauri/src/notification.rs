//! System notifications (AI-07). The WebView decides when one shows: only while the window is
//! not focused, which includes hidden to the tray (WIN-10). A click on one brings the window back
//! and tells the WebView what it was about.

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use tauri_specta::Event;

use crate::dto::{NotificationInput, NotificationTarget};
use crate::platform;

/// The key of the notification's extra data that carries its target.
const TARGET: &str = "target";

/// Reports clicks on notifications. Runs before the first one shows: the plugin reports clicks
/// only on notifications shown while a handler exists.
pub fn init(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.notification().on_action(move |action| {
        if action.action_id() != "tap" {
            return;
        }
        let target = action
            .notification()
            .and_then(|n| n.extra().get(TARGET))
            .and_then(|v| serde_json::from_value::<NotificationTarget>(v.clone()).ok())
            .unwrap_or_default();
        platform::window::reopen_main(&handle);
        let _ = target.emit(&handle);
    });
}

/// Shows a notification. One that cannot show (no notification server, notifications turned off
/// in the system) is only logged.
pub fn show(app: &AppHandle, input: NotificationInput) {
    let shown = app
        .notification()
        .builder()
        .title(input.title)
        .body(input.body)
        .extra(TARGET, &input.target)
        .show();
    if let Err(e) = shown {
        tracing::warn!("notification not shown: {e}");
    }
}
