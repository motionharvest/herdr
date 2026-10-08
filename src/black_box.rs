//! What herdr says to Black Box, the GTK terminal, when a client runs in it.
//!
//! Black Box registers VTE terminal properties under `vte.ext.blackbox.`.
//! A program sets them with OSC 666 statements, `KEY=VALUE` to set, `KEY`
//! alone to reset, and `KEY!` to raise a valueless one. VTE only reads OSC
//! 666 terminated by ST, so every sequence here ends with `ESC \`.

/// `TERM_PROGRAM` of Black Box.
pub(crate) const TERM_PROGRAM: &str = "BlackBox";

/// Tells Black Box that the program in the tab opens the links a person
/// Ctrl+clicks, so Black Box should not open them as well.
pub(crate) const CLAIM_LINK_CLICKS: &str = "\x1b]666;vte.ext.blackbox.app-opens-links=1\x1b\\";

/// Gives Ctrl+clicked links back to Black Box.
pub(crate) const RELEASE_LINK_CLICKS: &str = "\x1b]666;vte.ext.blackbox.app-opens-links\x1b\\";

/// Whether this process runs in Black Box.
pub(crate) fn is_host() -> bool {
    std::env::var("TERM_PROGRAM").is_ok_and(|program| program == TERM_PROGRAM)
}

/// The sequence that asks Black Box to open a herdr terminal in its own
/// window over the given cells of this terminal. Black Box builds the attach
/// command itself, so the sequence carries only the terminal id, never a
/// command.
pub(crate) fn pop_out_sequence(
    terminal_id: &str,
    title: &str,
    column: u16,
    row: u16,
    columns: u16,
    rows: u16,
) -> String {
    const PREFIX: &str = "vte.ext.blackbox.pop-out";
    format!(
        "\x1b]666;{PREFIX}.terminal-id={};{PREFIX}.title={};{PREFIX}.column={column};\
         {PREFIX}.row={row};{PREFIX}.columns={columns};{PREFIX}.rows={rows};{PREFIX}.open!\x1b\\",
        escape_string(terminal_id),
        escape_string(title),
    )
}

/// Escapes a string termprop value: OSC 666 separates statements with `;`,
/// so VTE reads `\s` as a semicolon and `\\` as a backslash. Control
/// characters cannot appear in an OSC string and are dropped.
fn escape_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            ';' => escaped.push_str("\\s"),
            ch if ch.is_control() => {}
            ch => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pop_out_sets_every_termprop_then_raises_open() {
        let sequence = pop_out_sequence("term_abc1", "Joseph", 3, 4, 50, 20);
        assert_eq!(
            sequence,
            "\x1b]666;vte.ext.blackbox.pop-out.terminal-id=term_abc1;\
             vte.ext.blackbox.pop-out.title=Joseph;vte.ext.blackbox.pop-out.column=3;\
             vte.ext.blackbox.pop-out.row=4;vte.ext.blackbox.pop-out.columns=50;\
             vte.ext.blackbox.pop-out.rows=20;vte.ext.blackbox.pop-out.open!\x1b\\"
        );
    }

    #[test]
    fn string_values_escape_separators_and_drop_controls() {
        assert_eq!(escape_string("a;b\\c\x07d"), "a\\sb\\\\cd");
    }

    #[test]
    fn link_click_claim_sets_and_release_resets_one_termprop() {
        assert!(CLAIM_LINK_CLICKS.contains("vte.ext.blackbox.app-opens-links=1"));
        assert!(RELEASE_LINK_CLICKS.ends_with("vte.ext.blackbox.app-opens-links\x1b\\"));
    }
}
