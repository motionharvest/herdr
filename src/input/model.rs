use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(any(not(windows), test))]
use crossterm::event::KeyboardEnhancementFlags;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyModifiers};
#[cfg(not(windows))]
use crossterm::event::{PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_INPUT, STD_INPUT_HANDLE,
};

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

/// Whether this process currently has crossterm mouse capture enabled.
///
/// crossterm's Windows implementation restores a saved console mode when
/// disabling mouse capture, and that saved mode only exists after an enable
/// in the same process. A disable before any enable is therefore a hard
/// error on Windows while staying a harmless escape-sequence no-op on Unix.
/// The flag keeps the disable a no-op on Windows until something was really
/// enabled, without changing Unix behavior.
static HOST_MOUSE_CAPTURED: AtomicBool = AtomicBool::new(false);

/// Whether host VT input is currently active in this process.
#[cfg(windows)]
static HOST_VT_INPUT_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether Herdr added the VT input bit itself. Only Herdr's own bit is
/// cleared on deactivate; a host that already delivered VT input keeps it.
#[cfg(windows)]
static HOST_VT_INPUT_ADDED_BY_HERDR: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
fn host_console_input_mode() -> io::Result<(HANDLE, u32)> {
    // SAFETY: plain console mode queries on the process stdin handle; both
    // calls only read kernel state and the handle is checked first.
    unsafe {
        let handle = GetStdHandle(STD_INPUT_HANDLE);
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::other("stdin is not a console handle"));
        }
        let mut mode: u32 = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((handle, mode))
    }
}

#[cfg(windows)]
fn host_set_console_input_mode(handle: HANDLE, mode: u32) -> io::Result<()> {
    // SAFETY: sets the input console mode for this handle only.
    if unsafe { SetConsoleMode(handle, mode) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Enable host-terminal mouse capture for Herdr's own mouse UI.
pub fn enable_host_mouse_capture() -> io::Result<()> {
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnableMouseCapture)?;
    HOST_MOUSE_CAPTURED.store(true, Ordering::Release);
    reapply_host_vt_input()?;
    Ok(())
}

/// Disable host-terminal mouse capture. Cold disables are a no-op on
/// Windows because there is no saved console mode to restore yet.
pub fn disable_host_mouse_capture() -> io::Result<()> {
    let was_captured = HOST_MOUSE_CAPTURED.load(Ordering::Acquire);
    let result = if cfg!(windows) && !was_captured {
        Ok(())
    } else {
        let mut stdout = io::stdout();
        crossterm::execute!(stdout, DisableMouseCapture)
    };
    if result.is_ok() {
        HOST_MOUSE_CAPTURED.store(false, Ordering::Release);
        reapply_host_vt_input()?;
    }
    result
}

/// Re-add the VT input bit after crossterm replaced the console input mode.
///
/// crossterm's Windows mouse capture does not go through escape sequences:
/// the enable writes a fixed mouse mode over the whole input mode and the
/// disable restores the mode saved at the first enable. Both drop
/// `ENABLE_VIRTUAL_TERMINAL_INPUT`, so every mouse toggle must put the bit
/// back while VT input is active. Nothing replaces the mode on Unix, so the
/// call is a no-op there.
#[cfg(windows)]
fn reapply_host_vt_input() -> io::Result<()> {
    if !HOST_VT_INPUT_ACTIVE.load(Ordering::Acquire) {
        return Ok(());
    }
    let (handle, mode) = host_console_input_mode()?;
    if mode & ENABLE_VIRTUAL_TERMINAL_INPUT != 0 {
        return Ok(());
    }
    host_set_console_input_mode(handle, mode | ENABLE_VIRTUAL_TERMINAL_INPUT)
}

/// Unix counterpart of [`reapply_host_vt_input`]: no console mode is
/// replaced on Unix, so there is nothing to reapply.
#[cfg(not(windows))]
fn reapply_host_vt_input() -> io::Result<()> {
    Ok(())
}

/// Activate VT input on the Windows console.
///
/// Legacy Windows consoles deliver key presses as structured `KEY_EVENT`
/// records unless the console input mode has `ENABLE_VIRTUAL_TERMINAL_INPUT`
/// set. Herdr reads raw stdin bytes and parses VT sequences, so arrow,
/// function, and modified keys only arrive as parsable bytes when the bit is
/// present. crossterm's raw mode never adds it. Activation records whether
/// Herdr added the bit itself; only then does [`deactivate_host_vt_input`]
/// clear it again.
#[cfg(windows)]
pub fn activate_host_vt_input() -> io::Result<()> {
    let (handle, mode) = host_console_input_mode()?;
    if mode & ENABLE_VIRTUAL_TERMINAL_INPUT == 0 {
        host_set_console_input_mode(handle, mode | ENABLE_VIRTUAL_TERMINAL_INPUT)?;
        HOST_VT_INPUT_ADDED_BY_HERDR.store(true, Ordering::Release);
    } else {
        // The host already delivers VT input; keep its bit on deactivate.
        HOST_VT_INPUT_ADDED_BY_HERDR.store(false, Ordering::Release);
    }
    HOST_VT_INPUT_ACTIVE.store(true, Ordering::Release);
    Ok(())
}

/// Unix counterpart of [`activate_host_vt_input`]: Unix terminals always
/// deliver key presses as byte sequences, so this is a successful no-op.
#[cfg(not(windows))]
pub fn activate_host_vt_input() -> io::Result<()> {
    Ok(())
}

/// Deactivate host VT input. Only the bit Herdr added itself is cleared;
/// a host that already delivered VT input keeps its bit.
#[cfg(windows)]
pub fn deactivate_host_vt_input() {
    if !HOST_VT_INPUT_ACTIVE.swap(false, Ordering::AcqRel) {
        return;
    }
    let added_by_herdr = HOST_VT_INPUT_ADDED_BY_HERDR.swap(false, Ordering::AcqRel);
    if !added_by_herdr {
        return;
    }
    if let Ok((handle, mode)) = host_console_input_mode() {
        if mode & ENABLE_VIRTUAL_TERMINAL_INPUT != 0 {
            let _ = host_set_console_input_mode(handle, mode & !ENABLE_VIRTUAL_TERMINAL_INPUT);
        }
    }
}

/// Unix counterpart of [`deactivate_host_vt_input`]: nothing was changed, so
/// this is a no-op.
#[cfg(not(windows))]
pub fn deactivate_host_vt_input() {}

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

/// Windows counterpart of [`push_keyboard_enhancement`]: the kitty protocol
/// has no Windows transport, so the push is a successful no-op.
#[cfg(windows)]
pub fn push_keyboard_enhancement() -> io::Result<()> {
    Ok(())
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

/// Windows counterpart of [`pop_keyboard_enhancement`]: nothing was ever
/// pushed, so the pop is a successful no-op.
#[cfg(windows)]
pub fn pop_keyboard_enhancement() -> io::Result<()> {
    Ok(())
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
