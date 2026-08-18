//! Start an agent on what the composer holds.
//!
//! The task goes on the harness's command line rather than being typed into it
//! once it is up. An agent started that way is working the moment it draws, and
//! there is nothing to wait for and nothing that can be swallowed by a prompt
//! that was not listening yet.
//!
//! The agent starts hidden. Starting work is not the same as wanting to watch
//! it, and a band that rearranges the screen every time it is used cannot be
//! used while something else is being read. The agent runs, its row appears in
//! the table, and the panes on screen stay exactly as they were. Dragging its
//! row onto a pane is what gives it one.

use std::path::PathBuf;

use crate::app::state::{ToastKind, ToastNotification, ToastTarget};
use crate::app::App;
use crate::composer::Pending;
use crate::harness::Launch;

impl App {
    /// Start an agent on the task in the band, and empty the field.
    ///
    /// The field is emptied only once the agent is running. A task that could
    /// not be started is a task still worth having, and retyping it is the one
    /// thing the band exists to avoid.
    pub(crate) fn submit_composer(&mut self, pending: Pending) {
        let result = match pending.launch() {
            Launch::Agent { agent, argv } => {
                self.start_hidden_agent(pending.cwd.clone(), &argv, Some(agent))
            }
            Launch::Terminal { command } => {
                self.start_hidden_terminal(pending.cwd.clone(), &command)
            }
        }
        .map_err(|err| self.agent_start_error_body(err).message);
        match result {
            Ok(_pane_id) => {
                self.state.composer.task.clear();
                self.state.composer.add_folder(pending.cwd.clone());
                let where_it_went = crate::workspace::display_path_with_home(&pending.cwd);
                self.show_composer_toast(
                    ToastKind::Finished,
                    &format!("started {} in {where_it_went}", pending.harness.name),
                    pending.message(),
                    None,
                );
            }
            Err(reason) => {
                self.show_composer_toast(
                    ToastKind::NeedsAttention,
                    &format!("could not start {}", pending.harness.name),
                    reason,
                    None,
                );
            }
        }
    }

    /// Say why a key could not do what it asked for. The band keeps whatever
    /// was typed: a task refused is a task still worth having.
    pub(crate) fn show_composer_trouble(&mut self, reason: String) {
        self.show_composer_toast(ToastKind::NeedsAttention, "composer", reason, None);
    }

    fn show_composer_toast(
        &mut self,
        kind: ToastKind,
        title: &str,
        context: String,
        target: Option<ToastTarget>,
    ) {
        let previous_toast = self.state.toast.clone();
        self.state.toast = Some(ToastNotification {
            kind,
            title: title.to_string(),
            context,
            target,
        });
        self.sync_toast_deadline(previous_toast);
    }
}

impl crate::app::AppState {
    /// The folders the composer offers: every folder something is working in,
    /// most recently used first, with the folder already on show kept where it
    /// is. A hidden agent counts, because work is work whether or not a pane
    /// happens to be showing it.
    ///
    /// It is rebuilt from what is running rather than remembered separately, so
    /// a space that closes takes its folder off the list and there is no second
    /// record of where the work is to fall out of step with the first. A folder
    /// typed by hand is the one exception: it is kept until something runs in
    /// it, because it was put there to be started in.
    ///
    /// This is asked for at the moments the list is consulted — reaching the
    /// band, and opening its folder list — rather than on every frame. A pane's
    /// working directory is read from `/proc` behind a lock the pane's own
    /// reader holds, so asking sixty times a second for a list nobody is
    /// looking at slows every pane down to keep a menu warm.
    pub(crate) fn refresh_composer_folders(
        &mut self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) {
        let mut folders: Vec<PathBuf> = Vec::new();
        for ws in &self.workspaces {
            for tab in &ws.tabs {
                for pane_id in tab.layout.pane_ids() {
                    let Some(cwd) = tab.cwd_for_pane(pane_id, &self.terminals, terminal_runtimes)
                    else {
                        continue;
                    };
                    if !folders.contains(&cwd) {
                        folders.push(cwd);
                    }
                }
            }
        }
        for detached in &self.detached_agents {
            let terminal_id = &detached.pane.attached_terminal_id;
            let Some(cwd) = terminal_runtimes
                .get(terminal_id)
                .and_then(|runtime| runtime.cwd())
                .or_else(|| {
                    self.terminals
                        .get(terminal_id)
                        .map(|terminal| terminal.cwd.clone())
                })
            else {
                continue;
            };
            if !folders.contains(&cwd) {
                folders.push(cwd);
            }
        }
        if let Some(showing) = self.composer.folder_path() {
            if !folders.iter().any(|folder| folder == showing) {
                folders.insert(0, showing.to_path_buf());
            }
        }
        self.composer.set_folders(folders);
    }
}
