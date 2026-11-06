//! Selected CLI strategies. Identity selects a parser policy, never an action.
//! Every policy still requires current source geometry and input evidence.

use serde::{Deserialize, Serialize};

#[path = "claude_chrome.rs"]
mod claude_chrome;

type ChromeFilter = fn(
    &bks_terminal::TerminalSnapshot,
    &bks_terminal::ReadingDocument,
    Option<&crate::tui_projection::Composer>,
    i32,
    u32,
    &str,
) -> bool;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    #[default]
    Auto,
    Shell,
    Universal,
    Codex,
    Claude,
    OpenCode,
    Kimi,
    Gemini,
}

impl AdapterKind {
    pub const ALL: [Self; 8] = [
        Self::Auto,
        Self::Shell,
        Self::Universal,
        Self::Codex,
        Self::Claude,
        Self::OpenCode,
        Self::Kimi,
        Self::Gemini,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Автоматически",
            Self::Shell => "Оболочка",
            Self::Universal => "Универсальный",
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::OpenCode => "OpenCode",
            Self::Kimi => "Kimi CLI",
            Self::Gemini => "Gemini CLI",
        }
    }

    pub(crate) const fn policy(self) -> Policy {
        match self {
            Self::Auto | Self::Shell | Self::Universal => universal::POLICY,
            Self::Codex => crate::codex::POLICY,
            Self::Claude => claude::POLICY,
            Self::OpenCode => crate::opencode::POLICY,
            Self::Kimi => kimi::POLICY,
            Self::Gemini => gemini::POLICY,
        }
    }

    pub fn encode_key(
        self,
        key: crate::terminal_input::SpecialKey,
        shift: bool,
        alt: bool,
        control: bool,
        application_cursor: bool,
    ) -> Vec<u8> {
        if self == Self::Codex {
            if let Some(bytes) = crate::codex::modified_enter(key, shift, alt, control) {
                return bytes;
            }
        }
        if self == Self::OpenCode {
            if let Some(bytes) = crate::opencode::modified_enter(key, shift, alt, control) {
                return bytes;
            }
        }
        crate::terminal_input::encode_key(key, shift, alt, control, application_cursor)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ComposerStrategy {
    Universal,
    Padded,
    Ruled,
    Gutter,
    Bounded,
}

#[derive(Clone, Copy)]
pub(crate) struct Semantics {
    pub panel_responses: bool,
    pub recorded_duration: bool,
    pub bullet_roles: bool,
    pub chrome: Option<ChromeFilter>,
    /// Recorded renderer text; shared code additionally proves live bounds
    /// and the separate indicator/label styles before using it as a boundary.
    pub bounded_busy: Option<fn(&str) -> bool>,
}

#[derive(Clone, Copy)]
pub(crate) struct Policy {
    pub composer: ComposerStrategy,
    pub semantics: Semantics,
}

mod universal {
    use super::*;
    pub(super) const POLICY: Policy = Policy {
        composer: ComposerStrategy::Universal,
        semantics: Semantics {
            panel_responses: true,
            recorded_duration: true,
            bullet_roles: true,
            chrome: None,
            bounded_busy: None,
        },
    };
}

mod claude {
    use super::*;
    pub(super) const POLICY: Policy = Policy {
        composer: ComposerStrategy::Ruled,
        semantics: Semantics {
            panel_responses: false,
            recorded_duration: false,
            bullet_roles: true,
            chrome: Some(claude_chrome::metadata),
            bounded_busy: None,
        },
    };
}

mod kimi {
    use super::*;
    pub(super) const POLICY: Policy = Policy {
        composer: ComposerStrategy::Bounded,
        semantics: Semantics {
            panel_responses: false,
            recorded_duration: false,
            // Kimi Code 2.4.1 uses a plain ● for assistant prose after its
            // accepted ❯ echo. The shared role parser still requires that
            // source boundary (or observed busy control), and completion
            // requires the separately proven working boundary to disappear.
            bullet_roles: true,
            chrome: None,
            bounded_busy: Some(working_marker),
        },
    };

    fn working_marker(text: &str) -> bool {
        let Some(text) = text.strip_prefix("  ") else {
            return false;
        };
        let Some(spinner) = text.chars().next() else {
            return false;
        };
        if !('\u{2801}'..='\u{28ff}').contains(&spinner) {
            return false;
        }
        text[spinner.len_utf8()..]
            .strip_prefix(" Working…")
            .is_some_and(|tail| {
                tail.is_empty()
                    || tail
                        .strip_prefix(" · Tip: ")
                        .is_some_and(|tip| !tip.trim().is_empty())
            })
    }
}

// No Gemini renderer has been captured locally; retain bounded input and
// labelled phases without adopting a different renderer's role/lifecycle UI.
mod gemini {
    use super::*;
    pub(super) const POLICY: Policy = Policy {
        composer: ComposerStrategy::Bounded,
        semantics: Semantics {
            panel_responses: false,
            recorded_duration: false,
            bullet_roles: false,
            chrome: None,
            bounded_busy: None,
        },
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_kinds_have_stable_serialization_and_default() {
        assert_eq!(AdapterKind::default(), AdapterKind::Auto);
        for kind in AdapterKind::ALL {
            let encoded = serde_json::to_string(&kind).unwrap();
            assert_eq!(serde_json::from_str::<AdapterKind>(&encoded).unwrap(), kind);
            assert!(!kind.label().is_empty());
        }
        assert_eq!(
            serde_json::to_string(&AdapterKind::OpenCode).unwrap(),
            "\"open_code\""
        );
    }

    #[test]
    fn only_recorded_kimi_spinner_text_supplies_a_bounded_busy_candidate() {
        let marker = AdapterKind::Kimi.policy().semantics.bounded_busy.unwrap();
        for text in ["  ⠋ Working…", "  ⠹ Working… · Tip: @: mention files"] {
            assert!(marker(text), "{text}");
        }
        for text in [
            "⠋ Working…",
            "  ● Working…",
            "  ⠋ Working...",
            "  ⠋ Working… in an example",
            "  ⠋ Working… · Tip: ",
            ">   ⠋ Working…",
            "  ⠋ Thinking…",
        ] {
            assert!(!marker(text), "{text}");
        }
        for adapter in AdapterKind::ALL
            .into_iter()
            .filter(|kind| *kind != AdapterKind::Kimi)
        {
            assert!(adapter.policy().semantics.bounded_busy.is_none());
        }
    }
}
