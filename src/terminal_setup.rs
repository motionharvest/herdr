//! Explicit setup/teardown transaction for the host-terminal protocols
//! layered on top of ratatui's raw mode and alternate screen.
//!
//! Teardown order is load-bearing on Windows. Crossterm caches the
//! console input mode present before the first mouse-capture enable and
//! writes that cached mode back on every capture disable, and the Windows
//! VT-input restore mode is captured before that first mouse operation.
//! Disabling mouse capture first and restoring the saved VT-input mode
//! last therefore lands the console on the true pre-setup mode. Every
//! teardown step always runs even when an earlier one fails, and the first
//! error is preserved.

use std::io::{self, Write};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
};
use crossterm::execute;

/// The protocols a setup applied, so teardown undoes exactly those. The
/// protocols every teardown disables unconditionally (mouse capture,
/// bracketed paste, focus reporting, ratatui's raw mode and alternate
/// screen) need no flag.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostProtocolSetup {
    /// xterm modifyOtherKeys was written and must be reset.
    pub(crate) modify_other_keys: bool,
    /// Kitty keyboard enhancement was pushed and must be popped.
    #[cfg(not(windows))]
    pub(crate) keyboard_enhancement: bool,
    /// The Windows win32 input mode sequence was written and must be
    /// disabled.
    #[cfg(windows)]
    pub(crate) win32_input_mode: bool,
}

fn write_flush(bytes: &[u8]) -> io::Result<()> {
    let mut stdout = io::stdout();
    stdout.write_all(bytes)?;
    stdout.flush()
}

impl HostProtocolSetup {
    /// Enables or disables host mouse capture through the shared helper
    /// that tolerates a Windows console which never enabled capture.
    pub(crate) fn set_mouse_capture(&mut self, enabled: bool) -> io::Result<()> {
        crate::input::set_host_mouse_capture(enabled)
    }

    /// Enables bracketed paste and focus reporting.
    pub(crate) fn enable_paste_focus(&mut self) -> io::Result<()> {
        execute!(io::stdout(), EnableBracketedPaste, EnableFocusChange)
    }

    /// Pushes kitty keyboard enhancement. Windows has no crossterm
    /// implementation, so it is a successful no-op there.
    pub(crate) fn push_keyboard_enhancement(&mut self) -> io::Result<()> {
        #[cfg(not(windows))]
        {
            let result = crate::input::push_keyboard_enhancement();
            if result.is_ok() {
                self.keyboard_enhancement = true;
            }
            result
        }
        #[cfg(windows)]
        Ok(())
    }

    /// Writes the xterm modifyOtherKeys enable sequence, remembering that it
    /// must be reset.
    pub(crate) fn enable_modify_other_keys(
        &mut self,
        mode: crate::input::ModifyOtherKeysMode,
    ) -> io::Result<()> {
        let result = write_flush(mode.set_sequence());
        if result.is_ok() {
            self.modify_other_keys = true;
        }
        result
    }

    /// Writes the Windows win32 input mode enable sequence.
    #[cfg(windows)]
    pub(crate) fn enable_win32_input_mode(&mut self) -> io::Result<()> {
        let result = write_flush(b"\x1b[?9001h");
        if result.is_ok() {
            self.win32_input_mode = true;
        }
        result
    }

    /// Tears the applied protocols back down: output protocol cleanup, the
    /// shared mouse disable, the saved Windows VTI mode, then ratatui's
    /// restore. Always finishes every step; returns the first error, if any.
    pub(crate) fn rollback(self) -> Option<io::Error> {
        let mut first_error: Option<io::Error> = None;

        // Output protocol cleanup.
        if self.modify_other_keys {
            if let Err(err) = write_flush(b"\x1b[>4;0m") {
                first_error.get_or_insert(err);
            }
        }
        #[cfg(not(windows))]
        if self.keyboard_enhancement {
            if let Err(err) = crate::input::pop_keyboard_enhancement() {
                first_error.get_or_insert(err);
            }
        }
        #[cfg(windows)]
        if self.win32_input_mode {
            if let Err(err) = write_flush(b"\x1b[?9001l") {
                first_error.get_or_insert(err);
            }
        }
        if let Err(err) = execute!(io::stdout(), DisableFocusChange, DisableBracketedPaste) {
            first_error.get_or_insert(err);
        }

        // A Windows console that never enabled capture cannot restore a mode
        // on disable; the shared helper tolerates that cold disable.
        if let Err(err) = crate::input::set_host_mouse_capture(false) {
            first_error.get_or_insert(err);
        }

        // Restore the saved Windows VT-input mode only AFTER disabling mouse
        // capture: crossterm's capture enable caches the pre-mouse console
        // input mode and its disable writes that cached mode back, so a
        // restore before the disable would be overwritten. The saved mode is
        // captured before the first mouse operation, so restoring it last
        // lands the console on the true pre-setup mode. Taking clears the
        // shared slot so a panic hook cannot restore the same mode twice.
        #[cfg(windows)]
        if let Some(mode) = crate::client::take_windows_vti_restore_mode() {
            crate::client::restore_windows_input_mode_value(mode);
        }

        if let Err(err) = ratatui::try_restore() {
            first_error.get_or_insert(err);
        }

        first_error
    }
}

/// Pure model of the Windows lifecycle order this module relies on:
/// capture/enable VT input before the first mouse-capture operation, record
/// that pre-mouse mode, re-apply VTI after the mouse write without replacing
/// the saved slot, then teardown: mouse disable first, saved-mode restore
/// last. Asserts the console lands back on the true pre-setup mode for both
/// a startup mouse enable and a startup disable followed by a runtime enable.
#[cfg(test)]
mod lifecycle_order_tests {
    /// ENABLE_MOUSE_MODE from crossterm: quick-edit, extended flags, mouse.
    const ENABLE_MOUSE_MODE: u32 = 0x0010 | 0x0080 | 0x0008;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

    #[derive(Default)]
    struct SimConsole {
        mode: u32,
        /// Crossterm's ORIGINAL_CONSOLE_MODE, cached once at the first
        /// capture enable.
        cached_baseline: Option<u32>,
        /// Herdr's shared WINDOWS_VTI_RESTORE_MODE slot.
        saved_restore: Option<u32>,
    }

    impl SimConsole {
        /// Crossterm 0.29 enable_mouse_capture.
        fn enable_mouse_capture(&mut self) {
            if self.cached_baseline.is_none() {
                self.cached_baseline = Some(self.mode);
            }
            self.mode = ENABLE_MOUSE_MODE;
        }

        /// Crossterm 0.29 disable_mouse_capture.
        fn disable_mouse_capture(&mut self) {
            self.mode = self.cached_baseline.unwrap_or(self.mode);
        }

        /// enable_windows_virtual_terminal_input: returns the pre-enable
        /// mode to restore, or None when the bit already stuck.
        fn enable_vti(&mut self) -> Option<u32> {
            if self.mode & ENABLE_VIRTUAL_TERMINAL_INPUT != 0 {
                return None;
            }
            let restore = self.mode;
            self.mode |= ENABLE_VIRTUAL_TERMINAL_INPUT;
            Some(restore)
        }
    }

    /// The setup order shared by the client and the monolithic launcher.
    fn sim_setup(console: &mut SimConsole, startup_mouse: bool) {
        let vti = console.enable_vti();
        console.saved_restore = vti;
        if startup_mouse {
            console.enable_mouse_capture();
        }
        // Re-apply VTI after the mouse write; the returned restore mode is
        // dropped so the saved pre-mouse slot is never replaced.
        let _ = console.enable_vti();
    }

    /// The rollback order from HostProtocolSetup::rollback.
    fn sim_teardown(console: &mut SimConsole) {
        console.disable_mouse_capture();
        if let Some(mode) = console.saved_restore.take() {
            console.mode = mode;
        }
    }

    #[test]
    fn startup_mouse_on_lifecycle_restores_pre_setup_mode() {
        let original = 0x00f7; // no VT input bit
        let mut console = SimConsole {
            mode: original,
            ..Default::default()
        };

        sim_setup(&mut console, true);
        // The fix's core invariant: the baseline cached at the mouse enable
        // was captured AFTER VTI was enabled, so later disable writes carry
        // the VT input bit.
        assert_eq!(
            console.cached_baseline,
            Some(original | ENABLE_VIRTUAL_TERMINAL_INPUT)
        );
        // The mouse enable wiped the mode; the VTI re-apply restored the bit.
        assert_ne!(console.mode & ENABLE_VIRTUAL_TERMINAL_INPUT, 0);

        // A runtime toggle writes back the cached (VTI-enabled) baseline.
        console.disable_mouse_capture();
        console.enable_mouse_capture();

        sim_teardown(&mut console);
        assert_eq!(console.mode, original);
    }

    #[test]
    fn startup_mouse_off_then_runtime_on_restores_pre_setup_mode() {
        let original = 0x00f7;
        let mut console = SimConsole {
            mode: original,
            ..Default::default()
        };

        sim_setup(&mut console, false);
        // Startup disable left the baseline uncached, so the first runtime
        // enable caches the VTI-enabled mode.
        assert_eq!(console.cached_baseline, None);
        console.enable_mouse_capture();
        assert_ne!(
            console.cached_baseline,
            Some(original),
            "runtime enable must not cache the pre-VTI mode"
        );

        sim_teardown(&mut console);
        assert_eq!(console.mode, original);
    }

    #[test]
    fn old_setup_and_teardown_orders_left_vti_enabled() {
        let original = 0x00f7;
        let mut console = SimConsole {
            mode: original,
            ..Default::default()
        };

        // Old setup order: mouse write first, then VTI enable with save.
        // Startup mouse off means a cold disable that caches nothing.
        console.saved_restore = console.enable_vti();

        // Runtime enable caches the VTI-enabled mode.
        console.enable_mouse_capture();

        // Old teardown order: restore the saved mode BEFORE the mouse
        // disable, so the disable writes the cached VTI-enabled baseline
        // back over the restore.
        if let Some(mode) = console.saved_restore.take() {
            console.mode = mode;
        }
        console.disable_mouse_capture();

        // The bug this change fixes: the console keeps the VT input bit
        // after teardown instead of landing on the pre-setup mode.
        assert_eq!(console.mode, original | ENABLE_VIRTUAL_TERMINAL_INPUT);
    }
}
