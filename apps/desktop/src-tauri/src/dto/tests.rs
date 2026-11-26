use super::{AiPermissionMode, LocalPrefs};

#[test]
fn the_update_check_is_on_unless_turned_off() {
    let prefs = LocalPrefs::from_stored(r#"{"language":"ja","host_probe":false}"#);
    assert!(prefs.auto_update_check);
    assert!(!prefs.host_probe);
    let prefs = LocalPrefs::from_stored(r#"{"auto_update_check":false}"#);
    assert!(!prefs.auto_update_check);
}

#[test]
fn device_security_prefs_have_defaults() {
    let prefs = LocalPrefs::from_stored(r#"{"language":"ja"}"#);
    assert_eq!(prefs.auto_lock_minutes, 15);
    assert!(prefs.lock_on_sleep && !prefs.lock_disconnects_sessions);
    assert_eq!(prefs.clipboard_clear_secs, 90);
    let prefs = LocalPrefs::from_stored(
        r#"{"auto_lock_minutes":0,"lock_on_sleep":false,"clipboard_clear_secs":0}"#,
    );
    assert_eq!(
        (prefs.auto_lock_minutes, prefs.clipboard_clear_secs),
        (0, 0)
    );
    assert!(!prefs.lock_on_sleep);
}

#[test]
fn ai_prefs_default_to_manual_and_unknown_values_fall_back() {
    let prefs = LocalPrefs::from_stored(r#"{"language":"ja"}"#);
    assert_eq!(prefs.ai_permission_mode, AiPermissionMode::Manual);
    assert!(!prefs.ai_bypass_confirmed && !prefs.ai_panel_open);
    assert!(prefs.ai_notifications);
    assert_eq!(prefs.ai_panel_width, 380);

    // The tool call limit an older version stored is ignored.
    let prefs = LocalPrefs::from_stored(
        r#"{"ai_permission_mode":"bypass","ai_bypass_confirmed":true,"ai_tool_call_limit":40,
            "ai_panel_open":true,"ai_panel_width":520,"ai_notifications":false}"#,
    );
    assert_eq!(prefs.ai_permission_mode, AiPermissionMode::Bypass);
    assert!(prefs.ai_bypass_confirmed && prefs.ai_panel_open);
    assert!(!prefs.ai_notifications);
    assert_eq!(prefs.ai_panel_width, 520);

    // AI-47: Auto is a mode of its own.
    let prefs = LocalPrefs::from_stored(r#"{"ai_permission_mode":"auto"}"#);
    assert_eq!(prefs.ai_permission_mode, AiPermissionMode::Auto);

    // A mode a newer build added, or a broken value, is read as the default.
    let prefs = LocalPrefs::from_stored(
        r#"{"ai_permission_mode":"ask_twice","ai_panel_width":-1,"host_probe":false}"#,
    );
    assert_eq!(prefs.ai_permission_mode, AiPermissionMode::Manual);
    assert_eq!(prefs.ai_panel_width, 380);
    assert!(!prefs.host_probe);
}
