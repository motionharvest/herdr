use std::path::PathBuf;

use bytes::Bytes;

use super::{terminal_targets::TerminalTargetError, App, Mode};
use crate::api::schema::{AgentStartParams, SplitDirection};

impl App {
    pub(super) fn collect_agent_infos(&self) -> Vec<crate::api::schema::AgentInfo> {
        self.state
            .workspaces
            .iter()
            .enumerate()
            .flat_map(|(ws_idx, ws)| {
                ws.tabs.iter().flat_map(move |tab| {
                    tab.layout
                        .pane_ids()
                        .into_iter()
                        .filter_map(move |pane_id| self.agent_info(ws_idx, pane_id))
                })
            })
            .collect()
    }

    pub(super) fn agent_info_for_target(
        &self,
        target: &str,
    ) -> Result<crate::api::schema::AgentInfo, TerminalTargetError> {
        let resolved = self.resolve_terminal_target(target)?;
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| TerminalTargetError::NotFound {
                target: target.to_string(),
            })
    }

    pub(super) fn focus_agent_target(
        &mut self,
        target: &str,
    ) -> Result<crate::api::schema::AgentInfo, TerminalTargetError> {
        let resolved = self.resolve_terminal_target(target)?;
        self.state
            .focus_pane_in_workspace(resolved.ws_idx, resolved.pane_id);
        self.state.mode = Mode::Terminal;
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| TerminalTargetError::NotFound {
                target: target.to_string(),
            })
    }

    pub(super) fn rename_agent_target(
        &mut self,
        target: &str,
        name: Option<String>,
    ) -> Result<crate::api::schema::AgentInfo, AgentRenameError> {
        let resolved = self
            .resolve_terminal_target(target)
            .map_err(AgentRenameError::Target)?;
        let normalized_name = name.and_then(|name| {
            let trimmed = name.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });

        if let Some(name) = normalized_name.as_deref() {
            let conflicts = self.agent_name_conflicts(name, &resolved.terminal_id);
            if !conflicts.is_empty() {
                return Err(AgentRenameError::DuplicateName {
                    name: name.to_string(),
                    candidates: conflicts,
                });
            }
        }

        let Some(terminal) = self
            .state
            .terminals
            .values_mut()
            .find(|terminal| terminal.id.to_string() == resolved.terminal_id)
        else {
            return Err(AgentRenameError::Target(TerminalTargetError::NotFound {
                target: target.to_string(),
            }));
        };
        match normalized_name {
            Some(name) => terminal.set_agent_name(name),
            None => terminal.clear_agent_name(),
        }
        self.state.mark_session_dirty();
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| {
                AgentRenameError::Target(TerminalTargetError::NotFound {
                    target: target.to_string(),
                })
            })
    }

    pub(super) fn start_agent(
        &mut self,
        params: AgentStartParams,
    ) -> Result<(crate::api::schema::AgentInfo, Vec<String>), AgentStartError> {
        let name = params.name.trim().to_string();
        if name.is_empty() {
            return Err(AgentStartError::InvalidName);
        }
        let conflicts = self.agent_name_conflicts(&name, "");
        if !conflicts.is_empty() {
            return Err(AgentStartError::DuplicateName {
                name,
                candidates: conflicts,
            });
        }
        let (ws_idx, pane_id, argv) = self.place_agent(params)?;
        self.name_started_agent(ws_idx, pane_id, name)?;
        let agent = self
            .agent_info(ws_idx, pane_id)
            .ok_or_else(|| AgentStartError::SpawnFailed("agent disappeared".into()))?;
        Ok((agent, argv))
    }

    /// Start an agent with no pane to show it.
    ///
    /// Everything an agent is made of is here — a process, a terminal reading
    /// it, a pane id and pane state wired to that terminal, and a row in the
    /// table. The one thing it has not got is a place in a layout, so it works
    /// without taking a room from anything already on screen. Docking it later
    /// gives it one, and that is the only difference between this agent and any
    /// other.
    ///
    /// `agent` is what the caller already knows it is starting. A pane shows
    /// its own process while detection catches up; a row has nothing else to
    /// show, so the row would be blank for as long as that took. Detection
    /// overwrites this the moment the agent draws.
    pub(super) fn start_hidden_agent(
        &mut self,
        cwd: PathBuf,
        argv: &[String],
        agent: Option<crate::detect::Agent>,
    ) -> Result<crate::layout::PaneId, AgentStartError> {
        if argv.is_empty() {
            return Err(AgentStartError::EmptyArgv);
        }
        let (rows, cols) = self.state.estimate_pane_size();
        let pane_id = crate::layout::PaneId::alloc();
        let runtime = crate::terminal::TerminalRuntime::spawn_argv_command(
            pane_id,
            rows,
            cols,
            cwd.clone(),
            argv,
            self.state.pane_scrollback_limit_bytes,
            self.state.host_terminal_theme,
            self.event_tx.clone(),
            self.render_notify.clone(),
            self.render_dirty.clone(),
        )
        .map_err(|err| AgentStartError::SpawnFailed(err.to_string()))?;

        let terminal_id = crate::terminal::TerminalId::alloc();
        let mut terminal = crate::terminal::TerminalState::new(terminal_id.clone(), cwd)
            .with_launch_argv(argv.to_vec());
        if let Some(agent) = agent {
            let _ = terminal.set_detected_state_with_screen_signals_at(
                Some(agent),
                crate::detect::AgentState::Idle,
                false,
                false,
                false,
                false,
                std::time::Instant::now(),
            );
        }
        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        self.state.terminals.insert(terminal_id.clone(), terminal);
        self.state
            .detached_agents
            .push(crate::app::state::DetachedAgent {
                pane_id,
                pane: crate::pane::PaneState::new(terminal_id),
            });
        self.state.mark_session_dirty();
        self.schedule_session_save();
        Ok(pane_id)
    }

    /// Start an ordinary interactive terminal with no place in a layout, then
    /// hand its shell the command from the composer. The shell stays alive
    /// after the command finishes, so docking the row gives the user the same
    /// terminal they would have opened by hand in `cwd`.
    pub(super) fn start_hidden_terminal(
        &mut self,
        cwd: PathBuf,
        command: &str,
    ) -> Result<crate::layout::PaneId, AgentStartError> {
        let (rows, cols) = self.state.estimate_pane_size();
        let pane_id = crate::layout::PaneId::alloc();
        let runtime = crate::terminal::TerminalRuntime::spawn(
            pane_id,
            rows,
            cols,
            cwd.clone(),
            self.state.pane_scrollback_limit_bytes,
            self.state.host_terminal_theme,
            crate::pane::PaneShellConfig::new(&self.state.default_shell, self.state.shell_mode),
            self.event_tx.clone(),
            self.render_notify.clone(),
            self.render_dirty.clone(),
        )
        .map_err(|err| AgentStartError::SpawnFailed(err.to_string()))?;

        let input = format!("{command}\r");
        if let Err(err) = runtime.try_send_bytes(Bytes::from(input)) {
            runtime.shutdown();
            return Err(AgentStartError::SpawnFailed(format!(
                "could not send command to terminal: {err}"
            )));
        }

        let terminal_id = crate::terminal::TerminalId::alloc();
        let terminal = crate::terminal::TerminalState::new(terminal_id.clone(), cwd);
        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        self.state.terminals.insert(terminal_id.clone(), terminal);
        self.state
            .detached_agents
            .push(crate::app::state::DetachedAgent {
                pane_id,
                pane: crate::pane::PaneState::new(terminal_id),
            });
        self.state.mark_session_dirty();
        self.schedule_session_save();
        Ok(pane_id)
    }

    fn name_started_agent(
        &mut self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
        name: String,
    ) -> Result<(), AgentStartError> {
        let terminal_id = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.terminal_id(pane_id))
            .cloned()
            .ok_or_else(|| AgentStartError::SpawnFailed("terminal disappeared".into()))?;
        let Some(terminal) = self.state.terminals.get_mut(&terminal_id) else {
            return Err(AgentStartError::SpawnFailed("terminal disappeared".into()));
        };
        terminal.set_agent_name(name);
        self.state.mark_session_dirty();
        Ok(())
    }

    /// Work out where an agent goes and spawn it there. Everything about
    /// placement lives here, so no two ways of asking for an agent can drift
    /// into landing in different places.
    fn place_agent(
        &mut self,
        params: AgentStartParams,
    ) -> Result<(usize, crate::layout::PaneId, Vec<String>), AgentStartError> {
        if params.argv.is_empty() {
            return Err(AgentStartError::EmptyArgv);
        }

        let cwd = params
            .cwd
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/"));
        let argv = params.argv;
        let focus = params.focus;
        let (rows, cols) = self.state.estimate_pane_size();

        let (ws_idx, tab_idx, pane_id) = if let Some(tab_id) = params.tab_id {
            let (ws_idx, tab_idx) =
                self.parse_tab_id(&tab_id)
                    .ok_or_else(|| AgentStartError::TargetNotFound {
                        target: tab_id.clone(),
                    })?;
            if let Some(workspace_id) = params.workspace_id.as_deref() {
                let requested_ws_idx = self.parse_workspace_id(workspace_id).ok_or_else(|| {
                    AgentStartError::TargetNotFound {
                        target: workspace_id.to_string(),
                    }
                })?;
                if requested_ws_idx != ws_idx {
                    return Err(AgentStartError::PlacementConflict);
                }
            }
            let target_pane = self.state.workspaces[ws_idx].tabs[tab_idx].layout.focused();
            self.spawn_agent_split(
                ws_idx,
                target_pane,
                params.split.unwrap_or(SplitDirection::Right),
                cwd,
                &argv,
                focus,
            )?
        } else if let Some(workspace_id) = params.workspace_id {
            let ws_idx = self.parse_workspace_id(&workspace_id).ok_or_else(|| {
                AgentStartError::TargetNotFound {
                    target: workspace_id.clone(),
                }
            })?;
            let tab_idx = self.state.workspaces[ws_idx].active_tab;
            let target_pane = self.state.workspaces[ws_idx].tabs[tab_idx].layout.focused();
            self.spawn_agent_split(
                ws_idx,
                target_pane,
                params.split.unwrap_or(SplitDirection::Right),
                cwd,
                &argv,
                focus,
            )?
        } else if self.state.workspaces.is_empty() {
            self.spawn_agent_workspace(cwd, rows, cols, &argv, focus)?
        } else {
            let ws_idx = self.state.active.unwrap_or(0);
            let tab_idx = self.state.workspaces[ws_idx].active_tab;
            let target_pane = self.state.workspaces[ws_idx].tabs[tab_idx].layout.focused();
            self.spawn_agent_split(
                ws_idx,
                target_pane,
                params.split.unwrap_or(SplitDirection::Right),
                cwd,
                &argv,
                focus,
            )?
        };

        debug_assert_eq!(
            self.agent_info(ws_idx, pane_id).map(|agent| agent.tab_id),
            self.public_tab_id(ws_idx, tab_idx)
        );
        Ok((ws_idx, pane_id, argv))
    }

    pub(super) fn agent_start_error_body(
        &self,
        err: AgentStartError,
    ) -> crate::api::schema::ErrorBody {
        match err {
            AgentStartError::InvalidName => crate::api::schema::ErrorBody {
                code: "invalid_agent_name".into(),
                message: "agent name must not be empty".into(),
            },
            AgentStartError::EmptyArgv => crate::api::schema::ErrorBody {
                code: "invalid_agent_argv".into(),
                message: "agent start argv must not be empty".into(),
            },
            AgentStartError::TargetNotFound { target } => crate::api::schema::ErrorBody {
                code: "agent_placement_not_found".into(),
                message: format!("agent placement target {target} not found"),
            },
            AgentStartError::PlacementConflict => crate::api::schema::ErrorBody {
                code: "agent_placement_conflict".into(),
                message: "--tab must belong to --workspace".into(),
            },
            AgentStartError::SpawnFailed(message) => crate::api::schema::ErrorBody {
                code: "agent_start_failed".into(),
                message,
            },
            AgentStartError::DuplicateName { name, candidates } => crate::api::schema::ErrorBody {
                code: "agent_name_taken".into(),
                message: format!(
                    "agent name {name} is already used; candidates: {}",
                    candidates
                        .into_iter()
                        .map(|candidate| format!(
                            "terminal_id={} pane_id={} workspace_id={} tab_id={} cwd={} status={:?}",
                            candidate.terminal_id,
                            candidate.pane_id,
                            candidate.workspace_id,
                            candidate.tab_id,
                            candidate.cwd.unwrap_or_else(|| "unknown".into()),
                            candidate.agent_status,
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            },
        }
    }

    pub(super) fn agent_target_error_body(
        &self,
        err: TerminalTargetError,
    ) -> crate::api::schema::ErrorBody {
        match err {
            TerminalTargetError::NotFound { target } => crate::api::schema::ErrorBody {
                code: "agent_not_found".into(),
                message: format!("agent target {target} not found"),
            },
            TerminalTargetError::Ambiguous { target, candidates } => {
                crate::api::schema::ErrorBody {
                    code: "agent_target_ambiguous".into(),
                    message: format!(
                        "agent target {target} is ambiguous; candidates: {}",
                        candidates
                            .into_iter()
                            .map(|candidate| format!(
                                "terminal_id={} pane_id={} workspace_id={} tab_id={} cwd={} status={:?}",
                                candidate.terminal_id,
                                candidate.pane_id,
                                candidate.workspace_id,
                                candidate.tab_id,
                                candidate.cwd.unwrap_or_else(|| "unknown".into()),
                                candidate.agent_status,
                            ))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                }
            }
        }
    }

    pub(super) fn agent_rename_error_body(
        &self,
        err: AgentRenameError,
    ) -> crate::api::schema::ErrorBody {
        match err {
            AgentRenameError::Target(err) => self.agent_target_error_body(err),
            AgentRenameError::DuplicateName { name, candidates } => crate::api::schema::ErrorBody {
                code: "agent_name_taken".into(),
                message: format!(
                    "agent name {name} is already used; candidates: {}",
                    candidates
                        .into_iter()
                        .map(|candidate| format!(
                            "terminal_id={} pane_id={} workspace_id={} tab_id={} cwd={} status={:?}",
                            candidate.terminal_id,
                            candidate.pane_id,
                            candidate.workspace_id,
                            candidate.tab_id,
                            candidate.cwd.unwrap_or_else(|| "unknown".into()),
                            candidate.agent_status,
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            },
        }
    }

    fn spawn_agent_workspace(
        &mut self,
        cwd: PathBuf,
        rows: u16,
        cols: u16,
        argv: &[String],
        focus: bool,
    ) -> Result<(usize, usize, crate::layout::PaneId), AgentStartError> {
        let (ws, terminal, runtime) = crate::workspace::Workspace::new_argv_command(
            cwd,
            rows,
            cols,
            argv,
            self.state.pane_scrollback_limit_bytes,
            self.state.host_terminal_theme,
            self.event_tx.clone(),
            self.render_notify.clone(),
            self.render_dirty.clone(),
        )
        .map_err(|err| AgentStartError::SpawnFailed(err.to_string()))?;
        self.terminal_runtimes.insert(terminal.id.clone(), runtime);
        self.state.terminals.insert(terminal.id.clone(), terminal);
        self.state.workspaces.push(ws);
        let ws_idx = self.state.workspaces.len() - 1;
        self.state
            .remove_alias_shadowed_by_new_pane(self.state.workspaces[ws_idx].tabs[0].root_pane);
        if focus || self.state.active.is_none() {
            self.state.switch_workspace(ws_idx);
            self.state.mode = Mode::Terminal;
        }
        self.schedule_session_save();
        let pane_id = self.state.workspaces[ws_idx].tabs[0].root_pane;
        Ok((ws_idx, 0, pane_id))
    }

    fn spawn_agent_split(
        &mut self,
        ws_idx: usize,
        target_pane: crate::layout::PaneId,
        split: SplitDirection,
        cwd: PathBuf,
        argv: &[String],
        focus: bool,
    ) -> Result<(usize, usize, crate::layout::PaneId), AgentStartError> {
        let (rows, cols) = self.state.estimate_pane_size();
        let previous_focus = self.state.current_pane_focus_target();
        let direction = match split {
            SplitDirection::Right => ratatui::layout::Direction::Horizontal,
            SplitDirection::Down => ratatui::layout::Direction::Vertical,
        };
        let result = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| {
                ws.split_pane_argv_command(
                    target_pane,
                    direction,
                    rows,
                    cols,
                    Some(cwd),
                    argv,
                    self.state.pane_scrollback_limit_bytes,
                    self.state.host_terminal_theme,
                    focus,
                )
            })
            .ok_or_else(|| AgentStartError::TargetNotFound {
                target: target_pane.raw().to_string(),
            })?
            .map_err(|err| AgentStartError::SpawnFailed(err.to_string()))?;
        self.terminal_runtimes
            .insert(result.1.terminal.id.clone(), result.1.runtime);
        self.state
            .remove_alias_shadowed_by_new_pane(result.1.pane_id);
        self.state
            .terminals
            .insert(result.1.terminal.id.clone(), result.1.terminal);
        if focus {
            self.state.switch_workspace_tab(ws_idx, result.0);
            self.state
                .record_pane_focus_change(previous_focus, ws_idx, result.1.pane_id);
            self.state.mode = Mode::Terminal;
        }
        self.schedule_session_save();
        Ok((ws_idx, result.0, result.1.pane_id))
    }

    fn agent_info(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<crate::api::schema::AgentInfo> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let pane_state = ws.pane_state(pane_id)?;
        let terminal = self.state.terminals.get(&pane_state.attached_terminal_id)?;
        if !terminal.is_agent_terminal() {
            return None;
        }
        let pane = self.pane_info(ws_idx, pane_id)?;
        Some(crate::api::schema::AgentInfo {
            terminal_id: pane.terminal_id,
            name: terminal.agent_name.clone(),
            agent: pane.agent,
            title: pane.title,
            display_agent: pane.display_agent,
            agent_status: pane.agent_status,
            custom_status: pane.custom_status,
            state_labels: pane.state_labels,
            agent_session: pane.agent_session,
            workspace_id: pane.workspace_id,
            tab_id: pane.tab_id,
            pane_id: pane.pane_id,
            focused: pane.focused,
            cwd: pane.cwd,
            foreground_cwd: pane.foreground_cwd,
            revision: pane.revision,
        })
    }

    fn agent_name_conflicts(
        &self,
        name: &str,
        except_terminal_id: &str,
    ) -> Vec<crate::api::schema::AgentInfo> {
        self.collect_agent_infos()
            .into_iter()
            .filter(|agent| {
                agent.name.as_deref() == Some(name) && agent.terminal_id != except_terminal_id
            })
            .collect()
    }
}

pub(super) enum AgentStartError {
    InvalidName,
    EmptyArgv,
    TargetNotFound {
        target: String,
    },
    PlacementConflict,
    SpawnFailed(String),
    DuplicateName {
        name: String,
        candidates: Vec<crate::api::schema::AgentInfo>,
    },
}

pub(super) enum AgentRenameError {
    Target(TerminalTargetError),
    DuplicateName {
        name: String,
        candidates: Vec<crate::api::schema::AgentInfo>,
    },
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::config::Config;
    use crate::workspace::Workspace;

    fn test_app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![Workspace::test_new("test")];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        app
    }

    #[tokio::test]
    async fn a_hidden_agent_runs_without_taking_a_pane() {
        let mut app = test_app();
        let panes_before = app.state.workspaces[0].tabs[0].layout.pane_ids();
        let focus_before = app.state.workspaces[0].focused_pane_id();

        let pane_id = app
            .start_hidden_agent(
                std::env::current_dir().unwrap(),
                &["sleep".to_string(), "30".to_string()],
                Some(crate::detect::Agent::Pi),
            )
            .unwrap_or_else(|_| panic!("hidden agent should start"));

        assert_eq!(app.state.workspaces.len(), 1);
        assert_eq!(
            app.state.workspaces[0].tabs[0].layout.pane_ids(),
            panes_before
        );
        assert_eq!(app.state.workspaces[0].focused_pane_id(), focus_before);
        assert_eq!(app.state.detached_agents.len(), 1);
        assert_eq!(app.state.detached_agents[0].pane_id, pane_id);

        let terminal_id = app.state.detached_agents[0]
            .pane
            .attached_terminal_id
            .clone();
        assert!(app.state.terminals.contains_key(&terminal_id));
        assert!(app.terminal_runtimes.get(&terminal_id).is_some());

        app.state.close_detached_agent(pane_id);
        app.shutdown_detached_terminal_runtimes();
    }

    #[tokio::test]
    async fn a_hidden_terminal_runs_the_task_in_its_directory_and_stays_open() {
        let mut app = test_app();
        app.state.default_shell = "/bin/sh".to_string();
        app.state.shell_mode = crate::config::ShellModeConfig::NonLogin;
        let cwd = std::env::temp_dir().join(format!(
            "herdr-composer-terminal-{}",
            crate::layout::PaneId::alloc().raw()
        ));
        std::fs::create_dir(&cwd).expect("scratch directory should be created");
        let proof = cwd.join("cwd.txt");

        let pane_id = app
            .start_hidden_terminal(cwd.clone(), "pwd > cwd.txt")
            .unwrap_or_else(|_| panic!("hidden terminal should start"));
        for _ in 0..100 {
            if proof.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert_eq!(
            std::fs::read_to_string(&proof)
                .expect("the command should write its working directory")
                .trim(),
            cwd.to_string_lossy()
        );
        let terminal_id = app.state.detached_agents[0]
            .pane
            .attached_terminal_id
            .clone();
        assert!(
            app.terminal_runtimes.get(&terminal_id).is_some(),
            "the interactive shell should remain available after the command"
        );

        app.state.close_detached_agent(pane_id);
        app.shutdown_detached_terminal_runtimes();
        std::fs::remove_dir_all(cwd).expect("scratch directory should be removed");
    }

    #[tokio::test]
    async fn docking_a_hidden_agent_gives_it_the_pane_it_never_had() {
        let mut app = test_app();
        let target = app.state.workspaces[0].tabs[0].root_pane;
        let pane_id = app
            .start_hidden_agent(
                std::env::current_dir().unwrap(),
                &["sleep".to_string(), "30".to_string()],
                Some(crate::detect::Agent::Pi),
            )
            .unwrap_or_else(|_| panic!("hidden agent should start"));

        assert!(app.state.dock_detached_agent(
            pane_id,
            target,
            crate::layout::DropZone::Edge(crate::layout::SplitSide::Right),
        ));

        assert!(app.state.detached_agents.is_empty());
        assert!(app.state.workspaces[0].pane_state(pane_id).is_some());
        assert_eq!(app.state.workspaces[0].focused_pane_id(), Some(pane_id));

        let terminal_id = app.state.terminal_id_for_pane(0, pane_id).unwrap();
        app.state.terminal_runtime_shutdowns.push(terminal_id);
        app.shutdown_detached_terminal_runtimes();
    }
}
