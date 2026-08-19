//! Hit testing for the agent table and the global menu that sits on it.
//!
//! The table draws itself from [`crate::app::state::ViewState`], and everything
//! a click can land on is already in there: each row knows which agent it is
//! and which cells it covers. So there is nothing to measure a second time —
//! this reads the same layout the last frame drew.

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use crate::app::state::{
    AgentTableFocus, AppState, ContextMenuKind, Mode, PendingAgentClose, SpaceMenuKind, ViewLayout,
};
use crate::input::TerminalKey;
use crate::terminal::TerminalRuntimeRegistry;

/// The width the launcher takes at the right end of the top row, with and
/// without the badge that says something is waiting.
const LAUNCHER_WIDTH: u16 = 6;
const LAUNCHER_BADGE_WIDTH: u16 = 8;

/// Whether a key belongs to the row a click just picked out, and acting on it
/// if it does.
///
/// Only the delete key does. It opens the question that removes the agent, and
/// releases the hold either way: every other key releases the hold and is
/// passed on untouched, so a click on a row followed by typing still types into
/// that agent. Backspace counts as delete because the key labeled delete on a
/// Mac keyboard sends backspace.
pub(crate) fn agent_table_delete_intercept(state: &mut AppState, key: TerminalKey) -> bool {
    if state.agent_table_focus.is_none() {
        return false;
    }
    let key_event = key.as_key_event();
    if key_event.kind == KeyEventKind::Release || matches!(key_event.code, KeyCode::Modifier(_)) {
        return false;
    }
    let deletes = matches!(key_event.code, KeyCode::Delete | KeyCode::Backspace)
        && (key_event.modifiers - KeyModifiers::SHIFT).is_empty();
    let Some(focus) = state.agent_table_focus.take() else {
        return false;
    };
    if !deletes {
        return false;
    }
    open_confirm_close_agent(state, focus)
}

/// Asks whether to remove the agent a row stands for. Returns whether the
/// question opened; an agent the table no longer lists has nothing to ask
/// about.
pub(crate) fn open_confirm_close_agent(state: &mut AppState, focus: AgentTableFocus) -> bool {
    let Some(name) = crate::ui::agent_panel_entries(state)
        .into_iter()
        .find(|entry| entry.pane_id == focus.pane_id)
        .map(|entry| entry.name)
    else {
        return false;
    };
    state.confirm_close_agent = Some(PendingAgentClose {
        docked: focus.docked,
        pane_id: focus.pane_id,
        name,
    });
    state.mode = Mode::ConfirmCloseAgent;
    true
}

/// Removes the agent the question named: a docked agent is ended in its pane,
/// the way its menu ends it, and a set-down agent is dropped outright.
pub(crate) fn confirm_close_agent_accept(
    state: &mut AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) {
    if let Some(pending) = state.confirm_close_agent.take() {
        if pending.docked {
            state.close_agent_in_pane(terminal_runtimes, pending.pane_id);
        } else {
            state.close_detached_agent(pending.pane_id);
        }
    }
    confirm_close_agent_cancel(state);
}

pub(crate) fn handle_confirm_close_agent_key(
    state: &mut AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    key: crossterm::event::KeyEvent,
) {
    use super::modal::{modal_action_from_key, ModalAction, CONFIRM_CLOSE_ACTIONS};
    match modal_action_from_key(&key, CONFIRM_CLOSE_ACTIONS) {
        Some(ModalAction::Confirm) => confirm_close_agent_accept(state, terminal_runtimes),
        Some(ModalAction::Cancel) => confirm_close_agent_cancel(state),
        _ => {}
    }
}

pub(crate) fn confirm_close_agent_cancel(state: &mut AppState) {
    state.confirm_close_agent = None;
    state.mode = if state.active.is_some() {
        Mode::Terminal
    } else {
        Mode::Navigate
    };
}

impl AppState {
    /// The button that opens the global menu: the far right of the frame's top
    /// row, aligned with the composer captions.
    pub(crate) fn global_launcher_rect(&self) -> Rect {
        if self.view.layout == ViewLayout::Mobile {
            return self.view.mobile_menu_hit_area;
        }

        let screen = self.screen_rect();
        if screen.width == 0 || screen.height == 0 {
            return Rect::default();
        }
        let width = if self.global_menu_attention_badge_visible() {
            LAUNCHER_BADGE_WIDTH
        } else {
            LAUNCHER_WIDTH
        }
        .min(screen.width.max(1));
        let x = screen.x + screen.width.saturating_sub(width);
        Rect::new(x, screen.y, width, 1)
    }

    pub(crate) fn global_menu_labels(&self) -> Vec<&'static str> {
        let mut labels = vec!["settings", "keybinds", "reload config"];
        if self.latest_release_notes_available {
            labels.push("what's new");
        }
        labels.push("detach");
        labels
    }

    /// The menu itself, hanging under the launcher. It opens downwards because
    /// the launcher sits at the top of the frame now, and a menu that opened
    /// upwards from there would have nowhere to go.
    pub(crate) fn global_menu_rect(&self) -> Rect {
        let screen = self.screen_rect();
        let launcher = self.global_launcher_rect();
        let labels = self.global_menu_labels();
        let content_width = labels
            .iter()
            .map(|label| {
                let badge_width = if self.global_menu_item_has_badge(label) {
                    2
                } else {
                    0
                };
                label.chars().count() as u16 + badge_width
            })
            .max()
            .unwrap_or(8)
            .saturating_add(2);
        let menu_w = content_width.saturating_add(2).min(screen.width.max(1));
        let menu_h = (labels.len() as u16 + 2).min(screen.height.max(1));
        let max_x = screen.x + screen.width.saturating_sub(menu_w);
        let desired_x = launcher.x + launcher.width.saturating_sub(menu_w);
        let x = desired_x.min(max_x);
        let max_y = screen.y + screen.height.saturating_sub(menu_h);
        let y = launcher.y.saturating_add(1).min(max_y);
        Rect::new(x, y, menu_w, menu_h)
    }

    /// The agent under a point, if a row is.
    pub(super) fn agent_table_target_at(
        &self,
        col: u16,
        row: u16,
    ) -> Option<crate::ui::AgentTableRow> {
        self.view.agent_table.row_at(col, row).cloned()
    }

    /// The agent whose name cell contains a point. Other cells keep their
    /// row-wide focus and drag behavior without becoming rename shortcuts.
    pub(super) fn agent_name_target_at(
        &self,
        col: u16,
        row: u16,
    ) -> Option<crate::ui::AgentTableRow> {
        let hit = self.view.agent_table.row_at(col, row)?;
        let name = self
            .view
            .agent_table
            .groups
            .get(hit.group)?
            .columns
            .first()?;
        (col >= name.x && col < name.x.saturating_add(name.width) && row == hit.rect.y)
            .then(|| hit.clone())
    }

    /// The agent whose done marker contains a point. The marker sits in the
    /// gutter left of the first column, and the whole gutter counts, because a
    /// one-cell target in a terminal is a target you miss.
    pub(super) fn agent_marker_target_at(
        &self,
        col: u16,
        row: u16,
    ) -> Option<crate::ui::AgentTableRow> {
        let hit = self.view.agent_table.row_at(col, row)?;
        let first = self
            .view
            .agent_table
            .groups
            .get(hit.group)?
            .columns
            .first()?;
        (col >= hit.rect.x && col < first.x && row == hit.rect.y).then(|| hit.clone())
    }

    /// Whether a point is anywhere in the table's band, rows and heading alike.
    /// A click that lands there is the table's, not the pane's below it.
    pub(super) fn in_agent_table(&self, col: u16, row: u16) -> bool {
        let table = self.view.agent_table.area;
        table.width > 0
            && table.height > 0
            && col >= table.x
            && col < table.x.saturating_add(table.width)
            && row >= table.y
            && row < table.y.saturating_add(table.height)
    }

    /// Insert-before index represented by a pointer over a row. Since terminal
    /// rows have no fractional vertical coordinate, moving down means after the
    /// pointed row and moving up means before it.
    pub(super) fn agent_drop_index_at(
        &self,
        col: u16,
        row: u16,
        source_pane_id: crate::layout::PaneId,
    ) -> Option<usize> {
        let hit = self.view.agent_table.row_at(col, row)?;
        let source = crate::ui::agent_panel_entries(self)
            .iter()
            .position(|entry| entry.pane_id == source_pane_id)?;
        Some(if hit.entry_idx > source {
            hit.entry_idx + 1
        } else {
            hit.entry_idx
        })
    }

    /// The menu a table row opens: the agent's own actions, then its space's.
    pub(super) fn agent_menu_kind(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> ContextMenuKind {
        let space = self
            .workspaces
            .get(ws_idx)
            .map(|ws| {
                let git_space = ws.git_space().cloned().or_else(|| {
                    ws.resolved_identity_cwd_from(&self.terminals, terminal_runtimes)
                        .as_deref()
                        .and_then(crate::workspace::git_space_metadata)
                });
                let linked = ws.worktree_space().map_or_else(
                    || {
                        git_space
                            .as_ref()
                            .is_some_and(|space| space.is_linked_worktree)
                    },
                    |space| space.is_linked_worktree,
                );
                if linked {
                    let membership = ws.worktree_space().filter(|space| space.is_linked_worktree);
                    let checkout_path = membership
                        .map(|space| space.checkout_path.clone())
                        .or_else(|| git_space.as_ref().map(|space| space.repo_root.clone()));
                    let parent_path =
                        membership.map(|space| space.repo_root.clone()).or_else(|| {
                            checkout_path
                                .as_deref()
                                .and_then(crate::workspace::composer_folder_path)
                        });
                    let parent_branch = parent_path
                        .as_ref()
                        .and_then(|path| crate::workspace::git_branch(path));
                    let already_landed = checkout_path
                        .as_ref()
                        .zip(parent_path.as_ref())
                        .is_some_and(|(checkout, parent)| {
                            crate::worktree::worktree_already_landed(checkout, parent)
                        });
                    let pane_cwd = ws
                        .find_tab_index_for_pane(pane_id)
                        .and_then(|idx| ws.tabs.get(idx))
                        .and_then(|tab| {
                            tab.cwd_for_pane(pane_id, &self.terminals, terminal_runtimes)
                        });
                    let in_worktree_directory = match pane_cwd.as_deref() {
                        Some(cwd) => crate::workspace::git_space_metadata(cwd)
                            .is_some_and(|space| space.is_linked_worktree),
                        None => true,
                    };
                    SpaceMenuKind::LinkedWorktree {
                        parent_branch,
                        already_landed,
                        in_worktree_directory,
                    }
                } else if ws.worktree_space().is_some() || git_space.is_some() {
                    SpaceMenuKind::Repo
                } else {
                    SpaceMenuKind::Plain
                }
            })
            .unwrap_or(SpaceMenuKind::Plain);
        let can_promote = self
            .workspaces
            .get(ws_idx)
            .is_some_and(|ws| ws.layout.pane_count() > 1);
        ContextMenuKind::Agent {
            ws_idx,
            pane_id,
            can_promote,
            space,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::input::{app_for_mouse_test, mouse};
    use crate::app::state::{ContextMenuKind, SpaceMenuKind};
    use crate::workspace::Workspace;
    use crossterm::event::{KeyEvent, MouseButton, MouseEventKind};
    use ratatui::layout::Rect;

    fn app_with_one_agent() -> (crate::app::App, crate::layout::PaneId) {
        let mut app = app_for_mouse_test();
        let workspace = Workspace::test_new("space");
        let pane_id = workspace.tabs[0].root_pane;
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        let terminal_id = app.state.workspaces[0]
            .pane_state(pane_id)
            .expect("agent pane")
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("agent terminal")
            .set_agent_name("codex".into());
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        (app, pane_id)
    }

    fn click_first_row(app: &mut crate::app::App) {
        let row = app.state.view.agent_table.rows[0].clone();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            row.rect.x + 4,
            row.rect.y,
        ));
    }

    fn key(code: KeyCode) -> TerminalKey {
        TerminalKey::new(code, KeyModifiers::empty())
    }

    #[test]
    fn delete_after_clicking_a_row_asks_before_removing_the_agent() {
        let (mut app, pane_id) = app_with_one_agent();
        click_first_row(&mut app);
        assert_eq!(
            app.state.agent_table_focus,
            Some(AgentTableFocus {
                docked: true,
                pane_id
            })
        );

        assert!(agent_table_delete_intercept(
            &mut app.state,
            key(KeyCode::Delete)
        ));
        assert_eq!(app.state.mode, Mode::ConfirmCloseAgent);
        let pending = app.state.confirm_close_agent.clone().expect("question");
        assert_eq!(pending.pane_id, pane_id);
        assert!(pending.docked);
        assert!(app.state.agent_table_focus.is_none());
    }

    #[test]
    fn escape_answers_the_question_without_removing_the_agent() {
        let (mut app, _) = app_with_one_agent();
        click_first_row(&mut app);
        agent_table_delete_intercept(&mut app.state, key(KeyCode::Delete));

        handle_confirm_close_agent_key(
            &mut app.state,
            &app.terminal_runtimes,
            KeyEvent::from(KeyCode::Esc),
        );

        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.state.confirm_close_agent.is_none());
        assert_eq!(app.state.workspaces[0].layout.pane_count(), 1);
    }

    #[test]
    fn any_other_key_releases_the_row_and_is_passed_on() {
        let (mut app, _) = app_with_one_agent();
        click_first_row(&mut app);

        assert!(!agent_table_delete_intercept(
            &mut app.state,
            key(KeyCode::Char('h'))
        ));
        assert!(app.state.agent_table_focus.is_none());
        assert_eq!(app.state.mode, Mode::Terminal);

        assert!(!agent_table_delete_intercept(
            &mut app.state,
            key(KeyCode::Delete)
        ));
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn delete_without_a_clicked_row_reaches_the_pane() {
        let (mut app, _) = app_with_one_agent();
        assert!(!agent_table_delete_intercept(
            &mut app.state,
            key(KeyCode::Delete)
        ));
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn removing_a_set_down_agent_drops_it_from_the_table() {
        let (mut app, _) = app_with_one_agent();
        let split_pane = app.state.workspaces[0].test_split(ratatui::layout::Direction::Horizontal);
        app.state.ensure_test_terminals();
        let split_terminal = app.state.workspaces[0]
            .pane_state(split_pane)
            .expect("split pane")
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&split_terminal)
            .expect("split terminal")
            .set_agent_name("codex".into());
        app.state.close_pane();
        assert_eq!(app.state.detached_agents.len(), 1);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let row = app
            .state
            .view
            .agent_table
            .rows
            .iter()
            .find(|row| !row.docked)
            .cloned()
            .expect("set-down row");
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            row.rect.x + 4,
            row.rect.y,
        ));

        assert!(agent_table_delete_intercept(
            &mut app.state,
            key(KeyCode::Delete)
        ));
        confirm_close_agent_accept(&mut app.state, &app.terminal_runtimes);

        assert!(app.state.detached_agents.is_empty());
        assert!(app.state.confirm_close_agent.is_none());
    }

    #[test]
    fn agent_menu_names_the_parent_checkout_branch() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let repo =
            std::env::temp_dir().join(format!("herdr-land-menu-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&repo).unwrap();
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        run(&["init", "--quiet", "-b", "release"]);
        run(&["config", "user.email", "herdr@example.invalid"]);
        run(&["config", "user.name", "Herdr Test"]);
        std::fs::write(repo.join("README.md"), "test\n").unwrap();
        run(&["add", "README.md"]);
        run(&["commit", "--quiet", "-m", "initial"]);

        let (app, pane_id) = app_with_one_agent();
        let mut state = app.state;
        state.workspaces[0].worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: repo.clone(),
            checkout_path: repo.join("worktree"),
            is_linked_worktree: true,
        });

        let kind = state.agent_menu_kind(&app.terminal_runtimes, 0, pane_id);
        match kind {
            ContextMenuKind::Agent {
                space:
                    SpaceMenuKind::LinkedWorktree {
                        parent_branch,
                        already_landed,
                        ..
                    },
                ..
            } => {
                assert_eq!(parent_branch.as_deref(), Some("release"));
                assert!(!already_landed);
            }
            other => panic!("unexpected menu kind: {other:?}"),
        }

        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn agent_menu_disables_land_when_worktree_shares_parent_commit() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let repo = std::env::temp_dir().join(format!(
            "herdr-landed-menu-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&repo).unwrap();
        let run = |cwd: &std::path::Path, args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(cwd)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        run(&repo, &["init", "--quiet", "-b", "main"]);
        run(&repo, &["config", "user.email", "herdr@example.invalid"]);
        run(&repo, &["config", "user.name", "Herdr Test"]);
        std::fs::write(repo.join("README.md"), "test\n").unwrap();
        run(&repo, &["add", "README.md"]);
        run(&repo, &["commit", "--quiet", "-m", "initial"]);
        let checkout = repo.join("worktree");
        run(
            &repo,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "worktree/landed-menu",
                checkout.to_str().unwrap(),
            ],
        );

        let (app, pane_id) = app_with_one_agent();
        let mut state = app.state;
        state.workspaces[0].worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: repo.clone(),
            checkout_path: checkout.clone(),
            is_linked_worktree: true,
        });

        let kind = state.agent_menu_kind(&app.terminal_runtimes, 0, pane_id);
        match kind {
            ContextMenuKind::Agent {
                space: SpaceMenuKind::LinkedWorktree { already_landed, .. },
                ..
            } => assert!(already_landed),
            other => panic!("unexpected menu kind: {other:?}"),
        }

        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["worktree", "remove", "--force", checkout.to_str().unwrap()])
            .status();
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn agent_menu_grays_worktree_delete_when_pane_cwd_is_not_a_worktree() {
        let cwd = std::env::temp_dir().join(format!(
            "herdr-plain-cwd-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&cwd).unwrap();

        let (app, pane_id) = app_with_one_agent();
        let mut state = app.state;
        state.workspaces[0].worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: cwd.join("parent"),
            checkout_path: cwd.join("worktree"),
            is_linked_worktree: true,
        });
        let terminal_id = state.workspaces[0]
            .pane_state(pane_id)
            .expect("agent pane")
            .attached_terminal_id
            .clone();
        state
            .terminals
            .get_mut(&terminal_id)
            .expect("agent terminal")
            .cwd = cwd.clone();

        let kind = state.agent_menu_kind(&app.terminal_runtimes, 0, pane_id);
        let menu = crate::app::state::ContextMenuState {
            kind,
            x: 0,
            y: 0,
            list: crate::app::state::MenuListState::new(0),
        };
        match &menu.kind {
            ContextMenuKind::Agent {
                space:
                    SpaceMenuKind::LinkedWorktree {
                        in_worktree_directory,
                        ..
                    },
                ..
            } => assert!(
                !*in_worktree_directory,
                "a pane whose cwd is not a checkout is not in a worktree directory"
            ),
            other => panic!("unexpected menu kind: {other:?}"),
        }
        let delete_idx = menu
            .items()
            .iter()
            .position(|item| item == "Delete agent / worktree...")
            .expect("delete worktree item");
        assert_eq!(delete_idx, menu.items().len() - 1);
        assert!(!menu.item_enabled(delete_idx));

        let _ = std::fs::remove_dir_all(cwd);
    }
}
