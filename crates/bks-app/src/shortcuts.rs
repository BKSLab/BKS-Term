//! User-editable application shortcuts, independent of Win32 focus handling.
//! Contextual editor and terminal keys remain the caller's responsibility.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppAction {
    NewTab,
    Profiles,
    CloseTab,
    NextTab,
    PreviousTab,
    ToggleReading,
    ToggleCapture,
    ReadCurrentInput,
    ReadNewOutput,
    OpenTuiDialog,
    CopyOutput,
    Paste,
    ScrollToBottom,
    Search,
    NextMatch,
    PreviousMatch,
    PreviousBlock,
    NextBlock,
    Help,
    NewWindow,
    OpenInFolder,
    Preferences,
    RestoreProfile,
    OpenAgentPanel,
    Tab1,
    Tab2,
    Tab3,
    Tab4,
    Tab5,
    Tab6,
    Tab7,
    Tab8,
    Tab9,
}

impl AppAction {
    pub const ALL: [Self; 33] = [
        Self::NewTab,
        Self::Profiles,
        Self::CloseTab,
        Self::NextTab,
        Self::PreviousTab,
        Self::ToggleReading,
        Self::ToggleCapture,
        Self::ReadCurrentInput,
        Self::ReadNewOutput,
        Self::OpenTuiDialog,
        Self::CopyOutput,
        Self::Paste,
        Self::ScrollToBottom,
        Self::Search,
        Self::NextMatch,
        Self::PreviousMatch,
        Self::PreviousBlock,
        Self::NextBlock,
        Self::Help,
        Self::NewWindow,
        Self::OpenInFolder,
        Self::Preferences,
        Self::RestoreProfile,
        Self::OpenAgentPanel,
        Self::Tab1,
        Self::Tab2,
        Self::Tab3,
        Self::Tab4,
        Self::Tab5,
        Self::Tab6,
        Self::Tab7,
        Self::Tab8,
        Self::Tab9,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::NewTab => "Новая вкладка: выбрать профиль",
            Self::Profiles => "Меню профилей и команд",
            Self::CloseTab => "Закрыть вкладку",
            Self::NextTab => "Следующая вкладка",
            Self::PreviousTab => "Предыдущая вкладка",
            Self::ToggleReading => "Переключить ввод и чтение",
            Self::ToggleCapture => "Захват клавиатуры терминалом",
            Self::ReadCurrentInput => "Прочитать текущую строку ввода",
            Self::ReadNewOutput => "Читать новый вывод",
            Self::OpenTuiDialog => "Доступный диалог CLI",
            Self::CopyOutput => "Копировать выделение или блок вывода",
            Self::Paste => "Вставить в терминал",
            Self::ScrollToBottom => "Конец вывода терминала",
            Self::Search => "Поиск по выводу",
            Self::NextMatch => "Следующее совпадение",
            Self::PreviousMatch => "Предыдущее совпадение",
            Self::PreviousBlock => "Предыдущий блок вывода",
            Self::NextBlock => "Следующий блок вывода",
            Self::Help => "Справка",
            Self::NewWindow => "Новое окно: выбрать профиль",
            Self::OpenInFolder => "Открыть в выбранной папке",
            Self::Preferences => "Настройки",
            Self::RestoreProfile => "Выбрать недавний профиль",
            Self::OpenAgentPanel => "Панель интеграции AI CLI",
            Self::Tab1 => "Вкладка 1",
            Self::Tab2 => "Вкладка 2",
            Self::Tab3 => "Вкладка 3",
            Self::Tab4 => "Вкладка 4",
            Self::Tab5 => "Вкладка 5",
            Self::Tab6 => "Вкладка 6",
            Self::Tab7 => "Вкладка 7",
            Self::Tab8 => "Вкладка 8",
            Self::Tab9 => "Вкладка 9",
        }
    }

    fn defaults(self) -> &'static str {
        match self {
            Self::NewTab => "Ctrl+Shift+T",
            Self::Profiles => "Ctrl+Shift+P",
            Self::CloseTab => "Ctrl+Shift+W",
            Self::NextTab => "Ctrl+Tab",
            Self::PreviousTab => "Ctrl+Shift+Tab",
            Self::ToggleReading => "Ctrl+Shift+Space",
            Self::ToggleCapture => "Ctrl+Shift+I",
            Self::ReadCurrentInput => "Ctrl+Shift+L",
            Self::ReadNewOutput => "Ctrl+Shift+R",
            Self::OpenTuiDialog => "Ctrl+Shift+D",
            Self::CopyOutput => "Ctrl+Shift+C",
            Self::Paste => "Ctrl+Shift+V",
            Self::ScrollToBottom => "Ctrl+Shift+End",
            Self::Search => "Ctrl+F",
            Self::NextMatch => "F3",
            Self::PreviousMatch => "Shift+F3",
            Self::PreviousBlock => "Ctrl+Up; Alt+Up",
            Self::NextBlock => "Ctrl+Down; Alt+Down",
            Self::Help => "F1",
            Self::NewWindow => "Ctrl+Shift+N",
            Self::OpenInFolder => "Ctrl+Shift+O",
            Self::Preferences => "",
            Self::RestoreProfile => "",
            Self::OpenAgentPanel => "",
            Self::Tab1 => "Alt+1",
            Self::Tab2 => "Alt+2",
            Self::Tab3 => "Alt+3",
            Self::Tab4 => "Alt+4",
            Self::Tab5 => "Alt+5",
            Self::Tab6 => "Alt+6",
            Self::Tab7 => "Alt+7",
            Self::Tab8 => "Alt+8",
            Self::Tab9 => "Alt+9",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Modifiers {
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Chord {
    pub key: usize,
    pub modifiers: Modifiers,
}

impl Chord {
    pub fn parse(text: &str) -> Result<Self, String> {
        text.parse()
    }

    fn validate(self) -> Result<(), String> {
        let m = self.modifiers;
        let supported = matches!(self.key, 0x09 | 0x0d | 0x1b | 0x20..=0x28 | 0x2d..=0x2e | 0x30..=0x39 | 0x41..=0x5a | 0x70..=0x87);
        let system = (m.alt && matches!(self.key, 0x09 | 0x1b | 0x20 | 0x73))
            || (m.control && self.key == 0x1b)
            || (m.control && m.alt);
        let editing = (m.control
            && !m.shift
            && !m.alt
            && matches!(self.key, 0x2d | 0x41 | 0x43 | 0x56 | 0x58 | 0x59 | 0x5a))
            || (!m.control && !m.alt && m.shift && self.key == 0x2d);
        let typing = !m.control && !m.alt && !(0x70..=0x87).contains(&self.key);
        if !supported || system || editing || typing || matches!(self.key, 0x0d | 0x1b) {
            return Err(format!(
                "Сочетание {self} зарезервировано для Windows, редактора или ввода терминала."
            ));
        }
        Ok(())
    }
}

impl FromStr for Chord {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut modifiers = Modifiers::default();
        let mut key = None;
        for part in text.split('+') {
            let token = part.trim().to_ascii_lowercase();
            let slot = match token.as_str() {
                "ctrl" | "control" => Some(&mut modifiers.control),
                "shift" => Some(&mut modifiers.shift),
                "alt" => Some(&mut modifiers.alt),
                _ => None,
            };
            if let Some(slot) = slot {
                if *slot {
                    return Err("Модификатор указан повторно.".to_owned());
                }
                *slot = true;
            } else if key.is_none() {
                key = key_code(&token);
                if key.is_none() {
                    return Err(format!(
                        "Неизвестная клавиша: {part}. Пример: Ctrl+Shift+T."
                    ));
                }
            } else {
                return Err("Укажите одну клавишу и модификаторы Ctrl, Alt, Shift.".to_owned());
            }
        }
        Ok(Self {
            key: key.ok_or_else(|| "Не указана клавиша сочетания.".to_owned())?,
            modifiers,
        })
    }
}

fn key_code(token: &str) -> Option<usize> {
    if token.len() == 1 {
        let byte = token.as_bytes()[0];
        if byte.is_ascii_alphanumeric() {
            return Some(byte.to_ascii_uppercase() as usize);
        }
    }
    if let Some(number) = token
        .strip_prefix('f')
        .and_then(|n| n.parse::<usize>().ok())
    {
        if (1..=24).contains(&number) {
            return Some(0x6f + number);
        }
    }
    Some(match token {
        "tab" => 0x09,
        "enter" | "return" => 0x0d,
        "esc" | "escape" => 0x1b,
        "space" => 0x20,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "end" => 0x23,
        "home" => 0x24,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "insert" | "ins" => 0x2d,
        "delete" | "del" => 0x2e,
        _ => return None,
    })
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.control {
            write!(f, "Ctrl+")?;
        }
        if self.modifiers.alt {
            write!(f, "Alt+")?;
        }
        if self.modifiers.shift {
            write!(f, "Shift+")?;
        }
        match self.key {
            0x30..=0x39 | 0x41..=0x5a => write!(f, "{}", self.key as u8 as char),
            0x70..=0x87 => write!(f, "F{}", self.key - 0x6f),
            key => write!(
                f,
                "{}",
                match key {
                    0x09 => "Tab",
                    0x0d => "Enter",
                    0x1b => "Escape",
                    0x20 => "Space",
                    0x21 => "PageUp",
                    0x22 => "PageDown",
                    0x23 => "End",
                    0x24 => "Home",
                    0x25 => "Left",
                    0x26 => "Up",
                    0x27 => "Right",
                    0x28 => "Down",
                    0x2d => "Insert",
                    0x2e => "Delete",
                    _ => "Unknown",
                }
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ShortcutMap(BTreeMap<AppAction, Vec<Chord>>);

impl Default for ShortcutMap {
    fn default() -> Self {
        Self(
            AppAction::ALL
                .into_iter()
                .map(|action| {
                    (
                        action,
                        parse_bindings(action.defaults()).expect("valid built-in shortcuts"),
                    )
                })
                .collect(),
        )
    }
}

impl ShortcutMap {
    pub fn dispatch(&self, key: usize, modifiers: Modifiers) -> Option<AppAction> {
        let chord = Chord { key, modifiers };
        self.0
            .iter()
            .find_map(|(action, chords)| chords.contains(&chord).then_some(*action))
    }

    pub fn bindings(&self, action: AppAction) -> &[Chord] {
        self.0.get(&action).map_or(&[], Vec::as_slice)
    }

    pub fn binding_text(&self, action: AppAction) -> String {
        self.bindings(action)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// An empty list disables a shortcut except the mandatory recovery keys.
    pub fn set(&mut self, action: AppAction, chords: Vec<Chord>) -> Result<(), String> {
        let mut next = self.clone();
        next.0.insert(action, chords);
        next.validate()?;
        *self = next;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        for (action, required) in [
            (AppAction::Help, "F1"),
            (AppAction::ToggleReading, "Ctrl+Shift+Space"),
        ] {
            let chord = Chord::parse(required).expect("valid recovery shortcut");
            if !self.bindings(action).contains(&chord) {
                return Err(format!(
                    "Сочетание {required} обязательно для действия «{}»: оно возвращает управление. Можно добавить другое сочетание, сохранив {required}.",
                    action.label()
                ));
            }
        }
        let mut seen = BTreeMap::new();
        for (action, chords) in &self.0 {
            if chords.len() > 4 {
                return Err(
                    "Одному действию можно назначить не более четырёх сочетаний.".to_owned(),
                );
            }
            for chord in chords {
                chord.validate()?;
                if chord.modifiers.control
                    && !chord.modifiers.alt
                    && matches!(chord.key, 0x24 | 0x23)
                    && !(*action == AppAction::ScrollToBottom
                        && chord.modifiers.shift
                        && chord.key == 0x23)
                {
                    return Err(format!(
                        "Сочетание {chord} зарезервировано для навигации в текстовом редакторе."
                    ));
                }
                if let Some(previous) = seen.insert(*chord, *action) {
                    return Err(format!(
                        "Сочетание {chord} уже назначено действию «{}»; конфликт с «{}».",
                        previous.label(),
                        action.label()
                    ));
                }
            }
        }
        Ok(())
    }
}

pub fn parse_bindings(text: &str) -> Result<Vec<Chord>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    text.split(';')
        .map(|part| Chord::parse(part.trim()))
        .collect()
}

impl Serialize for Chord {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for ShortcutMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let overrides = BTreeMap::<AppAction, Vec<Chord>>::deserialize(deserializer)?;
        let mut map = Self::default();
        // A newly introduced default must not invalidate an existing user's
        // explicit binding. Its action stays available in the menu/settings.
        for action in [AppAction::ReadCurrentInput, AppAction::ReadNewOutput] {
            if !overrides.contains_key(&action) {
                map.0
                    .get_mut(&action)
                    .expect("default action")
                    .retain(|chord| !overrides.values().any(|bindings| bindings.contains(chord)));
            }
        }
        map.0.extend(overrides);
        map.validate().map_err(serde::de::Error::custom)?;
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_dispatch_exact_modifiers_and_keep_block_aliases() {
        let map = ShortcutMap::default();
        map.validate().unwrap();
        for action in AppAction::ALL {
            for chord in map.bindings(action) {
                assert_eq!(map.dispatch(chord.key, chord.modifiers), Some(action));
                assert_eq!(Chord::parse(&chord.to_string()).unwrap(), *chord);
            }
        }
        assert_eq!(map.bindings(AppAction::PreviousBlock).len(), 2);
        assert_eq!(
            map.dispatch(
                0x54,
                Modifiers {
                    control: true,
                    shift: false,
                    alt: false
                }
            ),
            None
        );
    }

    #[test]
    fn conflicts_and_reserved_keys_leave_the_map_unchanged() {
        let mut map = ShortcutMap::default();
        let original = map.clone();
        for text in [
            "Ctrl+Shift+T",
            "Ctrl+C",
            "Ctrl+Insert",
            "Alt+F4",
            "Ctrl+Alt+Q",
            "Shift+Tab",
            "Enter",
            "A",
            "Alt+Space",
        ] {
            assert!(
                map.set(AppAction::Preferences, parse_bindings(text).unwrap())
                    .is_err(),
                "{text}"
            );
            assert_eq!(map, original);
        }
        assert!(parse_bindings("Ctrl+Ctrl+T").is_err());
        assert!(parse_bindings("Ctrl+T+W").is_err());
        assert!(parse_bindings("Ctrl+;").is_err());
        assert!(parse_bindings("Ctrl+F25").is_err());
    }

    #[test]
    fn overrides_round_trip_and_missing_actions_keep_defaults() {
        let mut map = ShortcutMap::default();
        map.set(AppAction::NewTab, parse_bindings("Ctrl+Shift+Y").unwrap())
            .unwrap();
        map.set(AppAction::NextBlock, Vec::new()).unwrap();
        let bytes = serde_json::to_vec(&map).unwrap();
        assert_eq!(serde_json::from_slice::<ShortcutMap>(&bytes).unwrap(), map);
        let partial: ShortcutMap =
            serde_json::from_str(r#"{"new_tab":["Ctrl+Shift+Y"],"next_block":[]}"#).unwrap();
        assert_eq!(partial, map);
        assert!(serde_json::from_str::<ShortcutMap>(r#"{"new_tab":["Ctrl+Shift+W"]}"#).is_err());
    }

    #[test]
    fn new_reading_actions_preserve_existing_explicit_user_shortcuts() {
        let map: ShortcutMap = serde_json::from_str(
            r#"{"toggle_reading":["Ctrl+Shift+Space","Ctrl+Shift+R"],"preferences":["Ctrl+Shift+L"]}"#,
        ).unwrap();
        assert!(map.bindings(AppAction::ReadNewOutput).is_empty());
        assert!(map.bindings(AppAction::ReadCurrentInput).is_empty());
        let chord = Chord::parse("Ctrl+Shift+R").unwrap();
        assert_eq!(
            map.dispatch(chord.key, chord.modifiers),
            Some(AppAction::ToggleReading)
        );
        let restored: ShortcutMap =
            serde_json::from_str(&serde_json::to_string(&map).unwrap()).unwrap();
        assert_eq!(restored, map);
    }

    #[test]
    fn recovery_shortcuts_cannot_be_removed_or_reassigned_but_accept_aliases() {
        let mut map = ShortcutMap::default();
        let original = map.clone();
        for (action, required, aliases) in [
            (AppAction::Help, "F1", "F1; Ctrl+Shift+H"),
            (
                AppAction::ToggleReading,
                "Ctrl+Shift+Space",
                "Ctrl+Shift+Space; Ctrl+Shift+H",
            ),
        ] {
            assert!(map.set(action, Vec::new()).is_err());
            assert!(map
                .set(action, parse_bindings("Ctrl+Shift+H").unwrap())
                .is_err());
            assert!(map
                .set(AppAction::Preferences, parse_bindings(required).unwrap())
                .is_err());
            assert_eq!(map, original);
            let mut aliases_map = map.clone();
            aliases_map
                .set(action, parse_bindings(aliases).unwrap())
                .unwrap();
            let chord = Chord::parse(required).unwrap();
            assert_eq!(
                aliases_map.dispatch(chord.key, chord.modifiers),
                Some(action)
            );
        }
        assert!(
            serde_json::from_str::<ShortcutMap>(r#"{"help":[],"preferences":["F1"]}"#).is_err()
        );
        assert!(serde_json::from_str::<ShortcutMap>(r#"{"toggle_reading":[]}"#).is_err());
    }

    #[test]
    fn editor_document_navigation_is_reserved_except_contextual_terminal_default() {
        let mut map = ShortcutMap::default();
        let original = map.clone();
        for text in ["Ctrl+Home", "Ctrl+End", "Ctrl+Shift+Home", "Ctrl+Shift+End"] {
            assert!(map
                .set(AppAction::Preferences, parse_bindings(text).unwrap())
                .is_err());
            assert_eq!(map, original);
        }
        map.set(
            AppAction::ScrollToBottom,
            parse_bindings("Ctrl+Shift+End; Ctrl+Shift+B").unwrap(),
        )
        .unwrap();
        map.validate().unwrap();
        let chord = Chord::parse("Ctrl+Shift+End").unwrap();
        assert_eq!(
            map.dispatch(chord.key, chord.modifiers),
            Some(AppAction::ScrollToBottom)
        );
        let chord = Chord::parse("Ctrl+Shift+B").unwrap();
        assert_eq!(
            map.dispatch(chord.key, chord.modifiers),
            Some(AppAction::ScrollToBottom)
        );
        assert!(map
            .set(
                AppAction::ScrollToBottom,
                parse_bindings("Ctrl+Home").unwrap()
            )
            .is_err());
    }
}
