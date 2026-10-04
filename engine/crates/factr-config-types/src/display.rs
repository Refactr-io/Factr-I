//! `[display]` section of the config. The terminal UI is gone; what remains is
//! what the engine still reads. Old `[display]` keys are accepted and ignored.

use crate::ReasoningDisplayMode;
use serde::{Deserialize, Serialize};

/// Display/UI configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    /// Request the model's reasoning from the provider (default: true)
    pub show_thinking: bool,
    /// How to render reasoning content into history (off/full/current).
    /// When unset, falls back to `show_thinking` (true => full, false => off).
    #[serde(
        default,
        deserialize_with = "crate::serde_lenient::lenient_optional_enum"
    )]
    pub(crate) reasoning_display: Option<ReasoningDisplayMode>,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            show_thinking: true,
            reasoning_display: Some(ReasoningDisplayMode::Full),
        }
    }
}

impl DisplayConfig {
    /// Resolve the effective reasoning display mode. Prefers the explicit
    /// `reasoning_display` field, falling back to the legacy `show_thinking`
    /// boolean (true => Full, false => Off) when unset.
    pub fn reasoning_display(&self) -> ReasoningDisplayMode {
        self.reasoning_display.unwrap_or(if self.show_thinking {
            ReasoningDisplayMode::Full
        } else {
            ReasoningDisplayMode::Off
        })
    }

    /// Whether the user explicitly chose a reasoning display mode, as opposed
    /// to inheriting the `show_thinking` fallback.
    pub fn has_explicit_reasoning_display(&self) -> bool {
        self.reasoning_display.is_some()
    }

    /// Set the reasoning display mode and keep `show_thinking` in sync so the
    /// provider request path (which keys off `show_thinking`) requests
    /// reasoning whenever any display mode is active.
    pub fn set_reasoning_display(&mut self, mode: ReasoningDisplayMode) {
        self.reasoning_display = Some(mode);
        self.show_thinking = !matches!(mode, ReasoningDisplayMode::Off);
    }
}

#[cfg(test)]
mod tests {
    use super::DisplayConfig;
    use crate::ReasoningDisplayMode;

    #[test]
    fn thinking_is_requested_by_default() {
        assert!(DisplayConfig::default().show_thinking);
        assert_eq!(
            DisplayConfig::default().reasoning_display(),
            ReasoningDisplayMode::Full
        );
        let missing: DisplayConfig = serde_json::from_str("{}").expect("display config");
        assert!(missing.show_thinking);
    }

    #[test]
    fn removed_terminal_ui_keys_still_parse_and_are_ignored() {
        let old: DisplayConfig = serde_json::from_str(
            r#"{"diff_mode":"inline","pin_todos":false,"centered":true,"reasoning_display":"nope","native_scrollbars":{"chat":false},"show_thinking":false,"debug_socket":true}"#,
        )
        .expect("old display config");
        // An unrecognized reasoning_display degrades to the show_thinking fallback.
        assert!(!old.show_thinking);
        assert!(!old.has_explicit_reasoning_display());
        assert_eq!(old.reasoning_display(), ReasoningDisplayMode::Off);
    }

    #[test]
    fn explicit_reasoning_display_keeps_show_thinking_in_sync() {
        let mut display = DisplayConfig::default();
        display.set_reasoning_display(ReasoningDisplayMode::Current);
        assert!(display.has_explicit_reasoning_display());
        assert_eq!(display.reasoning_display(), ReasoningDisplayMode::Current);
        assert!(display.show_thinking);
        display.set_reasoning_display(ReasoningDisplayMode::Off);
        assert!(!display.show_thinking);
    }
}
