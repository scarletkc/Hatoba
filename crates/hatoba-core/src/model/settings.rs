use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use super::ai::{AiEffort, effort_serde};

/// Fixed id of the single [`Settings`] item.
pub const SETTINGS_ID: &str = "settings";

/// Terminal colour theme preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Light,
    Dark,
    /// Follow the OS. Also what an unknown future value decodes to (`serde(other)` must be last).
    #[default]
    #[serde(other)]
    System,
}

/// Terminal cursor shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorStyle {
    /// Vertical bar.
    Bar,
    Underline,
    /// Filled block. Also what an unknown future value decodes to.
    #[default]
    #[serde(other)]
    Block,
}

/// What right-clicking in the terminal does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightClick {
    /// Open a context menu.
    Menu,
    /// Copy the selection if there is one, otherwise paste (the PuTTY habit). Also what an
    /// unknown future value decodes to.
    #[default]
    #[serde(other)]
    CopyPaste,
}

impl RightClick {
    /// What an unset field reads as: the PuTTY habit of copying and pasting, except on macOS,
    /// where a secondary click that pastes surprises users and a context menu is expected.
    ///
    /// This is fixed when the crate is built: `hatoba-core` takes no dependency on the desktop
    /// app, and the target OS of a build is the OS the vault is opened on.
    #[must_use]
    pub const fn platform_default() -> Self {
        if cfg!(target_os = "macos") {
            Self::Menu
        } else {
            Self::CopyPaste
        }
    }
}

/// Terminal appearance and behaviour settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct TerminalSettings {
    pub font_family: String,
    /// Font size in points.
    pub font_size: u16,
    #[zeroize(skip)]
    pub theme: ThemeMode,
    /// Id of the colour scheme used while the terminal is dark. `None` until one is picked,
    /// which reads as the app's own dark scheme; the desktop app owns the ids and reads one it
    /// does not know the same way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dark_scheme: Option<String>,
    /// Id of the colour scheme used while the terminal is light, like [`Self::dark_scheme`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_scheme: Option<String>,
    #[zeroize(skip)]
    pub cursor_style: CursorStyle,
    /// Scrollback lines.
    pub scrollback: u32,
    /// Right-click behaviour. `None` until a value is recorded, which items written by builds
    /// that kept it device-local never have, and which reads as [`RightClick::platform_default`];
    /// read it with [`Self::right_click`].
    #[zeroize(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_click: Option<RightClick>,
    /// Ask before pasting text with line breaks. `None` until a value is recorded; read it with
    /// [`Self::confirm_multiline_paste`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm_multiline_paste: Option<bool>,
    /// Show the rest of a matching command dimmed after the cursor at a shell prompt (TERM-14).
    /// `None` until a value is recorded; read it with [`Self::suggest_commands`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggest_commands: Option<bool>,
}

/// The terminal font a new vault starts with. Cascadia Mono comes with Windows (WIN-02), and
/// macOS has no family of that name; Menlo is on every macOS, while "SF Mono" is not a family
/// that WebKit can use. The value syncs like any other, and a device that lacks the font falls
/// back to its own monospace stack.
pub const DEFAULT_FONT_FAMILY: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "Cascadia Mono"
};

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_family: DEFAULT_FONT_FAMILY.to_owned(),
            font_size: 13,
            theme: ThemeMode::Dark,
            dark_scheme: None,
            light_scheme: None,
            cursor_style: CursorStyle::Block,
            scrollback: 10_000,
            right_click: None,
            confirm_multiline_paste: None,
            suggest_commands: None,
        }
    }
}

impl TerminalSettings {
    /// The right-click behaviour; [`RightClick::platform_default`] while unset.
    #[must_use]
    pub fn right_click(&self) -> RightClick {
        self.right_click.unwrap_or(RightClick::platform_default())
    }

    /// Whether multi-line pastes need confirming; `true` while unset.
    #[must_use]
    pub fn confirm_multiline_paste(&self) -> bool {
        self.confirm_multiline_paste.unwrap_or(true)
    }

    /// Whether a matching command shows dimmed after the cursor; `true` while unset.
    #[must_use]
    pub fn suggest_commands(&self) -> bool {
        self.suggest_commands.unwrap_or(true)
    }

    /// Records the right-click behaviour. An unset field stays unset when `value` is the default.
    pub fn set_right_click(&mut self, value: RightClick) {
        record(&mut self.right_click, value, RightClick::platform_default());
    }

    /// Records whether multi-line pastes need confirming. An unset field stays unset when `value`
    /// is the default.
    pub fn set_confirm_multiline_paste(&mut self, value: bool) {
        record(&mut self.confirm_multiline_paste, value, true);
    }

    /// Records whether a matching command shows dimmed after the cursor. An unset field stays
    /// unset when `value` is the default.
    pub fn set_suggest_commands(&mut self, value: bool) {
        record(&mut self.suggest_commands, value, true);
    }
}

/// Stores `value` in a field that reads as `default` while unset, except that an unset field
/// stays unset when `value` is that default: the user has not chosen anything yet, and leaving
/// it open lets a device carrying over a different device-local value fill it in.
fn record<T: PartialEq>(field: &mut Option<T>, value: T, default: T) {
    if field.is_some() || value != default {
        *field = Some(value);
    }
}

/// A model picked by provider and model id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct AiModelRef {
    /// Id of the [`AiProvider`] item.
    pub provider_id: String,
    /// Model id within that provider.
    pub model_id: String,
}

/// AI assistant settings that sync (spec §5.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct AiSettings {
    /// The model new conversations start with.
    pub default_model: Option<AiModelRef>,
    /// The thinking level new conversations start with (AI-05); `None` (absent) is Default.
    #[zeroize(skip)]
    #[serde(
        deserialize_with = "effort_serde::level",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_effort: Option<AiEffort>,
    /// Id of the [`SearchProvider`] item behind `web_search`.
    pub search_provider_id: Option<String>,
    /// While no search provider is chosen, `web_search` uses the chat provider's own search
    /// (AI-14). On by default, also for settings written before it existed.
    pub native_search: bool,
    /// The built-in `hatoba` skill is offered (AI-34). On by default, also for settings written
    /// before it existed.
    pub builtin_skill_enabled: bool,
    /// The user's instructions, sent with every request (AI-36), at most
    /// [`MAX_CUSTOM_INSTRUCTIONS_BYTES`] bytes of UTF-8. Absent while empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub custom_instructions: String,
    /// Messages go with the attached terminal's screen unless the panel's + menu turns it off for
    /// the conversation (AI-46). On by default, also for settings written before it existed.
    pub send_terminal_screen: bool,
    /// The judge of Auto mode (AI-47): a model of a System One provider, or of a provider that
    /// chats. `None` (absent) until the user chooses one, and Auto is unavailable until then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_model: Option<AiModelRef>,
    /// Conversations start with the memories, and the assistant can save and delete them (AI-53).
    /// On by default, also for settings written before it existed.
    pub memory: bool,
    /// The size of the AI panel's conversation and input text, in px (TERM-13), from
    /// [`AI_TEXT_SIZE_MIN`] to [`AI_TEXT_SIZE_MAX`]. [`AI_TEXT_SIZE_DEFAULT`] for settings written
    /// before it existed.
    pub text_size: u32,
}

/// The smallest [`AiSettings::text_size`], in px (TERM-13).
pub const AI_TEXT_SIZE_MIN: u32 = 11;
/// The largest [`AiSettings::text_size`], in px.
pub const AI_TEXT_SIZE_MAX: u32 = 24;
/// The [`AiSettings::text_size`] of settings that have none: the panel's design size, in px.
pub const AI_TEXT_SIZE_DEFAULT: u32 = 13;

/// The longest [`AiSettings::custom_instructions`], in bytes of UTF-8 (AI-36). Plain text this long
/// fits the settings item; text whose JSON is much longer (many quotes or backslashes) may not,
/// and saving settings that do not fit [`MAX_ITEM_PLAINTEXT_BYTES`] is refused.
pub const MAX_CUSTOM_INSTRUCTIONS_BYTES: usize = 32 * 1024;

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            default_model: None,
            default_effort: None,
            search_provider_id: None,
            native_search: true,
            builtin_skill_enabled: true,
            custom_instructions: String::new(),
            send_terminal_screen: true,
            judge_model: None,
            memory: true,
            text_size: AI_TEXT_SIZE_DEFAULT,
        }
    }
}

/// The user's synced settings. There is exactly one, with the fixed id [`SETTINGS_ID`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct Settings {
    pub terminal: TerminalSettings,
    pub ai: AiSettings,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}
