use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Tabs},
    Frame,
};

use super::widgets::{
    action_button_row_rects, centered_popup_rect, modal_stack_areas, panel_contrast_fg,
    render_action_button, render_modal_choice_list, render_panel_shell, ActionButtonSpec,
};
use crate::{
    app::{
        state::{ExperimentSetting, Palette},
        AppState,
    },
    config::ToastDelivery,
};

pub(super) fn render_settings_overlay(app: &AppState, frame: &mut Frame, area: Rect) {
    use crate::app::state::SettingsSection;

    let p = &app.palette;
    let Some(popup) = centered_popup_rect(area, 76, 22) else {
        return;
    };

    super::dim_background(frame, area);

    let Some(inner) = render_panel_shell(frame, popup, p.accent, p.panel_bg) else {
        return;
    };
    if inner.height < 4 || inner.width < 10 {
        return;
    }

    let stack = modal_stack_areas(inner, 3, 2, 0, 1);
    let header_rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas::<3>(stack.header);

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            " settings",
            Style::default().fg(p.text).add_modifier(Modifier::BOLD),
        )])),
        header_rows[0],
    );

    let tab_labels = SettingsSection::ALL.iter().map(|section| {
        if app.settings_section_has_badge(*section) {
            Line::from(vec![
                Span::styled(
                    "● ",
                    Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
                ),
                Span::raw(section.label()),
            ])
        } else {
            Line::from(section.label())
        }
    });
    let tabs = Tabs::new(tab_labels)
        .select(
            SettingsSection::ALL
                .iter()
                .position(|section| *section == app.settings.section)
                .unwrap_or(0),
        )
        .style(Style::default().fg(p.overlay1))
        .highlight_style(
            Style::default()
                .fg(panel_contrast_fg(p))
                .bg(p.accent)
                .add_modifier(Modifier::BOLD),
        )
        .divider(" ")
        .padding(" ", " ");
    frame.render_widget(tabs, header_rows[1]);

    let sep = "─".repeat(inner.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(&sep, Style::default().fg(p.surface0))),
        header_rows[2],
    );

    let content_area = stack.content;

    match app.settings.section {
        SettingsSection::Theme => {
            render_settings_theme(app, frame, content_area);
        }
        SettingsSection::Sound => {
            render_settings_sound(app, frame, content_area);
        }
        SettingsSection::Toast => {
            render_modal_choice_list(
                frame,
                content_area,
                "notification popups",
                "choose where background popup notifications should appear",
                &[
                    ("off", ToastDelivery::Off),
                    ("inside herdr", ToastDelivery::Herdr),
                    ("via terminal", ToastDelivery::Terminal),
                    ("via system", ToastDelivery::System),
                ],
                app.toast_delivery(),
                app.settings.list.selected,
                p,
                2,
            );
        }
        SettingsSection::PaneLabels => {
            render_settings_toggle(
                frame,
                content_area,
                p,
                "agent border labels",
                "show detected agent names in split pane borders",
                app.agent_border_labels_enabled(),
                app.settings.list.selected,
            );
        }
        SettingsSection::Experiments => {
            render_settings_experiments(app, frame, content_area);
        }
        SettingsSection::Integrations => {
            render_settings_integrations(app, frame, content_area);
        }
    }

    if let Some(footer_area) = stack.footer {
        let footer_rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)])
            .areas::<2>(footer_area);
        let primary_label = settings_primary_button_label(app.settings.section);
        let show_primary = settings_show_primary_action(app);
        let (apply_rect, close_rect) =
            settings_button_rects(inner, app.settings.section, show_primary);
        if let Some(apply_rect) = apply_rect {
            render_action_button(
                frame,
                apply_rect,
                Some("enter"),
                primary_label,
                Style::default()
                    .fg(panel_contrast_fg(p))
                    .bg(p.accent)
                    .add_modifier(Modifier::BOLD),
            );
        }
        render_action_button(
            frame,
            close_rect,
            Some("esc"),
            "close",
            Style::default()
                .fg(p.text)
                .bg(p.surface0)
                .add_modifier(Modifier::BOLD),
        );

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" up/down", Style::default().fg(p.overlay0)),
                Span::styled(" move  ", Style::default().fg(p.overlay1)),
                Span::styled("tab", Style::default().fg(p.overlay0)),
                Span::styled(" switch section", Style::default().fg(p.overlay1)),
            ])),
            footer_rows[0],
        );
    }
}

pub(crate) fn settings_primary_button_label(
    section: crate::app::state::SettingsSection,
) -> &'static str {
    match section {
        crate::app::state::SettingsSection::Integrations => "install",
        _ => "apply",
    }
}

pub(crate) fn settings_show_primary_action(app: &AppState) -> bool {
    app.settings.section != crate::app::state::SettingsSection::Integrations
        || app
            .integration_recommendations
            .iter()
            .any(crate::integration::IntegrationRecommendation::needs_install)
}

pub(crate) fn settings_button_rects(
    inner: Rect,
    section: crate::app::state::SettingsSection,
    show_primary: bool,
) -> (Option<Rect>, Rect) {
    if !show_primary {
        let rects = action_button_row_rects(
            inner,
            &[ActionButtonSpec {
                hint: Some("esc"),
                label: "close",
            }],
            2,
            inner.height.saturating_sub(1),
        );
        return (None, rects[0]);
    }

    let rects = action_button_row_rects(
        inner,
        &[
            ActionButtonSpec {
                hint: Some("enter"),
                label: settings_primary_button_label(section),
            },
            ActionButtonSpec {
                hint: Some("esc"),
                label: "close",
            },
        ],
        2,
        inner.height.saturating_sub(1),
    );
    (Some(rects[0]), rects[1])
}

fn render_settings_integrations(app: &AppState, frame: &mut Frame, area: Rect) {
    let p = &app.palette;
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas::<4>(area);

    frame.render_widget(
        Paragraph::new("agent integrations")
            .style(Style::default().fg(p.text).add_modifier(Modifier::BOLD)),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(
            "let agents report state directly instead of relying only on process detection",
        )
        .style(Style::default().fg(p.overlay1))
        .wrap(ratatui::widgets::Wrap { trim: false }),
        rows[1],
    );

    let list_area = rows[3];
    let list_bottom = list_area.y + list_area.height;
    let mut next_row_y = list_area.y;

    if app.integration_recommendations.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " no integration targets available",
                Style::default().fg(p.overlay1),
            )),
            Rect::new(list_area.x, next_row_y, list_area.width, 1),
        );
        next_row_y = next_row_y.saturating_add(1);
    } else {
        for (idx, item) in app.integration_recommendations.iter().enumerate() {
            if next_row_y >= list_bottom {
                break;
            }
            let marker = match item.state {
                crate::integration::IntegrationStatusKind::Current => "*",
                crate::integration::IntegrationStatusKind::Outdated => "!",
                crate::integration::IntegrationStatusKind::NotInstalled if item.available => "+",
                crate::integration::IntegrationStatusKind::NotInstalled => "-",
            };
            // Status markers keep their per-state color even on the selected
            // row so install state stays readable while moving the selection.
            let marker_style = match item.state {
                crate::integration::IntegrationStatusKind::Current => Style::default().fg(p.green),
                crate::integration::IntegrationStatusKind::Outdated => {
                    Style::default().fg(p.yellow)
                }
                crate::integration::IntegrationStatusKind::NotInstalled if item.available => {
                    Style::default().fg(p.accent)
                }
                crate::integration::IntegrationStatusKind::NotInstalled => {
                    Style::default().fg(p.overlay0)
                }
            };
            let selected = idx == app.settings.list.selected;
            let row_style = if selected {
                Style::default()
                    .bg(p.surface0)
                    .fg(p.text)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let (label_style, status_style) = if selected {
                (
                    Style::default()
                        .fg(p.text)
                        .bg(p.surface0)
                        .add_modifier(Modifier::BOLD),
                    Style::default()
                        .fg(p.text)
                        .bg(p.surface0)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    Style::default().fg(p.subtext0),
                    Style::default().fg(p.overlay1),
                )
            };
            let marker_style = if selected {
                marker_style.bg(p.surface0).add_modifier(Modifier::BOLD)
            } else {
                marker_style
            };
            let selector = if selected { ">" } else { " " };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!(" {selector} "), row_style),
                    Span::styled(format!("{marker} "), marker_style),
                    Span::styled(format!("{:<9}", item.label), label_style),
                    Span::styled(item.status_label(), status_style),
                ]))
                .style(row_style),
                Rect::new(list_area.x, next_row_y, list_area.width, 1),
            );
            next_row_y = next_row_y.saturating_add(1);
        }
    }

    let mut tail_lines = Vec::new();
    if !app.integration_install_messages.is_empty() {
        tail_lines.push(Line::from(""));
        for message in &app.integration_install_messages {
            tail_lines.push(Line::from(Span::styled(
                format!(" {message}"),
                Style::default().fg(p.overlay1),
            )));
        }
    } else {
        tail_lines.push(Line::from(""));
        let found_any = app.integration_recommendations.iter().any(|item| {
            item.available || item.state != crate::integration::IntegrationStatusKind::NotInstalled
        });
        let hint = if app
            .integration_recommendations
            .iter()
            .any(crate::integration::IntegrationRecommendation::needs_install)
        {
            " press install to add available or outdated integrations"
        } else if found_any {
            " all detected integrations are installed"
        } else {
            " no supported agent CLIs found on PATH"
        };
        tail_lines.push(Line::from(Span::styled(
            hint,
            Style::default().fg(p.overlay1),
        )));
    }

    if next_row_y < list_bottom {
        frame.render_widget(
            Paragraph::new(tail_lines),
            Rect::new(
                list_area.x,
                next_row_y,
                list_area.width,
                list_bottom - next_row_y,
            ),
        );
    }
}

fn render_settings_theme(app: &AppState, frame: &mut Frame, area: Rect) {
    use crate::app::state::THEME_NAMES;

    let p = &app.palette;
    let items: Vec<ListItem> = THEME_NAMES
        .iter()
        .map(|name| {
            let is_current = name.to_lowercase().replace([' ', '_'], "-")
                == app.theme_name.to_lowercase().replace([' ', '_'], "-");
            let marker = if is_current { " ✓" } else { "" };
            let swatch = crate::app::state::Palette::from_name(name)
                .map(|theme| theme.accent)
                .unwrap_or(p.subtext0);
            ListItem::new(Line::from(vec![
                Span::styled("■ ", Style::default().fg(swatch)),
                Span::styled(
                    crate::app::state::theme_display_name(name),
                    Style::default().fg(p.subtext0),
                ),
                Span::styled(marker, Style::default().fg(p.green)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(p.surface0)
                .fg(p.text)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(" ▸ ")
        .style(Style::default().fg(p.subtext0));

    let mut state = ListState::default().with_selected(Some(app.settings.list.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_settings_toggle(
    frame: &mut Frame,
    area: Rect,
    p: &Palette,
    title: &str,
    description: &str,
    current_value: bool,
    selected_idx: usize,
) {
    render_modal_choice_list(
        frame,
        area,
        title,
        description,
        &[("on", true), ("off", false)],
        current_value,
        selected_idx,
        p,
        1,
    );
}

/// Row offsets inside the Sound section's content area. `app::input::settings`
/// hit-tests clicks against these, so the two must move together.
pub(crate) const SOUND_ALERT_ROWS_OFFSET: u16 = 2;
pub(crate) const SOUND_CHOICE_ROWS_OFFSET: u16 = 6;

fn render_settings_sound(app: &AppState, frame: &mut Frame, area: Rect) {
    let p = &app.palette;

    super::widgets::render_modal_description(
        frame,
        Rect::new(area.x, area.y, area.width, 1),
        "play sounds when agents change state in background",
        Style::default().fg(p.overlay1),
    );

    let mut rows = vec![
        ("sound alerts: on".to_string(), app.sound_enabled()),
        ("sound alerts: off".to_string(), !app.sound_enabled()),
    ];

    let header_y = area.y + SOUND_CHOICE_ROWS_OFFSET - 1;
    if header_y < area.y + area.height {
        let header = if app.sound_enabled() {
            "done sound — selecting one plays it"
        } else {
            "done sound — turn sound alerts on to hear these"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {header}"),
                Style::default().fg(p.overlay1),
            )),
            Rect::new(area.x, header_y, area.width, 1),
        );
    }

    let active_idx = app.selected_done_sound_index();
    rows.extend(
        app.done_sound_choices()
            .into_iter()
            .enumerate()
            .map(|(idx, choice)| {
                (
                    format!("{} — {}", choice.label(), choice.description()),
                    idx == active_idx,
                )
            }),
    );

    for (idx, (text, active)) in rows.into_iter().enumerate() {
        let offset = if idx < 2 {
            SOUND_ALERT_ROWS_OFFSET + idx as u16
        } else {
            SOUND_CHOICE_ROWS_OFFSET + (idx - 2) as u16
        };
        if offset >= area.height {
            continue;
        }
        let style = if idx == app.settings.list.selected {
            Style::default()
                .bg(p.surface0)
                .fg(p.text)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.subtext0)
        };
        let marker = if active { " ✓" } else { "" };
        frame.render_widget(
            Paragraph::new(format!(" {text}{marker}")).style(style),
            Rect::new(area.x, area.y + offset, area.width, 1),
        );
    }
}

fn render_settings_experiments(app: &AppState, frame: &mut Frame, area: Rect) {
    let p = &app.palette;
    let [desc_area, _, list_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas::<3>(area);

    super::widgets::render_modal_description(
        frame,
        desc_area,
        "optional features that are off by default",
        Style::default().fg(p.overlay1),
    );

    for (idx, setting) in ExperimentSetting::ALL.iter().copied().enumerate() {
        let marker = if setting.enabled(app) { "[✓]" } else { "[ ]" };
        let style = if app.settings.list.selected == idx {
            Style::default()
                .bg(p.surface0)
                .fg(p.text)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.subtext0)
        };
        let row = Rect::new(list_area.x, list_area.y + idx as u16, list_area.width, 1);
        frame.render_widget(
            Paragraph::new(format!(" {} {marker}", setting.label())).style(style),
            row,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{state::SettingsSection, Mode};
    use crate::integration::IntegrationStatusKind;
    use ratatui::{backend::TestBackend, Terminal};

    fn rendered_settings(app: &AppState) -> String {
        let mut terminal =
            Terminal::new(TestBackend::new(90, 26)).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_settings_overlay(app, frame, Rect::new(0, 0, 90, 26)))
            .expect("settings overlay should render");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn sound_section_lists_every_done_sound_and_marks_the_active_one() {
        let mut app = AppState::test_new();
        app.sound.enabled = true;
        app.sound.done = Some("bell".to_string());
        app.settings.section = SettingsSection::Sound;
        app.mode = Mode::Settings;

        let rendered = rendered_settings(&app);

        for sound in crate::sound::DONE_SOUNDS {
            assert!(
                rendered.contains(sound.key),
                "expected {} in the picker: {rendered}",
                sound.key
            );
        }
        assert!(rendered.contains("bell — a struck bell, ringing out ✓"));
        assert!(rendered.contains("done sound — selecting one plays it"));
    }

    #[test]
    fn sound_section_leads_with_a_configured_custom_file() {
        let mut app = AppState::test_new();
        app.sound.enabled = true;
        app.sound.done_path = Some(std::path::PathBuf::from("/tmp/herdr-test/fanfare.mp3"));
        app.settings.section = SettingsSection::Sound;
        app.mode = Mode::Settings;

        let rendered = rendered_settings(&app);

        assert!(rendered.contains("fanfare.mp3 — your own mp3, set in the config file ✓"));
    }

    #[test]
    fn sound_section_fits_a_custom_file_and_every_built_in_on_a_short_terminal() {
        let mut app = AppState::test_new();
        app.sound.enabled = true;
        app.sound.done_path = Some(std::path::PathBuf::from("/tmp/herdr-test/fanfare.mp3"));
        app.settings.section = SettingsSection::Sound;
        app.mode = Mode::Settings;

        let mut terminal =
            Terminal::new(TestBackend::new(80, 24)).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_settings_overlay(&app, frame, Rect::new(0, 0, 80, 24)))
            .expect("settings overlay should render");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("fanfare.mp3"));
        let last = crate::sound::DONE_SOUNDS
            .last()
            .expect("there is at least one built-in sound");
        assert!(
            rendered.contains(last.description),
            "the last sound row was clipped: {rendered}"
        );
    }

    #[test]
    fn sound_section_says_previews_are_muted_when_alerts_are_off() {
        let mut app = AppState::test_new();
        app.sound.enabled = false;
        app.settings.section = SettingsSection::Sound;
        app.mode = Mode::Settings;

        let rendered = rendered_settings(&app);

        assert!(rendered.contains("done sound — turn sound alerts on to hear these"));
    }

    #[test]
    fn experiments_pane_history_uses_settings_checkmark_marker() {
        let mut app = AppState::test_new();
        app.pane_history_persistence = true;
        app.settings.section = SettingsSection::Experiments;
        app.settings.list.selected = 0;
        app.mode = Mode::Settings;

        let mut terminal =
            Terminal::new(TestBackend::new(80, 24)).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_settings_overlay(&app, frame, Rect::new(0, 0, 80, 24)))
            .expect("settings overlay should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("pane screen history [✓]"));
        assert!(!rendered.contains("[x]"));
    }

    #[test]
    fn experiments_pane_history_keeps_empty_checkbox_marker_when_disabled() {
        let mut app = AppState::test_new();
        app.pane_history_persistence = false;
        app.settings.section = SettingsSection::Experiments;
        app.settings.list.selected = 0;
        app.mode = Mode::Settings;

        let mut terminal =
            Terminal::new(TestBackend::new(80, 24)).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_settings_overlay(&app, frame, Rect::new(0, 0, 80, 24)))
            .expect("settings overlay should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("pane screen history [ ]"));
    }

    #[test]
    fn experiments_renders_switch_ascii_input_source_row() {
        let mut app = AppState::test_new();
        app.switch_ascii_input_source_in_prefix = true;
        app.settings.section = SettingsSection::Experiments;
        app.settings.list.selected = 1;
        app.mode = Mode::Settings;

        let mut terminal =
            Terminal::new(TestBackend::new(80, 24)).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_settings_overlay(&app, frame, Rect::new(0, 0, 80, 24)))
            .expect("settings overlay should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("switch to ascii input source in prefix (macOS) [✓]"));
    }

    fn integration_rec(
        label: &'static str,
        state: IntegrationStatusKind,
        available: bool,
    ) -> crate::integration::IntegrationRecommendation {
        crate::integration::IntegrationRecommendation {
            target: crate::api::schema::IntegrationTarget::Codex,
            label,
            command: label,
            available,
            path: std::path::PathBuf::from("/tmp/herdr-test"),
            state,
        }
    }

    #[test]
    fn integrations_marks_the_selected_row_with_an_ascii_selector_and_full_row_highlight() {
        let mut app = AppState::test_new();
        app.mode = Mode::Settings;
        app.settings.section = SettingsSection::Integrations;
        app.settings.list.selected = 1;
        app.integration_recommendations = vec![
            integration_rec("claude code", IntegrationStatusKind::Current, false),
            integration_rec("codex", IntegrationStatusKind::NotInstalled, true),
        ];

        let mut terminal =
            Terminal::new(TestBackend::new(90, 26)).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_settings_overlay(&app, frame, Rect::new(0, 0, 90, 26)))
            .expect("settings overlay should render");

        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..26u16)
            .map(|y| {
                (0..90u16)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect();

        let selected_y = rows
            .iter()
            .position(|row| row.contains('>'))
            .expect("the selected integration row shows an ascii > selector")
            as u16;
        let selected_row = &rows[selected_y as usize];
        assert!(
            selected_row.contains("> + codex"),
            "selected row: {selected_row}"
        );
        assert!(
            selected_row.contains("available"),
            "selected row: {selected_row}"
        );

        let current_y = rows
            .iter()
            .position(|row| row.contains("claude code"))
            .expect("the installed integration row renders") as u16;
        assert!(!rows[current_y as usize].contains('>'));

        let surface0 = app.palette.surface0;
        for x in 10..70u16 {
            assert_eq!(
                buffer[(x, selected_y)].bg,
                surface0,
                "row {selected_y} col {x} should carry the full-row selected background"
            );
        }
        assert_eq!(
            buffer[(10, current_y)].bg,
            app.palette.panel_bg,
            "unselected rows should keep the panel background"
        );

        let selector_x = (0..90u16)
            .find(|&x| buffer[(x, selected_y)].symbol() == ">")
            .expect("a selector cell on the selected row");
        assert_eq!(
            buffer[(selector_x + 2, selected_y)].fg,
            app.palette.accent,
            "the available marker keeps its accent color on the selected row"
        );
        assert!(
            buffer[(selector_x + 4, selected_y)]
                .modifier
                .contains(Modifier::BOLD),
            "selected row text is bold"
        );
    }

    #[test]
    fn settings_footer_and_primary_button_hints_are_ascii() {
        let mut app = AppState::test_new();
        app.mode = Mode::Settings;
        app.settings.section = SettingsSection::Sound;

        let rendered = rendered_settings(&app);

        assert!(rendered.contains("enter apply"));
        assert!(rendered.contains("esc close"));
        assert!(rendered.contains("up/down move"));
        assert!(rendered.contains("tab switch section"));
        assert!(!rendered.contains('↵'));
        assert!(!rendered.contains('↑'));

        app.settings.section = SettingsSection::Integrations;
        app.integration_recommendations = vec![integration_rec(
            "codex",
            IntegrationStatusKind::NotInstalled,
            true,
        )];
        let rendered = rendered_settings(&app);
        assert!(rendered.contains("enter install"));
        assert!(rendered.contains("esc close"));
    }
}
