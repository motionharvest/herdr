#[cfg(not(windows))]
use std::io;

#[cfg(any(not(windows), test))]
use crossterm::event::KeyboardEnhancementFlags;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyModifiers};
#[cfg(not(windows))]
use crossterm::event::{PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsKeyRecord {
    pub key_down: bool,
    pub repeat_count: u16,
    pub virtual_key_code: u16,
    pub virtual_scan_code: u16,
    pub unicode: u16,
    pub control_key_state: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextCommit {
    text: String,
}

impl TextCommit {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub(crate) fn into_string(self) -> String {
        self.text
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhysicalKeyId(u32);

/// Whether crossterm's Windows mouse-capture disable failed because no
/// enable ran in this process yet. Crossterm restores a saved console mode
/// on disable, and that saved mode only exists after an enable in the same
/// process, so this specific cold-disable error is a no-op, not a failure.
fn is_windows_cold_mouse_disable_error(err: &std::io::Error) -> bool {
    err.to_string() == "Initial console modes not set"
}

/// Sets host-terminal mouse capture, tolerating a Windows cold disable.
///
/// Unix terminals treat both directions as plain escape-sequence writes. A
/// Windows console that never enabled capture cannot restore a mode on
/// disable, so that one error is swallowed; every other error propagates.
pub fn set_host_mouse_capture(enabled: bool) -> std::io::Result<()> {
    if enabled {
        crossterm::execute!(std::io::stdout(), EnableMouseCapture)
    } else {
        match crossterm::execute!(std::io::stdout(), DisableMouseCapture) {
            Ok(()) => Ok(()),
            #[cfg(windows)]
            Err(err) if is_windows_cold_mouse_disable_error(&err) => Ok(()),
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod host_mouse_capture_tests {
    #[test]
    fn cold_disable_error_is_recognized_verbatim() {
        let err = std::io::Error::other("Initial console modes not set");
        assert!(super::is_windows_cold_mouse_disable_error(&err));
    }

    #[test]
    fn other_io_errors_are_not_cold_disable_errors() {
        let err = std::io::Error::other("Access is denied. (os error 5)");
        assert!(!super::is_windows_cold_mouse_disable_error(&err));
    }
}

/// Reserved for physical-key tracking in the Windows reader port; the
/// protocol and encoder only need `has_physical_identity` today.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyIdentity {
    Physical(PhysicalKeyId),
    Semantic(KeyCode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeySource {
    Synthesized,
    Vt {
        bytes: Vec<u8>,
    },
    WindowsConsole {
        record: WindowsKeyRecord,
        physical_key: Option<PhysicalKeyId>,
    },
}

impl WindowsKeyRecord {
    fn physical_key_id(self) -> Option<PhysicalKeyId> {
        const ENHANCED_KEY: u32 = 0x0100;
        (self.virtual_scan_code != 0).then(|| {
            PhysicalKeyId(
                u32::from(self.virtual_scan_code)
                    | (u32::from(self.control_key_state & ENHANCED_KEY != 0) << 16),
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalKey {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub kind: crossterm::event::KeyEventKind,
    pub repeat_count: u16,
    pub shifted_codepoint: Option<u32>,
    pub generated_text: Option<String>,
    source: KeySource,
}

impl TerminalKey {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self {
            code,
            modifiers,
            kind: crossterm::event::KeyEventKind::Press,
            repeat_count: 1,
            shifted_codepoint: None,
            generated_text: None,
            source: KeySource::Synthesized,
        }
    }

    pub fn with_kind(mut self, kind: crossterm::event::KeyEventKind) -> Self {
        if kind == crossterm::event::KeyEventKind::Release {
            self.repeat_count = 1;
            self.generated_text = None;
        }
        self.kind = kind;
        self
    }

    pub fn with_repeat_count(mut self, repeat_count: u16) -> Self {
        self.repeat_count = if self.kind == crossterm::event::KeyEventKind::Release {
            1
        } else {
            repeat_count.max(1)
        };
        self
    }

    pub(crate) fn with_modifiers(mut self, modifiers: KeyModifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    #[allow(dead_code)] // Reserved for the upcoming raw input parser to preserve shifted/base key pairs.
    pub fn with_shifted_codepoint(mut self, shifted_codepoint: u32) -> Self {
        self.shifted_codepoint = Some(shifted_codepoint);
        self
    }

    pub(crate) fn with_generated_text(mut self, text: Option<String>) -> Self {
        self.generated_text = if self.kind == crossterm::event::KeyEventKind::Release {
            None
        } else {
            text
        };
        self
    }

    pub(crate) fn with_vt_bytes(mut self, bytes: Vec<u8>) -> Self {
        self.source = KeySource::Vt { bytes };
        self
    }

    pub fn with_windows_record(mut self, record: WindowsKeyRecord) -> Self {
        self.repeat_count = if self.kind == crossterm::event::KeyEventKind::Release {
            1
        } else {
            record.repeat_count.max(1)
        };
        self.source = KeySource::WindowsConsole {
            physical_key: record.physical_key_id(),
            record,
        };
        self
    }

    #[cfg(any(windows, test))]
    pub(crate) fn vt_bytes(&self) -> Option<&[u8]> {
        match &self.source {
            KeySource::Vt { bytes } => Some(bytes),
            KeySource::Synthesized | KeySource::WindowsConsole { .. } => None,
        }
    }

    #[cfg(any(windows, test))]
    pub(crate) fn windows_record(&self) -> Option<WindowsKeyRecord> {
        match self.source {
            KeySource::WindowsConsole { record, .. } => Some(record),
            KeySource::Synthesized | KeySource::Vt { .. } => None,
        }
    }

    #[allow(dead_code)] // reserved for the Windows reader port's physical-key tracking
    pub(crate) fn identity(&self) -> KeyIdentity {
        match self.source {
            KeySource::WindowsConsole {
                physical_key: Some(physical_key),
                ..
            } => KeyIdentity::Physical(physical_key),
            KeySource::WindowsConsole {
                physical_key: None, ..
            }
            | KeySource::Synthesized
            | KeySource::Vt { .. } => KeyIdentity::Semantic(self.code),
        }
    }

    pub(crate) fn has_physical_identity(&self) -> bool {
        matches!(
            self.source,
            KeySource::WindowsConsole {
                physical_key: Some(_),
                ..
            }
        )
    }

    pub fn with_text_commit(mut self) -> Self {
        let has_text_only_modifiers = match self.code {
            KeyCode::Char(ch) if ch.is_uppercase() => {
                self.modifiers == KeyModifiers::SHIFT || self.modifiers.is_empty()
            }
            KeyCode::Char(_) => self.modifiers.is_empty(),
            _ => false,
        };
        if has_text_only_modifiers && self.kind == crossterm::event::KeyEventKind::Press {
            self.generated_text = match self.code {
                KeyCode::Char(ch) => Some(ch.to_string()),
                _ => None,
            };
        }
        self
    }

    pub fn as_key_event(&self) -> KeyEvent {
        KeyEvent::new_with_kind(self.code, self.modifiers, self.kind)
    }
}

impl From<KeyEvent> for TerminalKey {
    fn from(value: KeyEvent) -> Self {
        Self::new(value.code, value.modifiers).with_kind(value.kind)
    }
}

pub(crate) const KITTY_FLAG_REPORT_ALL_KEYS: u16 = 0b0000_1000;

#[cfg(any(not(windows), test))]
pub fn ime_compatible_keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifyOtherKeysMode {
    Mode1,
    Mode2,
}

impl ModifyOtherKeysMode {
    pub fn set_sequence(self) -> &'static [u8] {
        match self {
            Self::Mode1 => b"\x1b[>4;1m",
            Self::Mode2 => b"\x1b[>4;2m",
        }
    }
}

pub fn host_modify_other_keys_mode(
    in_tmux: bool,
    term_program: Option<&str>,
    wezterm_pane: bool,
) -> Option<ModifyOtherKeysMode> {
    if in_tmux {
        return Some(ModifyOtherKeysMode::Mode2);
    }

    if wezterm_pane || term_program.is_some_and(|program| program.eq_ignore_ascii_case("wezterm")) {
        return Some(ModifyOtherKeysMode::Mode1);
    }

    None
}

/// Pushes the kitty keyboard-enhancement flags onto the host terminal.
///
/// crossterm ships no Windows implementation for progressive keyboard
/// enhancement: `PushKeyboardEnhancementFlags` and
/// `PopKeyboardEnhancementFlags` always report ANSI support as false and
/// hard-error through the legacy WinAPI dispatcher, even on a VT-enabled
/// console, and crossterm's own support probe is hardcoded to `Ok(false)` on
/// Windows. Windows hosts therefore never enter the protocol and keep the
/// legacy keyboard path; other hosts push unconditionally.
#[cfg(not(windows))]
pub fn push_keyboard_enhancement() -> io::Result<()> {
    let mut stdout = io::stdout();
    crossterm::execute!(
        stdout,
        PushKeyboardEnhancementFlags(ime_compatible_keyboard_enhancement_flags())
    )
}

/// Pops one level of kitty keyboard-enhancement flags from the host
/// terminal.
///
/// See [`push_keyboard_enhancement`] for why Windows never pops.
#[cfg(not(windows))]
pub fn pop_keyboard_enhancement() -> io::Result<()> {
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, PopKeyboardEnhancementFlags)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardProtocol {
    Legacy,
    Kitty { flags: u16 },
}

impl KeyboardProtocol {
    pub fn from_kitty_flags(flags: u16) -> Self {
        if flags == 0 {
            Self::Legacy
        } else {
            Self::Kitty { flags }
        }
    }

    pub(crate) fn reports_event_types(self) -> bool {
        matches!(self, Self::Kitty { flags } if flags & 0b0000_0010 != 0)
    }

    pub(crate) fn reports_all_keys(self) -> bool {
        matches!(self, Self::Kitty { flags } if flags & KITTY_FLAG_REPORT_ALL_KEYS != 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseProtocolMode {
    None,
    Press,
    PressRelease,
    ButtonMotion,
    AnyMotion,
}

impl MouseProtocolMode {
    pub fn reporting_enabled(self) -> bool {
        self != Self::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseProtocolEncoding {
    Default,
    Utf8,
    Sgr,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_source_exposes_typed_identity_without_changing_semantics() {
        let record = WindowsKeyRecord {
            key_down: true,
            repeat_count: 1,
            virtual_key_code: 27,
            virtual_scan_code: 1,
            unicode: 27,
            control_key_state: 0,
        };
        let key = TerminalKey::new(KeyCode::Esc, KeyModifiers::empty()).with_windows_record(record);
        let enhanced = TerminalKey::new(KeyCode::Esc, KeyModifiers::empty()).with_windows_record(
            WindowsKeyRecord {
                control_key_state: 0x0100,
                ..record
            },
        );

        assert!(matches!(key.identity(), KeyIdentity::Physical(_)));
        assert_ne!(key.identity(), enhanced.identity());
        assert_eq!(key.code, KeyCode::Esc);
    }

    #[test]
    fn semantic_source_uses_semantic_identity() {
        let key = TerminalKey::new(KeyCode::Char('x'), KeyModifiers::CONTROL);

        assert_eq!(key.identity(), KeyIdentity::Semantic(KeyCode::Char('x')));
        assert!(!key.has_physical_identity());
        assert_eq!(key.windows_record(), None);
    }

    #[test]
    fn native_source_remains_immutable_when_canonical_phase_changes() {
        let key = TerminalKey::new(KeyCode::Esc, KeyModifiers::empty())
            .with_windows_record(WindowsKeyRecord {
                key_down: true,
                repeat_count: 1,
                virtual_key_code: 27,
                virtual_scan_code: 1,
                unicode: 27,
                control_key_state: 0,
            })
            .with_kind(crossterm::event::KeyEventKind::Release);

        assert_eq!(key.kind, crossterm::event::KeyEventKind::Release);
        assert_eq!(
            key.windows_record().map(|record| record.key_down),
            Some(true)
        );
        assert_eq!(key.repeat_count, 1);
    }

    #[test]
    fn release_clears_generated_text_and_grouped_repeat_count() {
        let release = TerminalKey::new(KeyCode::Char('a'), KeyModifiers::empty())
            .with_generated_text(Some("a".to_owned()))
            .with_repeat_count(4)
            .with_kind(crossterm::event::KeyEventKind::Release);
        let regrouped_release = release
            .clone()
            .with_repeat_count(4)
            .with_generated_text(Some("ignored".to_owned()));

        assert_eq!(release.generated_text, None);
        assert_eq!(release.repeat_count, 1);
        assert_eq!(regrouped_release.generated_text, None);
        assert_eq!(regrouped_release.repeat_count, 1);
    }

    #[test]
    fn non_ascii_uppercase_with_shift_is_committed_text() {
        let key = TerminalKey::new(KeyCode::Char('É'), KeyModifiers::SHIFT).with_text_commit();

        assert_eq!(key.generated_text.as_deref(), Some("É"));
    }

    #[test]
    fn protocol_from_zero_flags_is_legacy() {
        assert_eq!(
            KeyboardProtocol::from_kitty_flags(0),
            KeyboardProtocol::Legacy
        );
    }

    #[test]
    fn protocol_from_nonzero_flags_is_kitty() {
        assert_eq!(
            KeyboardProtocol::from_kitty_flags(7),
            KeyboardProtocol::Kitty { flags: 7 }
        );
    }

    #[test]
    fn keyboard_enhancement_flags_stay_ime_compatible() {
        let flags = ime_compatible_keyboard_enhancement_flags();

        assert!(flags.contains(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES));
        assert!(flags.contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES));
        assert!(flags.contains(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS));
        assert!(!flags.contains(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES));
    }

    #[test]
    fn modify_other_keys_mode_is_enabled_for_tmux() {
        assert_eq!(
            host_modify_other_keys_mode(true, Some("WezTerm"), true),
            Some(ModifyOtherKeysMode::Mode2)
        );
    }

    #[test]
    fn modify_other_keys_mode_is_enabled_for_wezterm_hosts() {
        assert_eq!(
            host_modify_other_keys_mode(false, Some("WezTerm"), false),
            Some(ModifyOtherKeysMode::Mode1)
        );
        assert_eq!(
            host_modify_other_keys_mode(false, None, true),
            Some(ModifyOtherKeysMode::Mode1)
        );
    }

    #[test]
    fn modify_other_keys_mode_is_not_enabled_for_unknown_hosts() {
        assert_eq!(
            host_modify_other_keys_mode(false, Some("ghostty"), false),
            None
        );
        assert_eq!(host_modify_other_keys_mode(false, None, false), None);
    }
}
