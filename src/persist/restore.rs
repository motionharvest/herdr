use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use ratatui::layout::Direction;
use tokio::sync::{mpsc, Notify};
use tracing::{error, warn};

use crate::detect::AgentState;
use crate::events::AppEvent;
use crate::layout::{Node, PaneId, TileLayout};
use crate::pane::PaneState;
use crate::terminal::{TerminalId, TerminalRuntime, TerminalState};
use crate::workspace::Workspace;

use super::snapshot::{
    PaneAgentSessionSnapshot, PaneHistorySnapshot, TabHistorySnapshot, WorkspaceHistorySnapshot,
};
use super::{
    DirectionSnapshot, LayoutSnapshot, SessionHistorySnapshot, SessionSnapshot, TabSnapshot,
    WorkspaceSnapshot,
};

struct AgentRestoreState<'a> {
    enabled: bool,
    resumed_sessions: &'a mut HashSet<String>,
}

struct PaneRestoreStartup<'a> {
    restore_plan: Option<crate::agent_resume::AgentResumePlan>,
    initial_history_ansi: Option<&'a str>,
    duplicate_agent_session: bool,
    reserved_agent_session: Option<String>,
}

struct RestoreRuntimeContext<'a> {
    scrollback_limit_bytes: usize,
    shell_config: crate::pane::PaneShellConfig<'a>,
    resume_agents_on_restore: bool,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<Notify>,
    render_dirty: Arc<AtomicBool>,
}

type RestoredSession = (
    Vec<Workspace>,
    HashMap<TerminalId, TerminalState>,
    HashMap<TerminalId, TerminalRuntime>,
    Vec<crate::app::state::DetachedAgent>,
);
type RestoredWorkspace = (
    Workspace,
    Vec<TerminalState>,
    HashMap<TerminalId, TerminalRuntime>,
);
type RestoredTab = (
    crate::workspace::Tab,
    Vec<TerminalState>,
    HashMap<TerminalId, TerminalRuntime>,
);
type RestoreFailures<T> = (T, usize);

/// One pane rebuilt from a snapshot. `runtime` is absent when the pane is
/// waiting on a native agent resume, which spawns its own process later.
struct RestoredPane {
    pane: PaneState,
    terminal: TerminalState,
    runtime: Option<TerminalRuntime>,
}

struct PaneRestoreFailure {
    error: std::io::Error,
    was_imported: bool,
}

/// Restore workspaces from a snapshot. Each pane gets a fresh shell in its saved cwd.
pub fn restore(
    snapshot: &SessionSnapshot,
    history: Option<&SessionHistorySnapshot>,
    rows: u16,
    cols: u16,
    scrollback_limit_bytes: usize,
    default_shell: &str,
    shell_mode: crate::config::ShellModeConfig,
    resume_agents_on_restore: bool,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<Notify>,
    render_dirty: Arc<AtomicBool>,
) -> RestoredSession {
    let mut imported_panes = HashMap::new();
    restore_with_imports(
        snapshot,
        history,
        rows,
        cols,
        scrollback_limit_bytes,
        crate::pane::PaneShellConfig::new(default_shell, shell_mode),
        resume_agents_on_restore,
        &mut imported_panes,
        events,
        render_notify,
        render_dirty,
    )
}

#[cfg(unix)]
pub fn restore_handoff(
    snapshot: &SessionSnapshot,
    scrollback_limit_bytes: usize,
    default_shell: &str,
    shell_mode: crate::config::ShellModeConfig,
    imports: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<Notify>,
    render_dirty: Arc<AtomicBool>,
) -> std::io::Result<RestoredSession> {
    restore_with_imports_strict(
        snapshot,
        None,
        24,
        80,
        scrollback_limit_bytes,
        crate::pane::PaneShellConfig::new(default_shell, shell_mode),
        true,
        imports,
        events,
        render_notify,
        render_dirty,
    )
}

#[cfg(unix)]
pub fn handoff_pane_aliases(
    snapshot: &SessionSnapshot,
    workspaces: &[Workspace],
) -> HashMap<u32, PaneId> {
    let mut aliases = HashMap::new();
    for (ws_snap, workspace) in snapshot.workspaces.iter().zip(workspaces) {
        for (tab_snap, tab) in ws_snap.tabs.iter().zip(&workspace.tabs) {
            let old_ids = collect_snapshot_pane_ids(&tab_snap.layout);
            let new_ids = tab.layout.pane_ids();
            for (old_id, new_id) in old_ids.into_iter().zip(new_ids) {
                if old_id != new_id.raw() {
                    aliases.insert(old_id, new_id);
                }
            }
        }
    }
    aliases
}

#[cfg(unix)]
fn collect_snapshot_pane_ids(node: &LayoutSnapshot) -> Vec<u32> {
    let mut ids = Vec::new();
    collect_snapshot_ids_inner(node, &mut ids);
    ids
}

#[cfg(unix)]
fn collect_snapshot_ids_inner(node: &LayoutSnapshot, ids: &mut Vec<u32>) {
    match node {
        LayoutSnapshot::Pane(id) => ids.push(*id),
        LayoutSnapshot::Split { first, second, .. } => {
            collect_snapshot_ids_inner(first, ids);
            collect_snapshot_ids_inner(second, ids);
        }
    }
}

#[cfg(unix)]
fn restore_with_imports_strict(
    snapshot: &SessionSnapshot,
    history: Option<&SessionHistorySnapshot>,
    rows: u16,
    cols: u16,
    scrollback_limit_bytes: usize,
    shell_config: crate::pane::PaneShellConfig<'_>,
    resume_agents_on_restore: bool,
    imported_panes: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<Notify>,
    render_dirty: Arc<AtomicBool>,
) -> std::io::Result<RestoredSession> {
    let (restored, failed_imports) = restore_with_imports_and_failures(
        snapshot,
        history,
        rows,
        cols,
        scrollback_limit_bytes,
        shell_config,
        resume_agents_on_restore,
        imported_panes,
        events,
        render_notify,
        render_dirty,
    );
    if failed_imports > 0 {
        return Err(std::io::Error::other(format!(
            "handoff failed to restore {failed_imports} imported pane runtime(s)"
        )));
    }
    if !imported_panes.is_empty() {
        return Err(std::io::Error::other(format!(
            "handoff import did not consume {} pane runtime(s)",
            imported_panes.len()
        )));
    }
    Ok(restored)
}

fn restore_with_imports(
    snapshot: &SessionSnapshot,
    history: Option<&SessionHistorySnapshot>,
    rows: u16,
    cols: u16,
    scrollback_limit_bytes: usize,
    shell_config: crate::pane::PaneShellConfig<'_>,
    resume_agents_on_restore: bool,
    imported_panes: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<Notify>,
    render_dirty: Arc<AtomicBool>,
) -> RestoredSession {
    restore_with_imports_and_failures(
        snapshot,
        history,
        rows,
        cols,
        scrollback_limit_bytes,
        shell_config,
        resume_agents_on_restore,
        imported_panes,
        events,
        render_notify,
        render_dirty,
    )
    .0
}

fn restore_with_imports_and_failures(
    snapshot: &SessionSnapshot,
    history: Option<&SessionHistorySnapshot>,
    rows: u16,
    cols: u16,
    scrollback_limit_bytes: usize,
    shell_config: crate::pane::PaneShellConfig<'_>,
    resume_agents_on_restore: bool,
    imported_panes: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<Notify>,
    render_dirty: Arc<AtomicBool>,
) -> RestoreFailures<RestoredSession> {
    let mut workspaces = Vec::new();
    let mut terminals = HashMap::new();
    let mut terminal_runtimes = HashMap::new();
    let mut resumed_agent_sessions = HashSet::new();
    let mut used_terminal_ids = HashSet::new();
    let mut failed_imports = 0;
    for (idx, ws_snap) in snapshot.workspaces.iter().enumerate() {
        let runtime_context = RestoreRuntimeContext {
            scrollback_limit_bytes,
            shell_config,
            resume_agents_on_restore,
            events: events.clone(),
            render_notify: render_notify.clone(),
            render_dirty: render_dirty.clone(),
        };
        let (restored, workspace_failed_imports) = restore_workspace(
            ws_snap,
            history.and_then(|history| history.workspaces.get(idx)),
            rows,
            cols,
            &runtime_context,
            &mut resumed_agent_sessions,
            &mut used_terminal_ids,
            imported_panes,
        );
        failed_imports += workspace_failed_imports;
        if let Some((workspace, restored_terminals, restored_runtimes)) = restored {
            for terminal in restored_terminals {
                terminals.insert(terminal.id.clone(), terminal);
            }
            terminal_runtimes.extend(restored_runtimes);
            workspaces.push(workspace);
        }
    }

    // The agents with no pane are restored after the spaces, so a terminal id
    // one of them saved cannot be taken from a pane that also wants it.
    let mut detached_agents = Vec::new();
    for saved in &snapshot.detached_agents {
        let runtime_context = RestoreRuntimeContext {
            scrollback_limit_bytes,
            shell_config,
            resume_agents_on_restore,
            events: events.clone(),
            render_notify: render_notify.clone(),
            render_dirty: render_dirty.clone(),
        };
        let pane_id = PaneId::alloc();
        match restore_pane(
            pane_id,
            Some(saved.pane_id),
            Some(&saved.pane),
            None,
            true,
            rows,
            cols,
            &runtime_context,
            &mut resumed_agent_sessions,
            &mut used_terminal_ids,
            imported_panes,
        ) {
            Ok(restored) => {
                if let Some(runtime) = restored.runtime {
                    terminal_runtimes.insert(restored.terminal.id.clone(), runtime);
                }
                let mut pane = restored.pane;
                // The pane snapshot already carries the marker. A snapshot
                // written before it did carries the marker beside the agent
                // instead, where an acknowledged pane is the default, so the
                // two are folded together rather than one overwriting the other.
                pane.seen &= saved.seen;
                pane.completed |= saved.completed || !saved.seen;
                terminals.insert(restored.terminal.id.clone(), restored.terminal);
                detached_agents.push(crate::app::state::DetachedAgent { pane_id, pane });
            }
            Err(failure) => {
                if failure.was_imported {
                    failed_imports += 1;
                    error!(
                        pane_id = saved.pane_id,
                        err = %failure.error,
                        "failed to restore imported set-down agent"
                    );
                }
                error!(
                    pane_id = saved.pane_id,
                    err = %failure.error,
                    "failed to restore set-down agent, skipping"
                );
            }
        }
    }

    (
        (workspaces, terminals, terminal_runtimes, detached_agents),
        failed_imports,
    )
}

fn restore_workspace(
    snap: &WorkspaceSnapshot,
    history: Option<&WorkspaceHistorySnapshot>,
    rows: u16,
    cols: u16,
    runtime_context: &RestoreRuntimeContext<'_>,
    resumed_agent_sessions: &mut HashSet<String>,
    used_terminal_ids: &mut HashSet<TerminalId>,
    imported_panes: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
) -> RestoreFailures<Option<RestoredWorkspace>> {
    let mut tabs = Vec::new();
    let mut terminals = Vec::new();
    let mut terminal_runtimes = HashMap::new();
    let mut public_pane_numbers = HashMap::new();
    let mut next_public_pane_number = 1;
    let mut failed_imports = 0;

    for (idx, tab_snap) in snap.tabs.iter().enumerate() {
        let (restored_tab, tab_failed_imports) = restore_tab(
            tab_snap,
            history.and_then(|history| history.tabs.get(idx)),
            idx + 1,
            rows,
            cols,
            runtime_context,
            resumed_agent_sessions,
            used_terminal_ids,
            imported_panes,
        );
        failed_imports += tab_failed_imports;
        let Some((tab, restored_terminals, restored_runtimes)) = restored_tab else {
            continue;
        };
        for pane_id in tab.layout.pane_ids() {
            public_pane_numbers.insert(pane_id, next_public_pane_number);
            next_public_pane_number += 1;
        }
        terminals.extend(restored_terminals);
        terminal_runtimes.extend(restored_runtimes);
        tabs.push(tab);
    }

    if tabs.is_empty() {
        return (None, failed_imports);
    }

    let worktree_space = restored_worktree_space_membership(snap.worktree_space.clone());
    let natural_pane_order: Vec<PaneId> =
        tabs.iter().flat_map(|tab| tab.layout.pane_ids()).collect();
    let agent_order = restored_agent_order(&snap.agent_order, &natural_pane_order);

    (
        Some(Workspace {
            id: snap
                .id
                .clone()
                .unwrap_or_else(crate::workspace::generate_workspace_id),
            custom_name: snap.custom_name.clone(),
            identity_cwd: snap.identity_cwd.clone(),
            cached_git_branch: crate::workspace::git_branch(&snap.identity_cwd),
            cached_git_ahead_behind: None,
            cached_git_space: crate::workspace::git_space_metadata(&snap.identity_cwd),
            cached_git_worktree_state: crate::workspace::GitWorktreeState::Clean,
            cached_git_landed: false,
            pane_git_statuses: HashMap::new(),
            worktree_space,
            public_pane_numbers,
            next_public_pane_number,
            agent_order,
            active_tab: snap.active_tab.min(tabs.len().saturating_sub(1)),
            tabs,
            #[cfg(test)]
            test_runtimes: HashMap::new(),
        })
        .map(|workspace| (workspace, terminals, terminal_runtimes)),
        failed_imports,
    )
}

/// Rebuild a space's custom agent order from saved natural-order positions.
/// Panes that failed to restore shift every later position, so a saved order
/// that isn't a permutation of the restored panes is dropped rather than
/// applied to the wrong panes.
fn restored_agent_order(saved: &[usize], natural_pane_order: &[PaneId]) -> Vec<PaneId> {
    if saved.len() != natural_pane_order.len() {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    if !saved
        .iter()
        .all(|position| *position < natural_pane_order.len() && seen.insert(*position))
    {
        return Vec::new();
    }
    saved
        .iter()
        .map(|position| natural_pane_order[*position])
        .collect()
}

fn restored_worktree_space_membership(
    space: Option<crate::workspace::WorktreeSpaceMembership>,
) -> Option<crate::workspace::WorktreeSpaceMembership> {
    space.filter(|space| {
        space.checkout_path.exists()
            && crate::workspace::git_space_metadata(&space.checkout_path)
                .is_some_and(|current| current.key == space.key)
    })
}

fn restore_tab(
    snap: &TabSnapshot,
    history: Option<&TabHistorySnapshot>,
    number: usize,
    rows: u16,
    cols: u16,
    runtime_context: &RestoreRuntimeContext<'_>,
    resumed_agent_sessions: &mut HashSet<String>,
    used_terminal_ids: &mut HashSet<TerminalId>,
    imported_panes: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
) -> RestoreFailures<Option<RestoredTab>> {
    let (node, id_map) = restore_node_remapped(&snap.layout);
    let reverse_id_map: HashMap<PaneId, u32> = id_map
        .iter()
        .map(|(&old_id, &new_id)| (new_id, old_id))
        .collect();
    let pane_ids = collect_pane_ids(&node);

    let mut panes = HashMap::new();
    let mut terminals = Vec::new();
    let mut terminal_runtimes = HashMap::new();
    let mut failed_imports = 0;
    for id in &pane_ids {
        let old_id = reverse_id_map.get(id).copied();
        let saved_pane = old_id.and_then(|old_id| snap.panes.get(&old_id));
        let saved_history =
            old_id.and_then(|old_id| history.and_then(|history| history.panes.get(&old_id)));
        // A pane opened for an argv-launched agent is still that agent after it
        // is docked. Restart its saved command just as we do while it is set
        // down; otherwise a normal server stop restores the pane as a shell
        // and silently drops the agent from the table.
        let restart_saved_command = saved_pane
            .and_then(|pane| pane.launch_argv.as_ref())
            .is_some_and(|argv| !argv.is_empty());
        match restore_pane(
            *id,
            old_id,
            saved_pane,
            saved_history,
            restart_saved_command,
            rows,
            cols,
            runtime_context,
            resumed_agent_sessions,
            used_terminal_ids,
            imported_panes,
        ) {
            Ok(restored) => {
                if let Some(runtime) = restored.runtime {
                    terminal_runtimes.insert(restored.terminal.id.clone(), runtime);
                }
                panes.insert(*id, restored.pane);
                terminals.push(restored.terminal);
            }
            Err(failure) => {
                if failure.was_imported {
                    failed_imports += 1;
                    error!(
                        tab = ?snap.custom_name,
                        pane_id = id.raw(),
                        err = %failure.error,
                        "failed to restore imported pane"
                    );
                }
                error!(
                    tab = ?snap.custom_name,
                    pane_id = id.raw(),
                    err = %failure.error,
                    "failed to restore pane, skipping"
                );
            }
        }
    }

    if panes.is_empty() {
        warn!(
            tab = ?snap.custom_name,
            "no panes could be restored for tab, dropping it"
        );
        return (None, failed_imports);
    }

    let surviving: HashSet<PaneId> = panes.keys().copied().collect();
    let Some(node) = prune_restored_node(node, &surviving) else {
        warn!(
            tab = ?snap.custom_name,
            "restored tab lost all panes after pruning missing layout nodes"
        );
        return (None, failed_imports);
    };
    let pane_ids = collect_pane_ids(&node);
    let Some(focus) = resolve_restored_pane(snap.focused, &id_map, &surviving, &pane_ids) else {
        return (None, failed_imports);
    };
    let Some(root_pane) = resolve_restored_pane(snap.root_pane, &id_map, &surviving, &pane_ids)
    else {
        return (None, failed_imports);
    };
    let layout = TileLayout::from_saved(node, focus);

    (
        Some((
            crate::workspace::Tab {
                custom_name: snap.custom_name.clone(),
                number,
                root_pane,
                layout,
                panes,
                #[cfg(test)]
                runtimes: HashMap::new(),
                zoomed: snap.zoomed,
                events: runtime_context.events.clone(),
                render_notify: runtime_context.render_notify.clone(),
                render_dirty: runtime_context.render_dirty.clone(),
            },
            terminals,
            terminal_runtimes,
        )),
        failed_imports,
    )
}

/// Rebuild one pane from what was saved of it: its terminal identity, its
/// labels, its agent session, and either the live runtime handed over by the
/// old process or a fresh one spawned in its place.
///
/// Nothing here knows whether the pane is going into a layout. A pane in a tab
/// and a set-down agent with no tab are the same thing to restore, so they are
/// restored the same way and cannot drift apart.
#[allow(clippy::too_many_arguments)]
fn restore_pane(
    pane_id: PaneId,
    old_pane_id: Option<u32>,
    saved_pane: Option<&super::snapshot::PaneSnapshot>,
    saved_history: Option<&PaneHistorySnapshot>,
    restart_saved_command: bool,
    rows: u16,
    cols: u16,
    runtime_context: &RestoreRuntimeContext<'_>,
    resumed_agent_sessions: &mut HashSet<String>,
    used_terminal_ids: &mut HashSet<TerminalId>,
    imported_panes: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
) -> Result<RestoredPane, PaneRestoreFailure> {
    let saved_cwd = saved_pane
        .map(|p| p.cwd.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));

    let cwd = if saved_cwd.exists() {
        saved_cwd
    } else {
        warn!(
            cwd = %saved_cwd.display(),
            "saved pane cwd does not exist, falling back to HOME"
        );
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"));
        if home.exists() {
            home
        } else {
            PathBuf::from("/")
        }
    };

    // Reuse the saved terminal identity so everything derived from it
    // (assigned pane names, agent bookkeeping) survives the restart.
    // Fall back to a fresh id for pre-field snapshots or duplicates.
    let terminal_id = saved_pane
        .and_then(|p| p.terminal_id.clone())
        .filter(|saved_id| used_terminal_ids.insert(saved_id.clone()))
        .unwrap_or_else(TerminalId::alloc);
    let saved_label = saved_pane.and_then(|p| p.label.clone());
    let saved_agent_name = saved_pane.and_then(|p| p.agent_name.clone());
    let saved_title_name = saved_pane.and_then(|p| p.title_name.clone());
    let saved_summary = saved_pane.and_then(|p| p.summary.clone());
    let saved_launch_argv = saved_pane.and_then(|p| p.launch_argv.clone());
    let saved_agent_timing = saved_pane.and_then(|p| p.agent_timing.as_ref());
    let saved_agent_session = saved_pane.and_then(|p| p.agent_session.as_ref());
    let startup = {
        let mut agent_restore = AgentRestoreState {
            enabled: runtime_context.resume_agents_on_restore,
            resumed_sessions: resumed_agent_sessions,
        };
        pane_restore_startup(saved_agent_session, saved_history, &mut agent_restore)
    };
    let initial_restore_agent = startup
        .restore_plan
        .as_ref()
        .and_then(|plan| crate::detect::parse_agent_label(&plan.agent))
        .or_else(|| {
            if !restart_saved_command {
                return None;
            }
            let program = saved_launch_argv.as_ref()?.first()?;
            std::path::Path::new(program)
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(crate::detect::parse_agent_label)
        });

    let imported_runtime = old_pane_id.and_then(|old_id| imported_panes.remove(&old_id));
    let was_imported = imported_runtime.is_some();
    let pending_native_agent_restore = if was_imported {
        None
    } else {
        startup.restore_plan.clone()
    };
    if let Some(plan) = pending_native_agent_restore {
        let mut terminal = TerminalState::new(terminal_id.clone(), cwd.clone())
            .with_pending_agent_resume_plan(plan)
            .with_restore_startup();
        if let Some(label) = saved_label {
            terminal.set_manual_label(label);
        }
        if let Some(agent_name) = saved_agent_name {
            terminal.set_agent_name(agent_name);
        }
        if let Some(title_name) = saved_title_name {
            terminal.title_name = Some(title_name);
        }
        if let Some(summary) = saved_summary {
            terminal.set_manual_summary(summary);
        }
        if let Some(timing) = saved_agent_timing {
            timing.restore_into(&mut terminal);
        }
        if let Some(agent) = initial_restore_agent {
            let _ = terminal.set_detected_state_with_screen_signals_at(
                Some(agent),
                AgentState::Idle,
                false,
                false,
                false,
                false,
                std::time::Instant::now(),
            );
        }
        if let Some(session) =
            restored_terminal_agent_session(saved_agent_session, startup.duplicate_agent_session)
        {
            terminal.set_persisted_agent_session(session);
        }
        return Ok(RestoredPane {
            pane: restored_pane_state(terminal_id, saved_pane),
            terminal,
            runtime: None,
        });
    }

    let restarted_saved_command = restart_saved_command
        && !was_imported
        && runtime_context.resume_agents_on_restore
        && saved_agent_session.is_none()
        && startup.restore_plan.is_none()
        && saved_launch_argv
            .as_ref()
            .is_some_and(|argv| !argv.is_empty());
    let restart_argv = restart_argv(saved_launch_argv.as_deref(), restarted_saved_command);
    let runtime_result = if let Some(imported) = imported_runtime {
        TerminalRuntime::from_handoff_fd(
            crate::handoff_runtime::ImportedHandoffRuntime {
                master_fd: imported.master_fd,
                state: imported.state.with_pane_id(pane_id),
            },
            runtime_context.scrollback_limit_bytes,
            crate::terminal_theme::TerminalTheme::default(),
            runtime_context.events.clone(),
            runtime_context.render_notify.clone(),
            runtime_context.render_dirty.clone(),
        )
    } else if restarted_saved_command {
        TerminalRuntime::spawn_argv_command(
            pane_id,
            rows,
            cols,
            cwd.clone(),
            restart_argv.as_deref().unwrap_or_default(),
            runtime_context.scrollback_limit_bytes,
            crate::terminal_theme::TerminalTheme::default(),
            runtime_context.events.clone(),
            runtime_context.render_notify.clone(),
            runtime_context.render_dirty.clone(),
        )
    } else {
        TerminalRuntime::spawn_with_initial_history(
            pane_id,
            rows,
            cols,
            cwd.clone(),
            runtime_context.scrollback_limit_bytes,
            crate::terminal_theme::TerminalTheme::default(),
            runtime_context.shell_config,
            startup.initial_history_ansi,
            runtime_context.events.clone(),
            runtime_context.render_notify.clone(),
            runtime_context.render_dirty.clone(),
        )
    };

    match runtime_result {
        Ok(runtime) => {
            let mut terminal = TerminalState::new(terminal_id.clone(), cwd.clone());
            // An imported pane holds the process it always held, so nothing
            // about it is starting up. A restarted one is booting its harness
            // again, and that boot is not work the marker should answer to.
            if !was_imported {
                terminal = terminal.with_restore_startup();
            }
            // An imported pane still holds the process its saved command
            // started, so that command stays with it. A restarted one is now
            // running the command with the task off, and writing that down is
            // what stops the task coming back at the next restore.
            if was_imported {
                if let Some(argv) = saved_launch_argv {
                    terminal = terminal.with_launch_argv(argv).with_respawn_shell_on_exit();
                }
            } else if let Some(argv) = restart_argv {
                terminal = terminal.with_launch_argv(argv);
            }
            if let Some(label) = saved_label {
                terminal.set_manual_label(label);
            }
            if let Some(agent_name) = saved_agent_name {
                terminal.set_agent_name(agent_name);
            }
            if let Some(title_name) = saved_title_name {
                terminal.title_name = Some(title_name);
            }
            if let Some(summary) = saved_summary {
                terminal.set_manual_summary(summary);
            }
            if let Some(timing) = saved_agent_timing {
                timing.restore_into(&mut terminal);
            }
            if let Some(agent) = initial_restore_agent {
                let _ = terminal.set_detected_state_with_screen_signals_at(
                    Some(agent),
                    AgentState::Idle,
                    false,
                    false,
                    false,
                    false,
                    std::time::Instant::now(),
                );
            }
            if let Some(session) = restored_terminal_agent_session(
                saved_agent_session,
                startup.duplicate_agent_session,
            ) {
                terminal.set_persisted_agent_session(session);
            }
            Ok(RestoredPane {
                pane: restored_pane_state(terminal_id, saved_pane),
                terminal,
                runtime: Some(runtime),
            })
        }
        Err(error) => {
            if let Some(key) = startup.reserved_agent_session.as_deref() {
                resumed_agent_sessions.remove(key);
            }
            Err(PaneRestoreFailure {
                error,
                was_imported,
            })
        }
    }
}

/// A restored pane wearing the marker it was saved with.
///
/// The marker is two flags, and only one combination has to be reconstructed:
/// a snapshot written before `completed` existed says an agent finished by
/// leaving it unacknowledged, so an unacknowledged agent is read as finished.
fn restored_pane_state(
    terminal_id: TerminalId,
    saved_pane: Option<&super::snapshot::PaneSnapshot>,
) -> PaneState {
    let mut pane = PaneState::new(terminal_id);
    if let Some(saved) = saved_pane {
        pane.seen = saved.seen;
        pane.completed = saved.completed || !saved.seen;
    }
    pane
}

/// The command a restored pane starts again, when starting it again is what
/// keeps its agent in the table.
///
/// The saved command carries the task the agent was started on, and a restore
/// is not that task being asked for a second time. Restarting the agent is what
/// keeps the row; sending it the task is what would make a stop and start cost
/// a turn from every agent at once, so the task comes off first.
fn restart_argv(saved_launch_argv: Option<&[String]>, restarting: bool) -> Option<Vec<String>> {
    saved_launch_argv
        .filter(|_| restarting)
        .map(crate::harness::without_task)
}

fn pane_restore_startup<'a>(
    session: Option<&PaneAgentSessionSnapshot>,
    history: Option<&'a PaneHistorySnapshot>,
    agent_restore: &mut AgentRestoreState<'_>,
) -> PaneRestoreStartup<'a> {
    // Native agent resume owns the conversation history. If a pane has a
    // resumable agent session and resume is enabled, do not replay saved pane
    // presentation history into that terminal, even when this pane is a
    // duplicate suppressed by session de-duplication.
    let restore_plan =
        session.and_then(|session| restore_plan_for_snapshot(session, agent_restore.enabled));
    let has_native_agent_restore = restore_plan.is_some();
    // Reserve before spawning so later panes in the same restore pass cannot
    // launch the same native agent session. The caller rolls this reservation
    // back if runtime spawn fails before any agent process is started.
    let mut reserved_agent_session = None;
    let duplicate_agent_session = restore_plan.as_ref().is_some_and(|plan| {
        if agent_restore
            .resumed_sessions
            .insert(plan.dedupe_key.clone())
        {
            reserved_agent_session = Some(plan.dedupe_key.clone());
            false
        } else {
            true
        }
    });
    let restore_plan = if duplicate_agent_session {
        None
    } else {
        restore_plan
    };

    PaneRestoreStartup {
        restore_plan,
        initial_history_ansi: if has_native_agent_restore {
            None
        } else {
            history.map(|history| history.ansi.as_str())
        },
        duplicate_agent_session,
        reserved_agent_session,
    }
}

fn restore_plan_for_snapshot(
    session: &PaneAgentSessionSnapshot,
    resume_agents_on_restore: bool,
) -> Option<crate::agent_resume::AgentResumePlan> {
    if !resume_agents_on_restore {
        return None;
    }
    let persisted = persisted_agent_session_from_snapshot(session)?;
    crate::agent_resume::plan(&session.source, &session.agent, &persisted.session_ref)
}

fn persisted_agent_session_from_snapshot(
    session: &PaneAgentSessionSnapshot,
) -> Option<crate::agent_resume::PersistedAgentSession> {
    crate::agent_resume::session_ref_from_snapshot(
        &session.source,
        &session.agent,
        session.kind,
        &session.value,
    )
}

fn restored_terminal_agent_session(
    session: Option<&PaneAgentSessionSnapshot>,
    duplicate_agent_session: bool,
) -> Option<crate::agent_resume::PersistedAgentSession> {
    if duplicate_agent_session {
        return None;
    }
    session.and_then(persisted_agent_session_from_snapshot)
}

#[cfg(test)]
fn take_restore_plan_for_snapshot(
    session: &PaneAgentSessionSnapshot,
    resume_agents_on_restore: bool,
    resumed_agent_sessions: &mut HashSet<String>,
) -> Option<crate::agent_resume::AgentResumePlan> {
    restore_plan_for_snapshot(session, resume_agents_on_restore)
        .filter(|plan| resumed_agent_sessions.insert(plan.dedupe_key.clone()))
}

pub(super) fn prune_restored_node(node: Node, surviving: &HashSet<PaneId>) -> Option<Node> {
    match node {
        Node::Pane(id) => surviving.contains(&id).then_some(Node::Pane(id)),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let first = prune_restored_node(*first, surviving);
            let second = prune_restored_node(*second, surviving);
            match (first, second) {
                (Some(first), Some(second)) => Some(Node::Split {
                    direction,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(remaining), None) | (None, Some(remaining)) => Some(remaining),
                (None, None) => None,
            }
        }
    }
}

pub(super) fn resolve_restored_pane(
    saved_old_id: Option<u32>,
    id_map: &HashMap<u32, PaneId>,
    surviving: &HashSet<PaneId>,
    pane_ids: &[PaneId],
) -> Option<PaneId> {
    saved_old_id
        .and_then(|old_id| id_map.get(&old_id).copied())
        .filter(|pane_id| surviving.contains(pane_id))
        .or_else(|| pane_ids.first().copied())
}

/// Restore a layout tree, remapping every pane ID to a fresh globally unique one.
/// Returns the new tree and a map of old_raw_id → new PaneId.
pub(super) fn restore_node_remapped(snap: &LayoutSnapshot) -> (Node, HashMap<u32, PaneId>) {
    let mut id_map = HashMap::new();
    let node = remap_inner(snap, &mut id_map);
    (node, id_map)
}

fn remap_inner(snap: &LayoutSnapshot, id_map: &mut HashMap<u32, PaneId>) -> Node {
    match snap {
        LayoutSnapshot::Pane(old_id) => {
            let new_id = PaneId::alloc();
            id_map.insert(*old_id, new_id);
            Node::Pane(new_id)
        }
        LayoutSnapshot::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let first_node = remap_inner(first, id_map);
            let second_node = remap_inner(second, id_map);
            let dir = match direction {
                DirectionSnapshot::Horizontal => Direction::Horizontal,
                DirectionSnapshot::Vertical => Direction::Vertical,
            };
            Node::Split {
                direction: dir,
                ratio: *ratio,
                first: Box::new(first_node),
                second: Box::new(second_node),
            }
        }
    }
}

pub(super) fn collect_pane_ids(node: &Node) -> Vec<PaneId> {
    let mut ids = Vec::new();
    collect_ids_inner(node, &mut ids);
    ids
}

fn collect_ids_inner(node: &Node, ids: &mut Vec<PaneId>) {
    match node {
        Node::Pane(id) => ids.push(*id),
        Node::Split { first, second, .. } => {
            collect_ids_inner(first, ids);
            collect_ids_inner(second, ids);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_and_restore_node_round_trip() {
        let node = Node::Split {
            direction: Direction::Horizontal,
            ratio: 0.5,
            first: Box::new(Node::Pane(PaneId::from_raw(0))),
            second: Box::new(Node::Split {
                direction: Direction::Vertical,
                ratio: 0.3,
                first: Box::new(Node::Pane(PaneId::from_raw(1))),
                second: Box::new(Node::Pane(PaneId::from_raw(2))),
            }),
        };

        let snap = super::super::snapshot::capture_node(&node);
        let (restored, id_map) = restore_node_remapped(&snap);

        assert_eq!(id_map.len(), 3);
        let ids = collect_pane_ids(&restored);
        assert_eq!(ids.len(), 3);
        let unique: std::collections::HashSet<u32> = ids.iter().map(|id| id.raw()).collect();
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn restored_agent_order_maps_saved_positions_onto_new_pane_ids() {
        let panes = [
            PaneId::from_raw(31),
            PaneId::from_raw(32),
            PaneId::from_raw(33),
        ];

        assert_eq!(
            restored_agent_order(&[2, 0, 1], &panes),
            vec![panes[2], panes[0], panes[1]]
        );
        // No saved order at all.
        assert!(restored_agent_order(&[], &panes).is_empty());
    }

    #[test]
    fn restored_agent_order_discards_orders_that_do_not_match_the_restored_panes() {
        let panes = [PaneId::from_raw(41), PaneId::from_raw(42)];

        // A pane failed to restore, so every saved position is suspect.
        assert!(restored_agent_order(&[2, 0, 1], &panes).is_empty());
        // Out of range, and duplicated.
        assert!(restored_agent_order(&[0, 5], &panes).is_empty());
        assert!(restored_agent_order(&[1, 1], &panes).is_empty());
    }

    #[test]
    fn prune_restored_node_collapses_missing_branch() {
        let keep = PaneId::from_raw(11);
        let missing = PaneId::from_raw(12);
        let node = Node::Split {
            direction: Direction::Horizontal,
            ratio: 0.5,
            first: Box::new(Node::Pane(keep)),
            second: Box::new(Node::Pane(missing)),
        };
        let surviving = std::collections::HashSet::from([keep]);

        let pruned = prune_restored_node(node, &surviving).expect("remaining pane should survive");

        assert!(matches!(pruned, Node::Pane(id) if id == keep));
    }

    #[test]
    fn resolve_restored_pane_prefers_surviving_saved_id_and_falls_back_to_first_remaining() {
        let first = PaneId::from_raw(21);
        let second = PaneId::from_raw(22);
        let id_map = HashMap::from([(0_u32, first), (1_u32, second)]);
        let surviving = std::collections::HashSet::from([first]);
        let pane_ids = vec![first];

        assert_eq!(
            resolve_restored_pane(Some(0), &id_map, &surviving, &pane_ids),
            Some(first)
        );
        assert_eq!(
            resolve_restored_pane(Some(1), &id_map, &surviving, &pane_ids),
            Some(first)
        );
    }

    #[test]
    fn restored_worktree_space_membership_drops_missing_checkout() {
        let missing =
            std::env::temp_dir().join(format!("herdr-missing-worktree-{}", std::process::id()));
        let membership = crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: missing.join("repo"),
            checkout_path: missing.join("checkout"),
            is_linked_worktree: true,
        };

        assert_eq!(restored_worktree_space_membership(Some(membership)), None);
    }

    #[test]
    fn a_restarted_agent_is_started_again_without_the_task_it_was_started_on() {
        let saved = vec!["claude".to_string(), "fix the drag preview".to_string()];

        assert_eq!(
            restart_argv(Some(&saved), true),
            Some(vec!["claude".to_string()]),
            "a stop and start must not spend a turn from every agent in the space"
        );
        assert_eq!(restart_argv(Some(&saved), false), None);
        assert_eq!(restart_argv(None, true), None);
    }

    #[test]
    fn a_restarted_command_that_is_not_a_task_launch_is_restarted_as_it_stands() {
        let saved = vec!["claude".to_string(), "--continue".to_string()];

        assert_eq!(restart_argv(Some(&saved), true), Some(saved.clone()));
    }

    #[test]
    fn restore_plan_respects_opt_in_and_allowlist() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: "/tmp/pi-session.jsonl".into(),
        };

        assert!(restore_plan_for_snapshot(&session, false).is_none());
        assert_eq!(
            restore_plan_for_snapshot(&session, true).unwrap().argv,
            vec!["pi", "--session", "/tmp/pi-session.jsonl"]
        );

        let unsupported_path = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: "/tmp/claude-session".into(),
        };
        assert!(restore_plan_for_snapshot(&unsupported_path, true).is_none());
    }

    #[test]
    fn restore_plan_selection_suppresses_duplicates() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: "/tmp/pi-session.jsonl".into(),
        };
        let mut resumed = HashSet::new();

        assert!(take_restore_plan_for_snapshot(&session, false, &mut resumed).is_none());
        assert!(resumed.is_empty());

        let first = take_restore_plan_for_snapshot(&session, true, &mut resumed)
            .expect("first restore should get a plan");
        assert_eq!(first.argv, vec!["pi", "--session", "/tmp/pi-session.jsonl"]);
        assert!(take_restore_plan_for_snapshot(&session, true, &mut resumed).is_none());
    }

    #[test]
    fn pane_restore_startup_suppresses_history_for_native_agent_resume() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: "/tmp/pi-session.jsonl".into(),
        };
        let history = super::super::snapshot::PaneHistorySnapshot {
            ansi: "RESTORED_HISTORY\r\n".into(),
            lines: 1,
        };
        let mut resumed = HashSet::new();
        let mut agent_restore = AgentRestoreState {
            enabled: true,
            resumed_sessions: &mut resumed,
        };

        let startup = pane_restore_startup(Some(&session), Some(&history), &mut agent_restore);

        assert!(startup.restore_plan.is_some());
        assert!(startup.initial_history_ansi.is_none());
        assert!(!startup.duplicate_agent_session);
    }

    #[test]
    fn pane_restore_startup_suppresses_history_for_duplicate_native_agent_session() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: "/tmp/pi-session.jsonl".into(),
        };
        let history = super::super::snapshot::PaneHistorySnapshot {
            ansi: "RESTORED_HISTORY\r\n".into(),
            lines: 1,
        };
        let mut resumed = HashSet::new();
        let mut agent_restore = AgentRestoreState {
            enabled: true,
            resumed_sessions: &mut resumed,
        };

        let first = pane_restore_startup(Some(&session), Some(&history), &mut agent_restore);
        let duplicate = pane_restore_startup(Some(&session), Some(&history), &mut agent_restore);

        assert!(first.restore_plan.is_some());
        assert!(first.initial_history_ansi.is_none());
        assert!(duplicate.restore_plan.is_none());
        assert!(duplicate.initial_history_ansi.is_none());
        assert!(duplicate.duplicate_agent_session);
    }

    #[test]
    fn pane_restore_startup_keeps_history_without_native_agent_resume() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: "/tmp/pi-session.jsonl".into(),
        };
        let history = super::super::snapshot::PaneHistorySnapshot {
            ansi: "RESTORED_HISTORY\r\n".into(),
            lines: 1,
        };
        let mut resumed = HashSet::new();
        let mut agent_restore = AgentRestoreState {
            enabled: false,
            resumed_sessions: &mut resumed,
        };

        let startup = pane_restore_startup(Some(&session), Some(&history), &mut agent_restore);

        assert!(startup.restore_plan.is_none());
        assert_eq!(startup.initial_history_ansi, Some("RESTORED_HISTORY\r\n"));
        assert!(!startup.duplicate_agent_session);
        assert!(resumed.is_empty());
    }

    #[tokio::test]
    async fn restore_brings_back_an_agent_that_had_no_pane() {
        let cwd = std::env::current_dir().unwrap();
        let terminal_id = TerminalId::alloc();
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: Vec::new(),
            active: None,
            selected: 0,
            composer_agent: None,
            agent_order: Vec::new(),
            detached_agents: vec![super::super::snapshot::DetachedAgentSnapshot {
                pane_id: 7,
                pane: super::super::snapshot::PaneSnapshot {
                    terminal_id: Some(terminal_id.clone()),
                    cwd: cwd.clone(),
                    label: None,
                    agent_name: Some("scout".into()),
                    agent_session: None,
                    launch_argv: None,
                    seen: true,
                    completed: false,
                    agent_timing: None,
                    title_name: None,
                    summary: None,
                },
                seen: false,
                completed: true,
            }],
            ..Default::default()
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (workspaces, terminals, runtimes, detached) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            "/usr/bin/true",
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            Arc::new(Notify::new()),
            Arc::new(AtomicBool::new(false)),
        );

        assert!(workspaces.is_empty());
        assert_eq!(detached.len(), 1);
        assert_eq!(detached[0].pane.attached_terminal_id, terminal_id);
        assert!(!detached[0].pane.seen, "the done marker should survive");
        assert_eq!(
            terminals
                .get(&terminal_id)
                .and_then(|terminal| terminal.agent_name.clone()),
            Some("scout".into())
        );
        assert!(runtimes.contains_key(&terminal_id));
        for runtime in runtimes.into_values() {
            runtime.shutdown();
        }
    }

    #[tokio::test]
    async fn restore_restarts_a_set_down_agent_without_a_native_session() {
        let cwd = std::env::current_dir().unwrap();
        let terminal_id = TerminalId::alloc();
        let launch_argv = vec!["/bin/sleep".into(), "30".into()];
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: Vec::new(),
            active: None,
            selected: 0,
            composer_agent: None,
            agent_order: Vec::new(),
            detached_agents: vec![super::super::snapshot::DetachedAgentSnapshot {
                pane_id: 7,
                pane: super::super::snapshot::PaneSnapshot {
                    terminal_id: Some(terminal_id.clone()),
                    cwd,
                    label: None,
                    agent_name: None,
                    agent_session: None,
                    launch_argv: Some(launch_argv.clone()),
                    seen: true,
                    completed: false,
                    agent_timing: None,
                    title_name: None,
                    summary: None,
                },
                seen: true,
                completed: false,
            }],
            ..Default::default()
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (_workspaces, terminals, runtimes, detached) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            "/usr/bin/true",
            crate::config::ShellModeConfig::NonLogin,
            true,
            events,
            Arc::new(Notify::new()),
            Arc::new(AtomicBool::new(false)),
        );

        assert_eq!(detached.len(), 1);
        assert_eq!(
            terminals
                .get(&terminal_id)
                .and_then(|terminal| terminal.launch_argv.clone()),
            Some(launch_argv)
        );
        assert!(terminals[&terminal_id].is_agent_terminal());
        assert!(runtimes.contains_key(&terminal_id));
        for runtime in runtimes.into_values() {
            runtime.shutdown();
        }
    }

    #[tokio::test]
    async fn restore_restarts_a_docked_agent_without_a_native_session() {
        let cwd = std::env::current_dir().unwrap();
        let terminal_id = TerminalId::alloc();
        let launch_argv = vec!["/bin/sleep".into(), "30".into()];
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("workspace".into()),
                custom_name: None,
                identity_cwd: cwd.clone(),
                worktree_space: None,
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Pane(7),
                    panes: HashMap::from([(
                        7,
                        super::super::snapshot::PaneSnapshot {
                            terminal_id: Some(terminal_id.clone()),
                            cwd,
                            label: None,
                            agent_name: None,
                            agent_session: None,
                            launch_argv: Some(launch_argv.clone()),
                            seen: true,
                            completed: false,
                            agent_timing: None,
                            title_name: None,
                            summary: None,
                        },
                    )]),
                    zoomed: false,
                    focused: Some(7),
                    root_pane: Some(7),
                }],
                active_tab: 0,
                agent_order: Vec::new(),
            }],
            active: Some(0),
            selected: 0,
            composer_agent: None,
            agent_order: vec![terminal_id.clone()],
            detached_agents: Vec::new(),
            ..Default::default()
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (workspaces, terminals, runtimes, detached) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            "/usr/bin/true",
            crate::config::ShellModeConfig::NonLogin,
            true,
            events,
            Arc::new(Notify::new()),
            Arc::new(AtomicBool::new(false)),
        );

        assert_eq!(workspaces.len(), 1);
        assert!(detached.is_empty());
        assert_eq!(
            terminals
                .get(&terminal_id)
                .and_then(|terminal| terminal.launch_argv.clone()),
            Some(launch_argv)
        );
        assert!(terminals[&terminal_id].is_agent_terminal());
        assert!(runtimes.contains_key(&terminal_id));
        // The agent is being started again, so its harness is about to boot.
        // The marker beside its row must not read that boot as a run.
        assert!(terminals[&terminal_id].is_starting_up());
        for runtime in runtimes.into_values() {
            runtime.shutdown();
        }
    }

    #[tokio::test]
    async fn restore_brings_back_the_marker_of_an_agent_in_a_pane() {
        let cwd = std::env::current_dir().unwrap();
        let acknowledged = TerminalId::alloc();
        let finished = TerminalId::alloc();
        let pane = |terminal_id: &TerminalId, seen: bool, completed: bool| {
            super::super::snapshot::PaneSnapshot {
                terminal_id: Some(terminal_id.clone()),
                cwd: cwd.clone(),
                label: None,
                agent_name: Some("scout".into()),
                agent_session: None,
                launch_argv: None,
                seen,
                completed,
                agent_timing: None,
                title_name: None,
                summary: None,
            }
        };
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("workspace".into()),
                custom_name: None,
                identity_cwd: cwd.clone(),
                worktree_space: None,
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Split {
                        direction: DirectionSnapshot::Horizontal,
                        ratio: 0.5,
                        first: Box::new(LayoutSnapshot::Pane(1)),
                        second: Box::new(LayoutSnapshot::Pane(2)),
                    },
                    panes: HashMap::from([
                        (1, pane(&acknowledged, true, true)),
                        (2, pane(&finished, false, true)),
                    ]),
                    zoomed: false,
                    focused: Some(1),
                    root_pane: Some(1),
                }],
                active_tab: 0,
                agent_order: Vec::new(),
            }],
            active: Some(0),
            selected: 0,
            composer_agent: None,
            agent_order: Vec::new(),
            detached_agents: Vec::new(),
            ..Default::default()
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (workspaces, _terminals, runtimes, _detached) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            "/usr/bin/true",
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            Arc::new(Notify::new()),
            Arc::new(AtomicBool::new(false)),
        );

        let panes = &workspaces[0].tabs[0].panes;
        let marker = |terminal_id: &TerminalId| {
            panes
                .values()
                .find(|pane| pane.attached_terminal_id == *terminal_id)
                .map(|pane| (pane.seen, pane.completed))
                .expect("restored pane")
        };
        assert_eq!(marker(&acknowledged), (true, true), "the check should stay");
        assert_eq!(marker(&finished), (false, true), "the done dot should stay");
        for runtime in runtimes.into_values() {
            runtime.shutdown();
        }
    }

    #[test]
    fn restore_rehydrates_agent_session_metadata() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:hermes".into(),
            agent: "hermes".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Id,
            value: "hermes-session".into(),
        };

        let preserved = restored_terminal_agent_session(Some(&session), false)
            .expect("restore should preserve metadata");
        assert_eq!(preserved.source, "herdr:hermes");
        assert_eq!(preserved.agent, "hermes");
        assert_eq!(preserved.session_ref.value, "hermes-session");
    }

    #[test]
    fn restore_does_not_rehydrate_duplicate_agent_session_metadata() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: "/tmp/pi-session.jsonl".into(),
        };
        let mut resumed = HashSet::new();
        assert!(take_restore_plan_for_snapshot(&session, true, &mut resumed).is_some());
        assert!(take_restore_plan_for_snapshot(&session, true, &mut resumed).is_none());

        assert!(restored_terminal_agent_session(Some(&session), true).is_none());
    }

    #[tokio::test]
    async fn restore_carries_persisted_agent_session_metadata() {
        let cwd = std::env::current_dir().unwrap();
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("workspace".into()),
                custom_name: None,
                identity_cwd: cwd.clone(),
                worktree_space: None,
                agent_order: Vec::new(),
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Pane(0),
                    panes: HashMap::from([(
                        0,
                        super::super::snapshot::PaneSnapshot {
                            terminal_id: None,
                            cwd,
                            label: None,
                            agent_name: None,
                            agent_session: Some(super::super::snapshot::PaneAgentSessionSnapshot {
                                source: "herdr:opencode".into(),
                                agent: "opencode".into(),
                                kind: crate::agent_resume::AgentSessionRefKind::Id,
                                value: "opencode-session".into(),
                            }),
                            launch_argv: None,
                            seen: true,
                            completed: false,
                            agent_timing: None,
                            title_name: None,
                            summary: None,
                        },
                    )]),
                    zoomed: false,
                    focused: Some(0),
                    root_pane: Some(0),
                }],
                active_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            composer_agent: None,
            agent_order: Vec::new(),
            detached_agents: Vec::new(),
            ..Default::default()
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (_workspaces, terminals, _runtimes, _detached) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            "/usr/bin/true",
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            Arc::new(Notify::new()),
            Arc::new(AtomicBool::new(false)),
        );

        let terminal = terminals
            .values()
            .next()
            .expect("restored terminal should exist");
        assert!(
            !terminal.respawn_shell_on_exit,
            "agent sessions should not use native restore lifecycle when resume_agents_on_restore is disabled"
        );
        let session = terminal
            .persisted_agent_session
            .as_ref()
            .expect("persisted agent session should survive restore");
        assert_eq!(session.source, "herdr:opencode");
        assert_eq!(session.agent, "opencode");
        assert_eq!(session.session_ref.value, "opencode-session");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn native_agent_restore_defers_runtime_launch() {
        let cwd = std::env::current_dir().unwrap();
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("workspace".into()),
                custom_name: None,
                identity_cwd: cwd.clone(),
                worktree_space: None,
                agent_order: Vec::new(),
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Pane(0),
                    panes: HashMap::from([(
                        0,
                        super::super::snapshot::PaneSnapshot {
                            terminal_id: None,
                            cwd,
                            label: None,
                            agent_name: None,
                            agent_session: Some(super::super::snapshot::PaneAgentSessionSnapshot {
                                source: "herdr:codex".into(),
                                agent: "codex".into(),
                                kind: crate::agent_resume::AgentSessionRefKind::Id,
                                value: "codex-session".into(),
                            }),
                            launch_argv: None,
                            seen: true,
                            completed: false,
                            agent_timing: None,
                            title_name: None,
                            summary: None,
                        },
                    )]),
                    zoomed: false,
                    focused: Some(0),
                    root_pane: Some(0),
                }],
                active_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            composer_agent: None,
            agent_order: Vec::new(),
            detached_agents: Vec::new(),
            ..Default::default()
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (_workspaces, terminals, runtimes, _detached) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            "/bin/sh",
            crate::config::ShellModeConfig::NonLogin,
            true,
            events,
            Arc::new(Notify::new()),
            Arc::new(AtomicBool::new(false)),
        );

        let terminal = terminals
            .values()
            .next()
            .expect("native agent restore should create terminal state");
        assert!(
            terminal.pending_agent_resume_plan.is_some(),
            "restored native agent panes should defer resume until client terminal context is known"
        );
        assert!(
            !terminal.respawn_shell_on_exit,
            "deferred agent resume should not use native restore lifecycle before launch"
        );
        assert!(
            runtimes.is_empty(),
            "native agent restore should not spawn a fallback-size runtime during snapshot restore"
        );
        let mut imports = HashMap::new();
        let (_handoff_workspaces, handoff_terminals, handoff_runtimes, _handoff_detached) =
            restore_handoff(
                &snapshot,
                0,
                "/bin/sh",
                crate::config::ShellModeConfig::NonLogin,
                &mut imports,
                mpsc::channel(4).0,
                Arc::new(Notify::new()),
                Arc::new(AtomicBool::new(false)),
            )
            .expect("handoff restore should preserve pending native agent resume");
        let handoff_terminal = handoff_terminals
            .values()
            .next()
            .expect("handoff restore should create terminal state");
        assert!(
            handoff_terminal.pending_agent_resume_plan.is_some(),
            "handoff restore should preserve pending native agent resume intent"
        );
        assert!(
            handoff_runtimes.is_empty(),
            "handoff restore should not replace pending native agent resume with a shell runtime"
        );
    }

    #[tokio::test]
    async fn restore_seeds_saved_pane_history_into_runtime() {
        let (snapshot, history) = snapshot_with_saved_pane_history();
        let (events, _events_rx) = mpsc::channel(8);
        let render_notify = Arc::new(Notify::new());
        let render_dirty = Arc::new(AtomicBool::new(false));

        let (_workspaces, _terminals, runtimes, _detached) = restore(
            &snapshot,
            Some(&history),
            5,
            40,
            4096,
            "/bin/sh",
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            render_notify,
            render_dirty,
        );
        let runtime = runtimes
            .values()
            .next()
            .expect("restored runtime should exist");

        assert!(
            runtime
                .recent_unwrapped_text(10)
                .contains("RESTORED_HISTORY"),
            "saved history should be visible in the restored terminal backend"
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while runtime.cwd().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = runtime.try_send_bytes(bytes::Bytes::from_static(b"exit\n"));
    }

    #[tokio::test]
    async fn restore_without_history_snapshot_keeps_pane_contents_empty() {
        let (snapshot, _history) = snapshot_with_saved_pane_history();
        let (events, _events_rx) = mpsc::channel(8);
        let render_notify = Arc::new(Notify::new());
        let render_dirty = Arc::new(AtomicBool::new(false));

        let (_workspaces, _terminals, runtimes, _detached) = restore(
            &snapshot,
            None,
            5,
            40,
            4096,
            "/bin/sh",
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            render_notify,
            render_dirty,
        );
        let runtime = runtimes
            .values()
            .next()
            .expect("restored runtime should exist");

        assert!(
            !runtime
                .recent_unwrapped_text(10)
                .contains("RESTORED_HISTORY"),
            "pane history should not restore unless a history snapshot is supplied"
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while runtime.cwd().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = runtime.try_send_bytes(bytes::Bytes::from_static(b"exit\n"));
    }

    fn snapshot_with_saved_pane_history() -> (SessionSnapshot, SessionHistorySnapshot) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let mut panes = HashMap::new();
        panes.insert(
            0,
            super::super::snapshot::PaneSnapshot {
                terminal_id: None,
                cwd: cwd.clone(),
                label: None,
                agent_name: None,
                agent_session: None,
                launch_argv: None,
                seen: true,
                completed: false,
                agent_timing: None,
                title_name: None,
                summary: None,
            },
        );
        let history = SessionHistorySnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceHistorySnapshot {
                tabs: vec![super::super::snapshot::TabHistorySnapshot {
                    panes: HashMap::from([(
                        0,
                        super::super::snapshot::PaneHistorySnapshot {
                            ansi: "RESTORED_HISTORY\r\n".to_string(),
                            lines: 1,
                        },
                    )]),
                }],
            }],
        };
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("workspace".into()),
                custom_name: None,
                identity_cwd: cwd,
                worktree_space: None,
                agent_order: Vec::new(),
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Pane(0),
                    panes,
                    zoomed: false,
                    focused: Some(0),
                    root_pane: Some(0),
                }],
                active_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            composer_agent: None,
            agent_order: Vec::new(),
            detached_agents: Vec::new(),
            ..Default::default()
        };
        (snapshot, history)
    }

    fn snapshot_with_pane_terminal_ids(
        cwd: PathBuf,
        first_id: Option<TerminalId>,
        second_id: Option<TerminalId>,
    ) -> SessionSnapshot {
        let pane = |terminal_id: Option<TerminalId>| super::super::snapshot::PaneSnapshot {
            terminal_id,
            cwd: cwd.clone(),
            label: None,
            agent_name: None,
            agent_session: None,
            launch_argv: None,
            seen: true,
            completed: false,
            agent_timing: None,
            title_name: None,
            summary: None,
        };
        SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("workspace".into()),
                custom_name: None,
                identity_cwd: cwd.clone(),
                worktree_space: None,
                agent_order: Vec::new(),
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Split {
                        direction: DirectionSnapshot::Horizontal,
                        ratio: 0.5,
                        first: Box::new(LayoutSnapshot::Pane(0)),
                        second: Box::new(LayoutSnapshot::Pane(1)),
                    },
                    panes: HashMap::from([(0, pane(first_id)), (1, pane(second_id))]),
                    zoomed: false,
                    focused: Some(0),
                    root_pane: Some(0),
                }],
                active_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            composer_agent: None,
            agent_order: Vec::new(),
            detached_agents: Vec::new(),
            ..Default::default()
        }
    }

    fn restore_terminals(snapshot: &SessionSnapshot) -> HashMap<TerminalId, TerminalState> {
        let (events, _event_rx) = mpsc::channel(4);
        let (_workspaces, terminals, _runtimes, _detached) = restore(
            snapshot,
            None,
            24,
            80,
            0,
            "/usr/bin/true",
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            Arc::new(Notify::new()),
            Arc::new(AtomicBool::new(false)),
        );
        terminals
    }

    #[tokio::test]
    async fn restore_preserves_saved_terminal_ids_and_pane_names() {
        let cwd = std::env::current_dir().unwrap();
        let first = TerminalId::alloc();
        let second = TerminalId::alloc();
        let snapshot =
            snapshot_with_pane_terminal_ids(cwd, Some(first.clone()), Some(second.clone()));

        let terminals = restore_terminals(&snapshot);

        assert!(terminals.contains_key(&first));
        assert!(terminals.contains_key(&second));
        // Names are derived from terminal ids, so preserved ids mean the
        // assigned names come out identical after the restart.
        let names = crate::pane_names::assigned_names(&terminals);
        let first_name = names.get(&first).expect("restored terminal keeps a name");
        assert!(first_name.starts_with(crate::pane_names::base_name_for(&first.to_string())));
    }

    #[tokio::test]
    async fn restore_preserves_manual_summary() {
        let cwd = std::env::current_dir().unwrap();
        let first = TerminalId::alloc();
        let mut snapshot = snapshot_with_pane_terminal_ids(cwd, Some(first.clone()), None);
        snapshot.workspaces[0].tabs[0]
            .panes
            .get_mut(&0)
            .unwrap()
            .summary = Some("Improve Agent Summary to be useful".into());

        let terminals = restore_terminals(&snapshot);
        assert_eq!(
            terminals
                .get(&first)
                .and_then(|t| t.manual_summary.as_deref()),
            Some("Improve Agent Summary to be useful")
        );
        assert_eq!(
            terminals
                .get(&first)
                .and_then(|t| t.effective_title())
                .as_deref(),
            Some("Improve Agent Summary to be useful")
        );
    }

    #[tokio::test]
    async fn restore_preserves_title_name() {
        let cwd = std::env::current_dir().unwrap();
        let first = TerminalId::alloc();
        let mut snapshot = snapshot_with_pane_terminal_ids(cwd, Some(first.clone()), None);
        snapshot.workspaces[0].tabs[0]
            .panes
            .get_mut(&0)
            .unwrap()
            .title_name = Some("herdr-pane-title".into());

        let terminals = restore_terminals(&snapshot);
        assert_eq!(
            terminals.get(&first).and_then(|t| t.title_name.as_deref()),
            Some("herdr-pane-title")
        );
        let names = crate::pane_names::assigned_names(&terminals);
        assert_eq!(
            names.get(&first).map(String::as_str),
            Some(crate::pane_names::base_name_for(&first.to_string()))
        );
    }

    #[tokio::test]
    async fn restore_deduplicates_corrupt_terminal_ids() {
        let cwd = std::env::current_dir().unwrap();
        let duplicated = TerminalId::alloc();
        let snapshot = snapshot_with_pane_terminal_ids(
            cwd,
            Some(duplicated.clone()),
            Some(duplicated.clone()),
        );

        let terminals = restore_terminals(&snapshot);

        assert_eq!(terminals.len(), 2, "each pane must keep its own terminal");
        assert!(terminals.contains_key(&duplicated));
    }
}
