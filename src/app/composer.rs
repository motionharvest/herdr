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
            Launch::Agent { agent, argv }
                if pending.harness.prefix != crate::harness::AUTO_PREFIX
                    && crate::workspace::git_space_metadata(&pending.cwd)
                        .is_some_and(|space| !space.is_linked_worktree) =>
            {
                self.start_managed_worktree_agent(&pending.cwd, &argv, agent)
            }
            Launch::Agent { agent, argv } => self
                .start_hidden_agent(pending.cwd.clone(), &argv, Some(agent))
                .map(|pane_id| (pane_id, pending.cwd.clone())),
            Launch::Terminal { command } => self
                .start_hidden_terminal(pending.cwd.clone(), &command)
                .map(|pane_id| (pane_id, pending.cwd.clone())),
        }
        .map_err(|err| self.agent_start_error_body(err).message);
        match result {
            Ok((_pane_id, started_cwd)) => {
                self.state.composer.task.clear();
                self.state.composer.add_folder(pending.cwd.clone());
                let where_it_went = crate::workspace::display_path_with_home(&started_cwd);
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
    /// happens to be showing it. A linked worktree is listed as its parent
    /// checkout, so the directory control stays a list of places to start from
    /// rather than a row for every checkout already created.
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
        let mut offer = |cwd: PathBuf| {
            let Some(folder) = crate::workspace::composer_folder_path(&cwd) else {
                return;
            };
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        };
        for ws in &self.workspaces {
            for tab in &ws.tabs {
                for pane_id in tab.layout.pane_ids() {
                    let Some(cwd) = tab.cwd_for_pane(pane_id, &self.terminals, terminal_runtimes)
                    else {
                        continue;
                    };
                    offer(cwd);
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
            offer(cwd);
        }
        if let Some(showing) = self.composer.folder_path() {
            if let Some(folder) = crate::workspace::composer_folder_path(showing) {
                folders.retain(|listed| listed != &folder);
                folders.insert(0, folder);
            }
        }
        self.composer.set_folders(folders);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppState;
    use crate::terminal::{TerminalRuntimeRegistry, TerminalState};
    use crate::workspace::Workspace;

    fn committed_repo(name: &str) -> PathBuf {
        let unique = format!(
            "herdr-composer-folders-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let repo = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            ["init", "--quiet"].as_slice(),
            ["config", "user.email", "herdr@example.invalid"].as_slice(),
            ["config", "user.name", "Herdr Test"].as_slice(),
        ] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        std::fs::write(repo.join("README.md"), "test\n").unwrap();
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "README.md"])
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["commit", "--quiet", "-m", "initial"])
            .status()
            .unwrap()
            .success());
        std::fs::canonicalize(&repo).unwrap()
    }

    fn add_worktree(repo: &std::path::Path, name: &str) -> PathBuf {
        let checkout = repo.parent().unwrap().join(format!(
            "{}-{name}",
            repo.file_name().unwrap().to_string_lossy()
        ));
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args([
                "worktree",
                "add",
                "--quiet",
                "-b",
                &format!("worktree/{name}"),
                checkout.to_str().unwrap(),
            ])
            .status()
            .unwrap()
            .success());
        std::fs::canonicalize(&checkout).unwrap()
    }

    fn state_working_in(cwd: PathBuf) -> AppState {
        let mut state = AppState::test_new();
        let ws = Workspace::test_new("space");
        let pane_id = ws.tabs[0].root_pane;
        let terminal_id = ws.tabs[0].panes[&pane_id].attached_terminal_id.clone();
        state.workspaces = vec![ws];
        state.active = Some(0);
        state
            .terminals
            .insert(terminal_id.clone(), TerminalState::new(terminal_id, cwd));
        state
    }

    #[test]
    fn composer_folders_list_the_parent_instead_of_a_linked_worktree() {
        let repo = committed_repo("parent");
        let checkout = add_worktree(&repo, "agent");
        let mut state = state_working_in(checkout.clone());

        state.refresh_composer_folders(&TerminalRuntimeRegistry::default());

        let paths: Vec<_> = state
            .composer
            .folders
            .iter()
            .map(|folder| folder.path.clone())
            .collect();
        assert!(
            paths.contains(&repo),
            "the parent checkout should stay pickable"
        );
        assert!(
            !paths.contains(&checkout),
            "a linked worktree should not appear"
        );

        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["worktree", "remove", "--force", checkout.to_str().unwrap()])
            .status();
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn two_linked_worktrees_of_the_same_repo_are_one_folder() {
        let repo = committed_repo("shared");
        let first = add_worktree(&repo, "first");
        let second = add_worktree(&repo, "second");
        let mut state = state_working_in(first.clone());
        let extra = Workspace::test_new("other");
        let pane_id = extra.tabs[0].root_pane;
        let terminal_id = extra.tabs[0].panes[&pane_id].attached_terminal_id.clone();
        state.workspaces.push(extra);
        state.terminals.insert(
            terminal_id.clone(),
            TerminalState::new(terminal_id, second.clone()),
        );

        state.refresh_composer_folders(&TerminalRuntimeRegistry::default());

        let paths: Vec<_> = state
            .composer
            .folders
            .iter()
            .map(|folder| folder.path.clone())
            .collect();
        assert_eq!(
            paths.iter().filter(|path| *path == &repo).count(),
            1,
            "one parent, not one row per checkout"
        );
        assert!(!paths.contains(&first));
        assert!(!paths.contains(&second));

        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["worktree", "remove", "--force", first.to_str().unwrap()])
            .status();
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["worktree", "remove", "--force", second.to_str().unwrap()])
            .status();
        let _ = std::fs::remove_dir_all(repo);
    }
}
