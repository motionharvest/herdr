//! The parts of herdr's interface that can be folded away and shown again.
//!
//! Each part keeps its own flag in [`AppState`], saved with the session, and
//! everything that folds one goes through here: a click, a key, the context
//! menu, the socket API's `view.set`, and the Commander. A part is either
//! shown or not, so showing, hiding and toggling are the same change.

use crate::app::state::AppState;

/// One part of the interface that folds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fold {
    /// The whole sidebar, down to a narrow strip.
    Sidebar,
    /// The sidebar's spaces section, down to its header and the space on
    /// screen.
    Spaces,
    /// The agent table, down to its one-row header.
    AgentTable,
    /// The agent and pane rows listed under a space's card.
    SpaceAgents(usize),
    /// The worktree spaces listed under their repository's card, by the
    /// group's key.
    SpaceGroup(String),
    /// A tab's layout preview under its name in the sidebar.
    Minimap { ws: usize, tab: usize },
}

impl AppState {
    pub(crate) fn is_shown(&self, fold: &Fold) -> bool {
        match fold {
            Fold::Sidebar => !self.sidebar_collapsed,
            Fold::Spaces => !self.spaces_collapsed,
            Fold::AgentTable => !self.agent_table_collapsed,
            Fold::SpaceAgents(ws) => crate::ui::workspace_agents_expanded(self, *ws),
            Fold::SpaceGroup(key) => !self.collapsed_space_keys.contains(key),
            Fold::Minimap { ws, tab } => self
                .workspaces
                .get(*ws)
                .and_then(|w| w.tabs.get(*tab))
                .is_some_and(|t| !t.layout_preview_hidden),
        }
    }

    /// Show or hide `fold`. Nothing changes, and nothing is saved, when it
    /// already is that way.
    pub(crate) fn set_shown(&mut self, fold: &Fold, shown: bool) {
        if self.is_shown(fold) == shown {
            return;
        }
        match fold {
            Fold::Sidebar => self.sidebar_collapsed = !shown,
            Fold::Spaces => self.spaces_collapsed = !shown,
            Fold::AgentTable => self.agent_table_collapsed = !shown,
            Fold::SpaceAgents(ws) => {
                let Some(id) = self.workspaces.get(*ws).map(|w| w.id.clone()) else {
                    return;
                };
                if shown {
                    self.collapsed_agent_space_ids.remove(&id);
                } else {
                    self.collapsed_agent_space_ids.insert(id);
                }
            }
            Fold::SpaceGroup(key) => {
                if shown {
                    self.collapsed_space_keys.remove(key);
                } else {
                    self.collapsed_space_keys.insert(key.clone());
                }
            }
            Fold::Minimap { ws, tab } => {
                let Some(tab) = self
                    .workspaces
                    .get_mut(*ws)
                    .and_then(|w| w.tabs.get_mut(*tab))
                else {
                    return;
                };
                tab.layout_preview_hidden = !shown;
            }
        }
        self.mark_session_dirty();
        // Folding rows changes how long the spaces list is, so a scroll that
        // was in range may no longer be.
        self.workspace_scroll = crate::ui::normalized_workspace_scroll(
            self,
            self.view.sidebar_rect,
            self.workspace_scroll,
        );
    }

    pub(crate) fn toggle_shown(&mut self, fold: &Fold) {
        let shown = self.is_shown(fold);
        self.set_shown(fold, !shown);
    }

    /// The group a space's card heads or belongs to, when it is a git space
    /// with worktrees listed under one card.
    pub(crate) fn space_group(&self, ws: usize) -> Option<Fold> {
        let key = self.workspaces.get(ws)?.worktree_space()?.key.clone();
        let members = self
            .workspaces
            .iter()
            .filter(|w| w.worktree_space().is_some_and(|m| m.key == key))
            .count();
        (members >= 2).then_some(Fold::SpaceGroup(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;

    #[test]
    fn showing_what_is_shown_changes_nothing() {
        let mut state = AppState::test_new();
        state.set_shown(&Fold::Sidebar, true);
        assert!(!state.sidebar_collapsed);
        state.set_shown(&Fold::Sidebar, false);
        assert!(state.sidebar_collapsed);
        state.toggle_shown(&Fold::Sidebar);
        assert!(!state.sidebar_collapsed);
    }

    #[test]
    fn a_space_folds_its_own_agents_and_minimaps() {
        let mut state = AppState::test_new();
        state.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        state.set_shown(&Fold::SpaceAgents(1), false);
        assert!(state.is_shown(&Fold::SpaceAgents(0)));
        assert!(!state.is_shown(&Fold::SpaceAgents(1)));
        let minimap = Fold::Minimap { ws: 0, tab: 0 };
        state.toggle_shown(&minimap);
        assert!(!state.is_shown(&minimap));
        assert!(state.is_shown(&Fold::Minimap { ws: 1, tab: 0 }));
    }
}
