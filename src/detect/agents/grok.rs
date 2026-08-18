use super::super::AgentState;

/// Grok Build detection.
///
/// Screen rules follow `herdrdev/herdr` `src/detect/manifests/grok.toml`
/// version `2026.07.16.2`, plus the live Grok 1.0.5 chrome that file does
/// not name. OSC title and OSC 9;4 progress rules from that file are
/// omitted because this tree has no OSC detection input.
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
    if has_live_working_chrome(content) {
        return (AgentState::Working, false);
    }
    if has_idle_shortcuts_footer(content) || has_legacy_turn_completed(content) {
        return (AgentState::Idle, false);
    }
    (AgentState::Idle, true)
}

fn has_live_working_chrome(content: &str) -> bool {
    has_background_work_chip(content)
        || has_still_running_status(content)
        || has_stop_chip(content)
        || has_esc_cancel_footer(content)
        || has_send_to_bg_footer(content)
        || has_live_phase_status(content)
        || has_legacy_waiting_or_tool_line(content)
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
    content.lines().any(is_legacy_background_chip_line)
}

fn is_legacy_background_chip_line(line: &str) -> bool {
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

fn has_still_running_status(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        if !trimmed.starts_with('◎') {
            return false;
        }
        let lower = trimmed.to_ascii_lowercase();
        lower.contains("still running") || lower.contains("waiting")
    })
}

fn has_stop_chip(content: &str) -> bool {
    content.lines().any(|line| line.contains("[stop]"))
}

fn has_shortcuts_hint(lower: &str) -> bool {
    lower.contains("ctrl+.:shortcuts") || lower.contains("ctrl+x:shortcuts")
}

fn has_esc_cancel_footer(content: &str) -> bool {
    let footer = bottom_non_empty(content, 2);
    footer.contains("esc:cancel") && has_shortcuts_hint(&footer)
}

fn has_send_to_bg_footer(content: &str) -> bool {
    let footer = bottom_non_empty(content, 2);
    footer.contains("ctrl+b:send to bg") && has_shortcuts_hint(&footer)
}

fn has_live_phase_status(content: &str) -> bool {
    let above_prompt = lines_above_prompt(content);
    let lines: Vec<&str> = if above_prompt.is_empty() {
        content.lines().collect()
    } else {
        above_prompt
    };
    lines.iter().any(|line| is_live_phase_line(line))
}

fn is_live_phase_line(line: &str) -> bool {
    let text = strip_leading_spinner(line.trim());
    let lower = text.to_ascii_lowercase();
    lower.starts_with("thinking…")
        || lower.starts_with("thinking...")
        || lower.starts_with("waiting for response")
        || lower.starts_with("waiting for reply")
        || lower.starts_with("waiting on subagent")
        || lower.starts_with("waiting on task")
}

fn strip_leading_spinner(text: &str) -> &str {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return text;
    };
    if first.is_alphanumeric() {
        return text;
    }
    chars.as_str().trim_start()
}

fn lines_above_prompt(content: &str) -> Vec<&str> {
    let lines: Vec<&str> = content.lines().collect();
    let Some(prompt_index) = lines.iter().rposition(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with('╭') && trimmed.contains('─')
    }) else {
        return Vec::new();
    };
    lines[..prompt_index]
        .iter()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .take(2)
        .copied()
        .collect()
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
        && !footer.contains("ctrl+b:send to bg")
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

    #[test]
    fn still_running_status_keeps_an_idle_footer_working() {
        let screen = "     Worked for 16m7s                                                               stop  [hooks: 2]\n\
             ◎ 1 command still running · send a message to interrupt\n\
           ╭──────────────────────────────────────────────╮\n\
           │ ❯                                            │\n\
           ╰────────────────────────────────────────────── Grok 4.6 (high) · always-approve ─╯\n\
           Shift+Tab:mode  │  Ctrl+x:shortcuts\n";
        assert_eq!(detect(screen), AgentState::Working);
        assert!(has_visible_working(screen));
        assert!(!is_ambiguous(screen));
    }

    #[test]
    fn still_running_counts_without_an_interrupt_hint_are_working() {
        let screen = "◎ 1 command · 2 monitors · 1 loop · 1 subagent still running\n\
           Shift+Tab:mode  │  Ctrl+x:shortcuts\n";
        assert_eq!(detect(screen), AgentState::Working);
    }

    #[test]
    fn waiting_dot_status_is_working() {
        let screen = "◎ waiting · send a message to interrupt\n\
           Shift+Tab:mode  │  Ctrl+x:shortcuts\n";
        assert_eq!(detect(screen), AgentState::Working);
    }

    #[test]
    fn waiting_for_response_above_the_prompt_is_working() {
        let screen = "    Waiting for response… 1.8s                                                      12s ⇣29.7k\n\
           ╭──────────────────────────────────────────────╮\n\
           │ ❯                                            │\n\
           ╰────────────────────────────────────────────── Grok 4.6 (high) · always-approve ─╯\n\
           Shift+Tab:mode  │  Ctrl+x:shortcuts\n";
        assert_eq!(detect(screen), AgentState::Working);
        assert!(has_visible_working(screen));
    }

    #[test]
    fn waiting_for_reply_above_the_prompt_is_working() {
        let screen = "    Waiting for reply… 3.2s\n\
           ╭──────────────────────────────────────────────╮\n\
           │ ❯                                            │\n\
           ╰────────────────────────────────────────────── Grok 4.6 (high) · always-approve ─╯\n\
           Shift+Tab:mode  │  Ctrl+x:shortcuts\n";
        assert_eq!(detect(screen), AgentState::Working);
    }

    #[test]
    fn thinking_progress_above_the_prompt_is_working() {
        let screen = "    Thinking… ████░░░░ 12s\n\
           ╭──────────────────────────────────────────────╮\n\
           │ ❯                                            │\n\
           ╰────────────────────────────────────────────── Grok 4.6 (high) · always-approve ─╯\n\
           Shift+Tab:mode  │  Ctrl+x:shortcuts\n";
        assert_eq!(detect(screen), AgentState::Working);
        assert!(!is_ambiguous(screen));
    }

    #[test]
    fn stop_chip_with_trailing_interrupt_hint_is_working() {
        let screen = "    ⠦ Capture this working Grok pane's screen… 0.5s                    1m46s ⇣100k [↓][stop] · send a message to interrupt\n\
           Shift+Tab:mode  │  Esc:cancel  │  Ctrl+b:send to bg  │  Ctrl+x:shortcuts\n";
        assert_eq!(detect(screen), AgentState::Working);
        assert!(has_visible_working(screen));
    }

    #[test]
    fn send_to_bg_footer_is_working() {
        assert_eq!(
            detect("Shift+Tab:mode  │  Ctrl+b:send to bg  │  Ctrl+x:shortcuts"),
            AgentState::Working
        );
    }

    #[test]
    fn finished_thinking_in_the_transcript_is_not_working() {
        let screen = "     ◆ Thought for 10.1s\n\
             Worked for 2m51s                                                          stop  [hooks: 2]\n\
           ╭──────────────────────────────────────────────╮\n\
           │ ❯                                            │\n\
           ╰────────────────────────────────────────────── Grok 4.6 (high) · always-approve ─╯\n\
           Shift+Tab:mode  │  Ctrl+x:shortcuts\n";
        assert_eq!(detect(screen), AgentState::Idle);
    }
}
