use super::super::AgentState;

/// Grok Build detection.
///
/// Screen rules follow `herdrdev/herdr` `src/detect/manifests/grok.toml`
/// version `2026.07.16.2`. OSC title and OSC 9;4 progress rules from that
/// file are omitted because this tree has no OSC detection input.
/// `ctrl+x:shortcuts` is accepted as the live alias of that file's
/// `ctrl+.:shortcuts`.
pub(super) fn detect(content: &str) -> AgentState {
    detect_with_confidence(content).0
}

pub(super) fn is_ambiguous(content: &str) -> bool {
    detect_with_confidence(content).1
}

pub(super) fn has_visible_working(content: &str) -> bool {
    matches!(
        detect_with_confidence(content),
        (AgentState::Working, false)
    )
}

pub(super) fn has_visible_idle(content: &str) -> bool {
    matches!(detect_with_confidence(content), (AgentState::Idle, false))
}

fn detect_with_confidence(content: &str) -> (AgentState, bool) {
    if has_option_dialog(content)
        || has_permission_hints(content)
        || has_question_dialog_hints(content)
        || has_legacy_permission_scope(content)
    {
        return (AgentState::Blocked, false);
    }
    if has_background_work_chip(content)
        || has_stop_chip_spinner_line(content)
        || has_esc_cancel_footer(content)
        || has_legacy_waiting_or_tool_line(content)
    {
        return (AgentState::Working, false);
    }
    if has_idle_shortcuts_footer(content) || has_legacy_turn_completed(content) {
        return (AgentState::Idle, false);
    }
    (AgentState::Idle, true)
}

fn has_option_dialog(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with('┃')
            && trimmed.contains('(')
            && (trimmed.contains('●') || trimmed.contains('○'))
    })
}

fn has_permission_hints(content: &str) -> bool {
    let footer = bottom_non_empty(content, 2);
    footer.contains(":select") && footer.contains("ctrl+o:yolo") && footer.contains("ctrl+c:cancel")
}

fn has_question_dialog_hints(content: &str) -> bool {
    let footer = bottom_non_empty(content, 2);
    footer.contains("tab:scrollback") && footer.contains("shift+x:dismiss")
}

fn has_legacy_permission_scope(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("yes, proceed") && lower.contains("no, reject")
}

fn has_background_work_chip(content: &str) -> bool {
    let Some(line) = content.lines().find(|line| !line.trim().is_empty()) else {
        return false;
    };
    let trimmed = line.trim();
    let mut chars = trimmed.chars();
    let Some(mark) = chars.next() else {
        return false;
    };
    if !matches!(mark, '⋅' | ':' | '⸬' | '⁙' | '.' | '·') {
        return false;
    }
    let rest = chars.as_str().trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.starts_with('0') {
        return false;
    }
    rest[digits.len()..].trim_start().starts_with('│')
}

fn has_stop_chip_spinner_line(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        let mut chars = trimmed.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !('\u{2801}'..='\u{28FF}').contains(&first) {
            return false;
        }
        let rest = chars.as_str();
        rest.starts_with(|c: char| c.is_whitespace()) && trimmed.ends_with("[stop]")
    })
}

fn has_shortcuts_hint(lower: &str) -> bool {
    lower.contains("ctrl+.:shortcuts") || lower.contains("ctrl+x:shortcuts")
}

fn has_esc_cancel_footer(content: &str) -> bool {
    let footer = bottom_non_empty(content, 2);
    footer.contains("esc:cancel") && has_shortcuts_hint(&footer)
}

fn has_legacy_waiting_or_tool_line(content: &str) -> bool {
    let lower = content.to_lowercase();
    if lower.contains("ctrl+c:cancel") && lower.contains("ctrl+enter:interject") {
        return true;
    }
    content.lines().any(|line| {
        let trimmed = line.trim();
        let mut chars = trimmed.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !('\u{2801}'..='\u{28FF}').contains(&first) {
            return false;
        }
        let rest = chars.as_str().trim_start();
        rest.starts_with("Run ")
            || rest.starts_with("Read ")
            || rest.starts_with("Search ")
            || rest.starts_with("List ")
            || rest.starts_with("Waiting")
    })
}

fn has_idle_shortcuts_footer(content: &str) -> bool {
    let footer = bottom_non_empty(content, 2);
    has_shortcuts_hint(&footer)
        && !footer.contains("esc:cancel")
        && !footer.contains("ctrl+c:cancel")
}

fn has_legacy_turn_completed(content: &str) -> bool {
    content.to_lowercase().contains("turn completed")
}

fn bottom_non_empty(content: &str, n: usize) -> String {
    content
        .lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .take(n)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::AgentState;

    fn thinking_screen() -> String {
        "    ⠙ Thinking… 25s                                                                 10m59s ⇣188k [stop]\n\
         \n\
           ╭──────────────────────────────────────────────╮\n\
           │ ❯                                            │\n\
           ╰────────────────────────────────────────────── Grok 4.6 (high) · always-approve ─╯\n\
         \n\
           Shift+Tab:mode  │  Esc:cancel  │  Ctrl+x:shortcuts\n"
            .to_string()
    }

    fn idle_screen() -> String {
        "     Worked for 16m7s                                                               stop  [hooks: 2]\n\
         \n\
           ╭──────────────────────────────────────────────╮\n\
           │ ❯                                            │\n\
           ╰────────────────────────────────────────────── Grok 4.6 (high) · always-approve ─╯\n\
         \n\
           Shift+Tab:mode  │  Ctrl+x:shortcuts\n"
            .to_string()
    }

    #[test]
    fn thinking_status_with_stop_chip_is_working() {
        assert_eq!(detect(&thinking_screen()), AgentState::Working);
        assert_eq!(
            detect(
                "    ⠙ Thinking… 25s                                                                 10m59s ⇣188k [stop]\n"
            ),
            AgentState::Working
        );
    }

    #[test]
    fn current_esc_cancel_footer_is_working() {
        assert_eq!(
            detect("Shift+Tab:mode  │  Esc:cancel  │  Ctrl+x:shortcuts"),
            AgentState::Working
        );
        assert_eq!(
            detect("Shift+Tab:mode  │  Esc:cancel  │  Ctrl+.:shortcuts"),
            AgentState::Working
        );
    }

    #[test]
    fn finished_turn_with_idle_footer_is_idle() {
        assert_eq!(detect(&idle_screen()), AgentState::Idle);
    }

    #[test]
    fn stale_read_transcript_does_not_keep_an_idle_pane_working() {
        let screen = "  ❙  ◈ Read 2 files  [hooks: 6]\n\
             Worked for 2m51s                                                          stop  [hooks: 2]\n\
           Shift+Tab:mode  │  Ctrl+x:shortcuts\n";
        assert_eq!(detect(screen), AgentState::Idle);
    }

    #[test]
    fn splash_braille_without_stop_chip_is_not_working() {
        let screen = "⣿⣿⣿ grok splash\nShift+Tab:mode  │  Ctrl+x:shortcuts\n";
        assert_eq!(detect(screen), AgentState::Idle);
    }

    #[test]
    fn mid_repaint_without_footer_or_stop_chip_is_ambiguous() {
        let content = "I'll check the other agents' panes next.";
        assert_eq!(detect(content), AgentState::Idle);
        assert!(is_ambiguous(content));
    }

    #[test]
    fn thinking_is_never_ambiguous() {
        assert!(!is_ambiguous(&thinking_screen()));
    }

    #[test]
    fn idle_footer_is_never_ambiguous() {
        assert!(!is_ambiguous(&idle_screen()));
    }

    #[test]
    fn legacy_waiting_spinner_and_interject_footer_still_work() {
        let screen = "⠋ Waiting… 1.8s\nCtrl+c:cancel │ Ctrl+Enter:interject";
        assert_eq!(detect(screen), AgentState::Working);
    }

    #[test]
    fn option_dialog_is_blocked() {
        let screen = "┃  2 (○) Yes, proceed\n1/3:select │ Ctrl+o:yolo │ Ctrl+c:cancel";
        assert_eq!(detect(screen), AgentState::Blocked);
    }
}
