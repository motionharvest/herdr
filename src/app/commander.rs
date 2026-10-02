//! The Commander's frame clock, delivering the message its star carries, and
//! reading a line with Jev.
//!
//! With a TypeSafe key set, `Enter` sends the line to Jev on a thread of its
//! own and the box says it is asking. The answer comes back as an event. A
//! confident reading of something that cannot lose work is carried out at
//! once; anything else is said under the field and held, and the next `Enter`
//! on the same line carries it out. An edit lets the held reading go. When Jev
//! cannot be reached, the fixed reading is offered in its place, held the same
//! way.
//!
//! A line can hold several commands: `switch to the fifth space and add a
//! Claude Code agent, then prompt it to audit this week's diffs, and tell the
//! Grok agent to check the PR`. Jev first cuts it where one command ends and
//! the next begins, leaving an `and` inside a message alone. A headless Claude
//! Code running Opus then works out which commands wait for which; the first
//! command starts while it thinks, since it can wait for nothing. Every command
//! whose waits are over is read by Jev and carried out at once, so commands
//! that wait for nothing run together. Each is read against what is on screen
//! by then and what the commands it waits for did, so `it` finds the agent just
//! started. What waits for a command waits for it to settle: its message lands,
//! and an agent it started reaches its prompt (or 30 seconds pass). An unsure
//! command waits for `Enter` without holding up the ones that do not wait for
//! it; `Esc` or an edit abandons what has not run. Without the planner, the
//! commands run in the order written, one at a time.
//!
//! What the CLI can do goes through the same socket API request the CLI would
//! send, handled in process, so the Commander and `herdr` on the command line
//! cannot drift apart.
//!
//! While a star is waiting to leave, in the air, or still fading, the app
//! ticks it about sixty times a second. The first tick after a message is sent
//! aims the star, because only then has a frame been laid out with the target
//! pane on screen. The tick on which the star lands pastes the message into
//! the pane and presses Enter.

use std::time::{Duration, Instant};

use super::App;
use crate::api::schema::{
    Method, PaneRenameParams, PaneSendKeysParams, PaneSendTextParams, PaneTarget, Request,
    TabCreateParams, TabRenameParams, TabTarget, WorkspaceCreateParams, WorkspaceRenameParams,
    WorkspaceTarget, WorktreeCreateParams, WorktreeLandParams, WorktreeRemoveParams,
};
use crate::app::state::{Mode, ToastKind, ToastNotification};
use crate::commander::intent::{self, Command, Kind};
use std::sync::Arc;

use crate::commander::jev::{Jev, Oracle};
use crate::commander::plan::{self, Reading};
use crate::commander::{
    order, trail::Trail, trail::V2, Aim, Cargo, Delivery, Paragraph, Phase, Settle,
};
use crate::events::AppEvent;

/// A slow frame steps the star at most this far, so a stall does not make it
/// jump across the screen.
const MAX_STEP: Duration = Duration::from_millis(50);

/// A reading at least this sure is carried out without a second `Enter`,
/// unless it could lose work.
const ACT_AT_ONCE: f64 = 0.6;

/// The pause after each command of a paragraph before the next is read.
const SETTLE: Duration = Duration::from_millis(300);
/// How long the next command waits for an agent the one before started.
const AGENT_READY_TIMEOUT: Duration = Duration::from_secs(30);
/// How often a waiting paragraph checks whether it can go on.
const SETTLE_POLL: Duration = Duration::from_millis(150);

/// Whether the next command of a paragraph can go.
enum Settled {
    No,
    Yes,
    /// A started agent is stuck on a question; the paragraph stops.
    Stuck,
}

impl App {
    /// A key pressed while the Commander is open.
    pub(crate) fn commander_key(&mut self, key: crate::input::TerminalKey) {
        if super::input::handle_commander_key(&mut self.state, &self.terminal_runtimes, key)
            == super::input::CommanderKeyOutcome::Submit
        {
            self.submit_commander();
        }
    }

    /// A paste while the Commander is open. A pasted line is a finished line,
    /// so it is carried out at once.
    pub(crate) fn commander_paste(&mut self, text: &str) {
        self.state.commander.field.insert_str(text);
        super::input::refresh_commander_reading(&mut self.state, &self.terminal_runtimes);
        self.submit_commander();
    }

    pub(crate) fn submit_commander(&mut self) {
        let line = self.state.commander.field.text();
        let same_line = self
            .state
            .commander
            .paragraph
            .as_ref()
            .is_some_and(|p| p.line == line);
        if same_line {
            self.answer_paragraph();
        } else if self.state.commander.asking.is_some() {
            // Jev is still cutting the line; a second Enter waits with it.
        } else if self.state.jev_api_key.is_some() {
            if !line.trim().is_empty() {
                self.begin_paragraph(line);
            }
        } else if let Some(command) =
            super::input::submit_commander(&mut self.state, &mut self.terminal_runtimes)
        {
            self.state.commander.finish();
            self.dispatch(command, true);
        }
        self.sync_commander_clock(Instant::now());
    }

    /// `Enter` on a line already under way: carry out the first command held
    /// for it, or ask again about the ones Jev could not read. The commands
    /// already carried out are never run twice.
    fn answer_paragraph(&mut self) {
        let Some(paragraph) = self.state.commander.paragraph.as_mut() else {
            return;
        };
        if let Some(i) = paragraph.first_held() {
            let phase = std::mem::replace(&mut paragraph.steps[i].phase, Phase::Waiting);
            if let Phase::Held { command, .. } = phase {
                self.carry_out_step(i, command);
            }
        } else {
            for step in &mut paragraph.steps {
                if matches!(step.phase, Phase::Failed(_)) {
                    step.phase = Phase::Waiting;
                }
            }
        }
        self.pump_paragraph(Instant::now());
    }

    /// What answers questions about a line: Jev with the configured key,
    /// or what a test put in its place.
    fn jev(&self) -> Option<Arc<dyn Oracle + Send + Sync>> {
        if let Some(oracle) = &self.commander_oracle {
            return Some(oracle.clone());
        }
        Some(Arc::new(Jev {
            api_key: self.state.jev_api_key.clone()?,
            model: self.state.jev_model.clone(),
        }))
    }

    fn next_generation(&mut self) -> u64 {
        self.state.commander.generation += 1;
        self.state.commander.generation
    }

    /// Start on what was typed. A line with no place where a second command
    /// could begin is one command; anything else is first cut into commands
    /// by Jev.
    fn begin_paragraph(&mut self, line: String) {
        self.state.commander.paragraph = None;
        if plan::boundaries(&line).is_empty() {
            let clause = line.trim().to_string();
            return self.start_paragraph(line, vec![clause]);
        }
        let Some(jev) = self.jev() else {
            return;
        };
        let generation = self.next_generation();
        self.state.commander.asking = Some(generation);
        self.state.commander.reading = None;
        let event_tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let result = plan::split(jev.as_ref(), &line);
            let _ = event_tx.blocking_send(AppEvent::CommanderSplit {
                generation,
                line,
                result,
            });
        });
    }

    /// Whether an answer is still wanted: the box is open and still holds the
    /// line the answer is about.
    fn line_still_there(&self, line: &str) -> bool {
        self.state.mode == Mode::Commander && self.state.commander.field.text() == line
    }

    /// Jev's cut of a line into commands. A cut that failed leaves the line
    /// as one command, which is how it was typed.
    pub(crate) fn commander_split(
        &mut self,
        generation: u64,
        line: &str,
        result: Result<Vec<String>, String>,
    ) {
        if self.state.commander.asking != Some(generation) {
            return;
        }
        self.state.commander.asking = None;
        if !self.line_still_there(line) {
            return;
        }
        let clauses = match result {
            Ok(clauses) if !clauses.is_empty() => clauses,
            Ok(_) => return,
            Err(trouble) => {
                tracing::info!(error = %trouble, "commander: Jev could not split the line");
                vec![line.trim().to_string()]
            }
        };
        self.start_paragraph(line.to_string(), clauses);
    }

    /// Set the commands going. With several, the planner is asked which wait
    /// for which; the first starts meanwhile, since it can wait for nothing.
    fn start_paragraph(&mut self, line: String, clauses: Vec<String>) {
        let count = clauses.len();
        let mut paragraph = Paragraph::new(line.clone(), clauses.clone());
        let model = self.state.commander_planner_model.trim().to_string();
        if count == 1 {
            paragraph.after = Some(vec![Vec::new()]);
        } else if model.is_empty() {
            paragraph.after = Some(order::in_turn(count));
        } else {
            let generation = self.next_generation();
            paragraph.planning = Some(generation);
            let screen = order::screen(&super::input::commander_catalog(
                &self.state,
                &self.terminal_runtimes,
            ));
            let event_tx = self.event_tx.clone();
            std::thread::spawn(move || {
                let result = order::plan("claude", &model, &clauses, &screen);
                if let Err(trouble) = &result {
                    tracing::info!(error = %trouble, "commander: planner failed; running in order");
                }
                let _ = event_tx.blocking_send(AppEvent::CommanderOrdered {
                    generation,
                    line,
                    result,
                });
            });
        }
        self.state.commander.paragraph = Some(paragraph);
        self.pump_paragraph(Instant::now());
    }

    /// The planner's answer. When it failed, the commands run in the order
    /// written, one at a time.
    pub(crate) fn commander_ordered(
        &mut self,
        generation: u64,
        line: &str,
        result: Result<order::After, String>,
    ) {
        let Some(paragraph) = self.state.commander.paragraph.as_mut() else {
            return;
        };
        if paragraph.planning != Some(generation) || paragraph.line != line {
            return;
        }
        paragraph.planning = None;
        let after = result.unwrap_or_else(|_| order::in_turn(paragraph.steps.len()));
        tracing::info!(?after, "commander: planned which commands wait for which");
        paragraph.after = Some(after);
        self.pump_paragraph(Instant::now());
        self.sync_commander_clock(Instant::now());
    }

    /// Move the line on: steps that have settled are done, steps whose waits
    /// are over are sent to Jev, and a line with every command carried out
    /// closes the box. Returns whether anything changed.
    pub(crate) fn pump_paragraph(&mut self, now: Instant) -> bool {
        let Some(paragraph) = self.state.commander.paragraph.as_ref() else {
            return false;
        };
        let mut changed = false;
        let settling: Vec<(usize, Settle)> = paragraph
            .steps
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s.phase {
                Phase::Settling(settle) => Some((i, settle)),
                _ => None,
            })
            .collect();
        for (i, settle) in settling {
            match self.settled(settle, now) {
                Settled::No => {}
                Settled::Yes => {
                    tracing::info!(step = i, "commander: command settled");
                    if let Some(p) = self.state.commander.paragraph.as_mut() {
                        p.steps[i].phase = Phase::Done;
                    }
                    changed = true;
                }
                Settled::Stuck => {
                    self.stop_paragraph("the new agent is waiting on a question");
                    return true;
                }
            }
        }
        let ready: Vec<usize> = self
            .state
            .commander
            .paragraph
            .as_ref()
            .map(|p| (0..p.steps.len()).filter(|&i| p.is_ready(i)).collect())
            .unwrap_or_default();
        for i in ready {
            self.read_step(i);
            changed = true;
        }
        if self
            .state
            .commander
            .paragraph
            .as_ref()
            .is_some_and(Paragraph::is_finished)
        {
            self.state.commander.paragraph = None;
            return true;
        }
        self.show_paragraph();
        changed
    }

    /// Ask Jev to read step `i`, against what is on screen now and what the
    /// steps it waits for did. Its answer comes back as
    /// [`AppEvent::CommanderRead`].
    fn read_step(&mut self, i: usize) {
        let Some(jev) = self.jev() else {
            return;
        };
        let catalog = super::input::commander_catalog(&self.state, &self.terminal_runtimes);
        let generation = self.next_generation();
        let Some(paragraph) = self.state.commander.paragraph.as_mut() else {
            return;
        };
        let (earlier, made) = paragraph.context(i);
        let snapshot = plan::Snapshot {
            line: paragraph.steps[i].said.clone(),
            catalog,
            earlier,
            made,
        };
        paragraph.steps[i].phase = Phase::Reading(generation);
        let line = paragraph.line.clone();
        let event_tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let result = plan::read(jev.as_ref(), &snapshot);
            if let Err(trouble) = &result {
                tracing::info!(error = %trouble, "commander: Jev could not read the line");
            }
            let _ = event_tx.blocking_send(AppEvent::CommanderRead {
                generation,
                line,
                result,
            });
        });
    }

    /// Jev's reading of one step. A sure reading of something that cannot
    /// lose work is carried out at once; anything else is held for `Enter`.
    pub(crate) fn commander_read(
        &mut self,
        generation: u64,
        line: &str,
        result: Result<Reading, String>,
    ) {
        if !self.line_still_there(line) {
            tracing::info!(generation, mode = ?self.state.mode, "commander: dropped a reading; the line is gone");
            return;
        }
        let Some(i) = self
            .state
            .commander
            .paragraph
            .as_ref()
            .filter(|p| p.line == line)
            .and_then(|p| p.reading(generation))
        else {
            tracing::info!(
                generation,
                "commander: dropped a reading no command waits for"
            );
            return;
        };
        let said = self.state.commander.paragraph.as_ref().unwrap().steps[i]
            .said
            .clone();
        if let Ok(reading) = &result {
            tracing::info!(
                step = i,
                read = %reading.command.describe(),
                confidence = reading.confidence,
                "commander: Jev read a command"
            );
        }
        let phase = match result {
            Ok(reading) if reading.confidence >= ACT_AT_ONCE && !reading.careful => {
                self.carry_out_step(i, reading.command);
                None
            }
            Ok(reading) => {
                let described = reading.command.describe();
                let shown = match &reading.runner_up {
                    Some(other) if !reading.careful => {
                        format!("{described}? or {other} — enter to {described}")
                    }
                    _ if reading.careful => format!("{described} — enter to confirm"),
                    _ => format!("{described}? — enter to confirm"),
                };
                Some(Phase::Held {
                    command: reading.command,
                    said: shown,
                })
            }
            Err(trouble) => {
                let catalog = super::input::commander_catalog(&self.state, &self.terminal_runtimes);
                Some(match intent::interpret(&said, &catalog) {
                    Ok(command) => Phase::Held {
                        said: format!("{} — {trouble}; enter to run anyway", command.describe()),
                        command,
                    },
                    Err(_) => Phase::Failed(trouble),
                })
            }
        };
        if let (Some(phase), Some(p)) = (phase, self.state.commander.paragraph.as_mut()) {
            p.steps[i].phase = phase;
        }
        self.pump_paragraph(Instant::now());
        self.sync_commander_clock(Instant::now());
    }

    /// Carry out step `i`. While other steps are still to come, the box
    /// opens again to show how the line is getting on, and what waits for
    /// this step waits for it to settle. A command that opened something of
    /// its own, such as settings or a rename prompt, stops the line there,
    /// because the keyboard now belongs to that.
    fn carry_out_step(&mut self, i: usize, command: Command) {
        self.while_busy(|app| app.carry_out_step_now(i, command));
    }

    /// Run `f` with the Commander's answers held back, then handle the ones
    /// that arrived meanwhile. Carrying out a command through the socket API
    /// handles other pending events part way through.
    fn while_busy(&mut self, f: impl FnOnce(&mut Self)) {
        let was_busy = std::mem::replace(&mut self.commander_busy, true);
        f(self);
        self.commander_busy = was_busy;
        if !was_busy {
            for event in std::mem::take(&mut self.commander_deferred) {
                self.handle_commander_event(event);
            }
        }
    }

    /// A Commander answer held back while a command was carried out.
    fn handle_commander_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::CommanderSplit {
                generation,
                line,
                result,
            } => self.commander_split(generation, &line, result),
            AppEvent::CommanderOrdered {
                generation,
                line,
                result,
            } => self.commander_ordered(generation, &line, result),
            AppEvent::CommanderRead {
                generation,
                line,
                result,
            } => self.commander_read(generation, &line, result),
            _ => {}
        }
    }

    fn carry_out_step_now(&mut self, i: usize, command: Command) {
        let did = command.describe();
        let several = self
            .state
            .commander
            .paragraph
            .as_ref()
            .is_some_and(|p| p.steps.len() > 1);
        let (made, flight) = self.dispatch(command, !several);
        let now = Instant::now();
        let Some(paragraph) = self.state.commander.paragraph.as_mut() else {
            return self.state.commander.finish();
        };
        let step = &mut paragraph.steps[i];
        step.did = Some(did);
        step.made = made;
        step.phase = Phase::Settling(Settle {
            not_before: now + SETTLE,
            flight,
            agent: made,
            give_up: now + AGENT_READY_TIMEOUT,
        });
        paragraph.last = Some(i);
        let all_carried_out = paragraph
            .steps
            .iter()
            .all(|s| matches!(s.phase, Phase::Settling(_) | Phase::Done));
        if all_carried_out {
            self.state.commander.finish();
            if self.state.mode == Mode::Commander {
                super::input::close_commander(&mut self.state);
            }
            return;
        }
        if !matches!(self.state.mode, Mode::Terminal | Mode::Navigate) {
            return self.stop_paragraph("a command opened something that takes the keyboard");
        }
        self.state.mode = Mode::Commander;
        self.show_paragraph();
    }

    /// Give up on the commands not yet carried out, and say which they were.
    fn stop_paragraph(&mut self, why: &str) {
        let rest: Vec<String> = self
            .state
            .commander
            .paragraph
            .as_ref()
            .map(|p| {
                p.steps
                    .iter()
                    .filter(|s| !matches!(s.phase, Phase::Settling(_) | Phase::Done))
                    .map(|s| s.said.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.state.commander.finish();
        if self.state.mode == Mode::Commander {
            super::input::close_commander(&mut self.state);
        }
        if !rest.is_empty() {
            self.commander_trouble::<()>(format!("{why}; not run: {}", rest.join("; ")));
        }
    }

    /// The line under the field while a line is under way: the first command
    /// waiting for `Enter`, else the first that could not be read, else how
    /// far it has got.
    fn show_paragraph(&mut self) {
        let Some(p) = self.state.commander.paragraph.as_ref() else {
            return;
        };
        if let Some(i) = p.first_held() {
            if let Phase::Held { said, .. } = &p.steps[i].phase {
                self.state.commander.reading = Some(Ok(format!("{}{said}", p.step_prefix(i))));
            }
            return;
        }
        if let Some((i, trouble)) = p
            .steps
            .iter()
            .enumerate()
            .find_map(|(i, s)| match &s.phase {
                Phase::Failed(trouble) => Some((i, trouble)),
                _ => None,
            })
        {
            self.state.commander.reading = Some(Err(format!(
                "{}{trouble} — enter to ask again",
                p.step_prefix(i)
            )));
            return;
        }
        let count = p.steps.len();
        let carried = p
            .steps
            .iter()
            .filter(|s| matches!(s.phase, Phase::Settling(_) | Phase::Done))
            .count();
        let reading = p
            .steps
            .iter()
            .filter(|s| matches!(s.phase, Phase::Reading(_)))
            .count();
        let waking = p
            .steps
            .iter()
            .any(|s| matches!(s.phase, Phase::Settling(settle) if settle.agent.is_some()));
        let mut parts = Vec::new();
        if let Some(did) = p.last.and_then(|i| p.steps[i].did.as_ref()) {
            parts.push(format!("✓ {did}"));
        }
        if count > 1 {
            parts.push(format!("{carried} of {count} done"));
        }
        if p.planning.is_some() {
            parts.push("planning the order".to_string());
        }
        if reading > 0 {
            parts.push(if count > 1 {
                format!("asking Jev about {reading}")
            } else {
                "asking Jev…".to_string()
            });
        }
        if waking {
            parts.push("waiting for an agent to be ready".to_string());
        }
        self.state.commander.reading = Some(Ok(parts.join(" · ")));
    }

    /// Whether a carried-out step has settled: its pause is over, its message
    /// has landed, and an agent it started is at its prompt. An agent still
    /// not there after the wait counts as settled anyway, unless it is stuck
    /// on a question of its own, such as whether to trust its folder: a
    /// prompt typed into that question would answer it.
    fn settled(&self, settle: Settle, now: Instant) -> Settled {
        use crate::detect::AgentState;
        if now < settle.not_before
            || settle
                .flight
                .is_some_and(|id| self.state.commander.is_carrying(id))
        {
            return Settled::No;
        }
        let Some(pane_id) = settle.agent else {
            return Settled::Yes;
        };
        let state = self
            .state
            .terminal_id_for_any_pane(pane_id)
            .and_then(|id| self.state.terminals.get(&id))
            .map(|terminal| terminal.state);
        match state {
            None | Some(AgentState::Idle) => Settled::Yes,
            Some(_) if now < settle.give_up => Settled::No,
            Some(AgentState::Blocked) => Settled::Stuck,
            Some(_) => Settled::Yes,
        }
    }

    /// Close the box and send `command` on a star to what it acts on. The
    /// command rides the star and happens when the star lands, so the box is
    /// seen giving the order and the workspace carrying it out: a switch lands
    /// on the space card or tab, a message or keys on the pane's cursor, a
    /// rename on the pane's name or the tab's label, a split or a zoom on the focused pane's name,
    /// a new tab on the tab bar's `+`, a new space on the sidebar's `+ new`.
    /// Opening or toggling one of herdr's panels has no place to land and
    /// happens at once.
    ///
    /// `show_target` brings a message's pane on screen first, so its star
    /// lands on the pane itself. A line of several commands passes false:
    /// commands running alongside may depend on what is on screen, and a
    /// message switching it would move that from under them; its star then
    /// lands on the pane's tab or space instead.
    ///
    /// Returns the star carrying it, which what waits for it waits for, or
    /// for a command that happened at once, the pane it started, if any.
    pub(crate) fn dispatch(
        &mut self,
        command: Command,
        show_target: bool,
    ) -> (Option<crate::layout::PaneId>, Option<u64>) {
        let from = super::input::commander_launch_point(&self.state);
        super::input::close_commander(&mut self.state);
        let Some(aim) = self.aim(&command) else {
            return (self.run_now(command), None);
        };
        let cargo = match command {
            Command::Send { to, text } => {
                let (Some(pane_id), Some(ws)) = (to.pane, self.state.workspaces.get(to.ws)) else {
                    return (None, None);
                };
                let workspace_id = ws.id.clone();
                let tab = ws.find_tab_index_for_pane(pane_id).unwrap_or(to.tab);
                let on_screen = self.state.active == Some(to.ws) && ws.active_tab_index() == tab;
                if !on_screen && show_target {
                    self.state.switch_workspace_tab(to.ws, tab);
                }
                Cargo::Message(Delivery {
                    workspace_id,
                    pane_id,
                    text,
                    label: to.label,
                })
            }
            command => {
                let workspace_id = command_entry(&command)
                    .and_then(|e| self.state.workspaces.get(e.ws))
                    .map(|ws| ws.id.clone())
                    .unwrap_or_default();
                Cargo::Command {
                    command,
                    workspace_id,
                }
            }
        };
        let id = self.state.commander.launch(from, aim, Some(cargo));
        (None, Some(id))
    }

    /// Carry out `command` at once. Returns the pane it started, if any.
    fn run_now(&mut self, command: Command) -> Option<crate::layout::PaneId> {
        let rest =
            super::input::perform_commander(&mut self.state, &mut self.terminal_runtimes, command)?;
        self.run_commander_request(rest)
    }

    /// What `command` acts on, as it is on screen before it happens, or
    /// `None` for a command with nothing on screen to act on. Spaces are named
    /// by id, since ids do not move when a space opens or closes.
    fn aim(&self, command: &Command) -> Option<Aim> {
        use crate::commander::intent::Action as A;
        let space_id = |ws: usize| self.state.workspaces.get(ws).map(|w| w.id.clone());
        let of = |entry: &intent::Entry| -> Option<Aim> {
            Some(match entry.kind {
                Kind::Space => Aim::Space {
                    workspace_id: space_id(entry.ws)?,
                },
                Kind::Tab => Aim::Tab {
                    workspace_id: space_id(entry.ws)?,
                    tab: entry.tab,
                },
                Kind::Pane => Aim::Pane(entry.pane?),
            })
        };
        let active = self.state.active;
        let active_ws = active.and_then(|ws| self.state.workspaces.get(ws));
        let tab_of_active = |tab: usize| -> Option<Aim> {
            Some(Aim::Tab {
                workspace_id: active_ws?.id.clone(),
                tab,
            })
        };
        let focused_in = |ws: usize| -> Option<Aim> {
            let workspace = self.state.workspaces.get(ws)?;
            match workspace.focused_pane_id() {
                Some(pane) if active == Some(ws) => Some(Aim::Pane(pane)),
                _ => Some(Aim::Space {
                    workspace_id: workspace.id.clone(),
                }),
            }
        };
        let space_step = |forward: bool| -> Option<Aim> {
            let order = self.state.visible_workspace_order();
            let current = active.unwrap_or(self.state.selected);
            let at = order.iter().position(|&ws| ws == current).unwrap_or(0);
            let n = order.len();
            let next = if forward {
                (at + 1) % n
            } else {
                (at + n - 1) % n
            };
            Some(Aim::Space {
                workspace_id: space_id(*order.get(next)?)?,
            })
        };
        match command {
            Command::Go(entry) | Command::Close(entry) => of(entry),
            Command::Rename { target, .. } => of(target),
            Command::Send { to, .. } | Command::Type { to, .. } | Command::Keys { to, .. } => {
                to.pane.map(Aim::Input)
            }
            Command::RemoveWorktree(space)
            | Command::LandWorktree(space)
            | Command::NewWorktree { space, .. } => Some(Aim::Space {
                workspace_id: space_id(space.ws)?,
            }),
            Command::NewTab { space, .. } => Some(Aim::NewTab {
                workspace_id: space_id(space.ws)?,
            }),
            Command::NewSpace { .. } | Command::Act(A::NewSpace) => Some(Aim::NewSpace),
            Command::StartAgent { space, .. } => focused_in(space.ws),
            Command::View { part, space, .. } => {
                use crate::api::schema::ViewPart as P;
                match (part, space) {
                    (P::AgentTable, _) => Some(Aim::AgentTable),
                    (P::SpaceAgents | P::SpaceGroup, Some(space)) => Some(Aim::Space {
                        workspace_id: space_id(space.ws)?,
                    }),
                    _ => Some(Aim::Sidebar),
                }
            }
            Command::Act(action) => {
                let ws = active?;
                let workspace = active_ws?;
                let tabs = workspace.tabs.len().max(1);
                let tab = workspace.active_tab_index();
                match action {
                    A::NewTab => Some(Aim::NewTab {
                        workspace_id: workspace.id.clone(),
                    }),
                    A::CloseTab | A::RenameTab => tab_of_active(tab),
                    A::NextTab => tab_of_active((tab + 1) % tabs),
                    A::PreviousTab => tab_of_active((tab + tabs - 1) % tabs),
                    A::CloseSpace | A::RenameSpace => Some(Aim::Space {
                        workspace_id: workspace.id.clone(),
                    }),
                    A::NextSpace => space_step(true),
                    A::PreviousSpace => space_step(false),
                    A::FocusLeft
                    | A::FocusRight
                    | A::FocusUp
                    | A::FocusDown
                    | A::LastPane
                    | A::NextAgent
                    | A::PreviousAgent
                    | A::SplitRight
                    | A::SplitLeft
                    | A::SplitDown
                    | A::SplitUp
                    | A::ClosePane
                    | A::Zoom
                    | A::Resize
                    | A::RenamePane => focused_in(ws),
                    _ => None,
                }
            }
        }
    }
    /// The point on screen a star aimed at `aim` flies to, as of the last
    /// frame laid out.
    pub(crate) fn aim_point(&self, aim: &Aim) -> V2 {
        let view = &self.state.view;
        let fallback = || V2::center_of(view.terminal_area);
        let ws_index = |id: &str| self.state.workspaces.iter().position(|ws| ws.id == id);
        let space_card = |ws: usize| -> Option<V2> {
            if let Some(card) = view.workspace_card_areas.iter().find(|c| c.ws_idx == ws) {
                let row = if card.rect.height >= 3 {
                    card.rect.y + 1
                } else {
                    card.rect.y
                };
                let name = self.state.workspaces[ws]
                    .display_name_from(&self.state.terminals, &self.terminal_runtimes);
                let x = card.rect.x + 4 + (name.chars().count() as u16 / 2);
                return Some(V2::from_cell(
                    x.min(card.rect.right().saturating_sub(1)),
                    row,
                ));
            }
            if self.state.sidebar_collapsed {
                let (area, _, _) = crate::ui::collapsed_sidebar_sections(view.sidebar_rect);
                let row = area.y + ws as u16;
                if area.width > 0 && row < area.bottom() {
                    return Some(V2::from_cell(area.x + area.width / 2, row));
                }
            }
            None
        };
        let tab_label = |ws: usize, tab: usize| -> Option<V2> {
            if self.state.active != Some(ws) {
                return space_card(ws);
            }
            match view.tab_hit_areas.get(tab) {
                Some(rect) if rect.width > 0 => Some(V2::center_of(*rect)),
                _ => space_card(ws),
            }
        };
        let pane_name = |pane: crate::layout::PaneId| -> Option<V2> {
            if let Some(title) = view.pane_title_hit_areas.iter().find(|t| t.pane_id == pane) {
                // The title starts with a corner and a dash; aim past them,
                // at the middle of the name.
                let name_width = title.rect.width.saturating_sub(4);
                return Some(V2::from_cell(
                    title.rect.x + 3 + name_width / 2,
                    title.rect.y,
                ));
            }
            if let Some(info) = self.state.pane_info_by_id(pane) {
                return Some(V2::from_cell(info.rect.x + 4, info.rect.y));
            }
            // Not on screen: its name where agents and panes are listed, so
            // the star reaches the agent before anything switches to it.
            let listed = view
                .agent_row_areas
                .iter()
                .filter(|row| row.pane_id == pane)
                .map(|row| row.rect)
                .chain(
                    view.pane_row_areas
                        .iter()
                        .filter(|row| row.pane_id == pane)
                        .map(|row| row.rect),
                )
                .find(|rect| rect.width > 0);
            if let Some(row) = listed {
                let x = crate::ui::listed_name_column(row) + 2;
                return Some(V2::from_cell(x.min(row.right().saturating_sub(1)), row.y));
            }
            if let Some(row) = view
                .agent_table
                .rows
                .iter()
                .find(|row| row.pane_id == pane && row.rect.width > 0)
            {
                let x = row.rect.x + 4;
                return Some(V2::from_cell(
                    x.min(row.rect.right().saturating_sub(1)),
                    row.rect.y,
                ));
            }
            let (ws, tab) = self
                .state
                .workspaces
                .iter()
                .enumerate()
                .find_map(|(i, ws)| ws.find_tab_index_for_pane(pane).map(|tab| (i, tab)))?;
            tab_label(ws, tab)
        };
        let pane_input = |pane: crate::layout::PaneId| -> Option<V2> {
            let Some(info) = self.state.pane_info_by_id(pane) else {
                return pane_name(pane);
            };
            let area = info.inner_rect;
            // The cursor counts even when the program hides it: an agent that
            // draws its own caret still leaves the real one where its input
            // goes.
            let cursor = self
                .state
                .workspaces
                .iter()
                .position(|ws| ws.find_tab_index_for_pane(pane).is_some())
                .and_then(|ws| {
                    self.state
                        .runtime_for_pane_in_workspace(&self.terminal_runtimes, ws, pane)
                })
                .and_then(|runtime| runtime.cursor_state(area, true));
            Some(match cursor {
                Some(cursor) => V2::from_cell(cursor.x, cursor.y),
                None => V2::center_of(area),
            })
        };
        let point = match aim {
            Aim::Pane(pane) => pane_name(*pane),
            Aim::Input(pane) => pane_input(*pane),
            Aim::Tab { workspace_id, tab } => {
                ws_index(workspace_id).and_then(|ws| tab_label(ws, *tab))
            }
            Aim::Space { workspace_id } => ws_index(workspace_id).and_then(space_card),
            Aim::NewTab { workspace_id } => ws_index(workspace_id).and_then(|ws| {
                let plus = view.new_tab_hit_area;
                if self.state.active == Some(ws) && plus.width > 0 {
                    Some(V2::center_of(plus))
                } else {
                    space_card(ws)
                }
            }),
            Aim::Sidebar => {
                let sidebar = view.sidebar_rect;
                (sidebar.width > 0)
                    .then(|| V2::from_cell(sidebar.x + sidebar.width.min(12) / 2, sidebar.y))
            }
            Aim::AgentTable => {
                let table = view.agent_table.area;
                (table.width > 0).then(|| V2::from_cell(table.x + 4, table.y))
            }
            Aim::NewSpace => {
                let button = self.state.sidebar_new_button_rect();
                if button.width > 0 {
                    Some(V2::center_of(button))
                } else {
                    self.state.active.and_then(space_card)
                }
            }
        };
        point.unwrap_or_else(fallback)
    }

    /// What a star carried, now that it has landed. Returns the pane it
    /// started, if it started an agent.
    fn land(&mut self, cargo: Cargo) -> Option<crate::layout::PaneId> {
        match cargo {
            Cargo::Message(delivery) => {
                self.deliver_commander_message(delivery);
                None
            }
            Cargo::Command {
                mut command,
                workspace_id,
            } => {
                // Spaces may have opened or closed while the star flew, which
                // moves every position after them; the space is found again
                // by its id.
                if let Some(entry) = command_entry_mut(&mut command) {
                    match self
                        .state
                        .workspaces
                        .iter()
                        .position(|ws| ws.id == workspace_id)
                    {
                        Some(ws) => entry.ws = ws,
                        None => {
                            let gone = format!("{} closed before the star arrived", entry.label);
                            return self.commander_trouble(gone);
                        }
                    }
                }
                let mut made = None;
                self.while_busy(|app| made = app.run_now(command));
                made
            }
        }
    }

    /// Record what a landed star did on the step it carried, and keep the
    /// box showing the line, since a command that has landed may have moved
    /// the keyboard. One that opened something taking the keyboard, such as a
    /// rename prompt, stops the line there.
    fn landed(&mut self, flight: u64, made: Option<crate::layout::PaneId>, box_open: bool) {
        let now = Instant::now();
        let Some(paragraph) = self.state.commander.paragraph.as_mut() else {
            return;
        };
        for step in &mut paragraph.steps {
            if let Phase::Settling(settle) = &mut step.phase {
                if settle.flight == Some(flight) {
                    step.made = made;
                    settle.agent = made;
                    settle.give_up = now + AGENT_READY_TIMEOUT;
                }
            }
        }
        match self.state.mode {
            Mode::Commander => {}
            Mode::Terminal | Mode::Navigate if box_open => {
                self.state.mode = Mode::Commander;
                self.show_paragraph();
            }
            Mode::Terminal | Mode::Navigate => {}
            _ => self.stop_paragraph("a command opened something that takes the keyboard"),
        }
    }

    /// Carry out what the CLI can also do, through the socket API request the
    /// CLI would send. A refusal is shown as a toast.
    fn run_commander_request(&mut self, command: Command) -> Option<crate::layout::PaneId> {
        let pane_id = |app: &App, entry: &intent::Entry| app.public_pane_id(entry.ws, entry.pane?);
        let method = match command {
            Command::Go(_) | Command::Send { .. } | Command::Act(_) => return None,
            Command::StartAgent {
                space,
                harness,
                task,
            } => {
                let pending = crate::composer::Pending {
                    cwd: self
                        .seed_cwd_from_workspace(space.ws)
                        .unwrap_or_else(|| self.resolve_new_terminal_cwd(None)),
                    harness,
                    task,
                    worktree: false,
                };
                return match self.start_pending(&pending) {
                    Ok((pane_id, cwd)) => {
                        self.focus_composer_started_agent(pane_id, false);
                        let where_it_went = crate::workspace::display_path_with_home(&cwd);
                        self.show_composer_toast(
                            ToastKind::Finished,
                            &format!("started {} in {where_it_went}", harness.name),
                            pending.message(),
                            None,
                        );
                        Some(pane_id)
                    }
                    Err(reason) => self.commander_trouble(reason),
                };
            }
            Command::Type { to, text } => match pane_id(self, &to) {
                Some(pane_id) => Method::PaneSendText(PaneSendTextParams { pane_id, text }),
                None => return self.commander_trouble(format!("{} is gone", to.label)),
            },
            Command::Keys { to, keys, .. } => match pane_id(self, &to) {
                Some(pane_id) => Method::PaneSendKeys(PaneSendKeysParams { pane_id, keys }),
                None => return self.commander_trouble(format!("{} is gone", to.label)),
            },
            Command::NewTab { space, name } => Method::TabCreate(TabCreateParams {
                workspace_id: Some(self.public_workspace_id(space.ws)),
                cwd: None,
                focus: true,
                label: name,
            }),
            Command::NewSpace { name } => Method::WorkspaceCreate(WorkspaceCreateParams {
                cwd: None,
                focus: true,
                label: name,
            }),
            Command::Close(entry) => match entry.kind {
                Kind::Space => Method::WorkspaceClose(WorkspaceTarget {
                    workspace_id: self.public_workspace_id(entry.ws),
                }),
                Kind::Tab => match self.public_tab_id(entry.ws, entry.tab) {
                    Some(tab_id) => Method::TabClose(TabTarget { tab_id }),
                    None => return self.commander_trouble(format!("{} is gone", entry.label)),
                },
                Kind::Pane => match pane_id(self, &entry) {
                    Some(pane_id) => Method::PaneClose(PaneTarget { pane_id }),
                    None => return self.commander_trouble(format!("{} is gone", entry.label)),
                },
            },
            Command::Rename { target, name } => match target.kind {
                Kind::Space => Method::WorkspaceRename(WorkspaceRenameParams {
                    workspace_id: self.public_workspace_id(target.ws),
                    label: name,
                }),
                Kind::Tab => match self.public_tab_id(target.ws, target.tab) {
                    Some(tab_id) => Method::TabRename(TabRenameParams {
                        tab_id,
                        label: name,
                    }),
                    None => return self.commander_trouble(format!("{} is gone", target.label)),
                },
                Kind::Pane => match pane_id(self, &target) {
                    Some(pane_id) => Method::PaneRename(PaneRenameParams {
                        pane_id,
                        label: Some(name),
                    }),
                    None => return self.commander_trouble(format!("{} is gone", target.label)),
                },
            },
            Command::NewWorktree { space, branch } => {
                Method::WorktreeCreate(WorktreeCreateParams {
                    workspace_id: Some(self.public_workspace_id(space.ws)),
                    branch,
                    focus: true,
                    ..WorktreeCreateParams::default()
                })
            }
            Command::RemoveWorktree(space) => Method::WorktreeRemove(WorktreeRemoveParams {
                workspace_id: self.public_workspace_id(space.ws),
                force: false,
            }),
            Command::LandWorktree(space) => Method::WorktreeLand(WorktreeLandParams {
                target: Some(self.public_workspace_id(space.ws)),
                all: false,
            }),
            Command::View {
                part,
                change,
                space,
            } => Method::ViewSet(crate::api::schema::ViewSetParams {
                part,
                change,
                workspace_id: space.map(|space| self.public_workspace_id(space.ws)),
                tab_id: None,
            }),
        };
        let response = self.handle_api_request(Request {
            id: "commander".into(),
            method,
        });
        let refusal = serde_json::from_str::<serde_json::Value>(&response)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
            });
        match refusal {
            Some(refusal) => self.commander_trouble(refusal),
            None => None,
        }
    }

    /// Say in a toast why a command could not be carried out. Returns nothing,
    /// for whatever the caller was going to return.
    fn commander_trouble<T>(&mut self, trouble: String) -> Option<T> {
        tracing::warn!(error = %trouble, "commander action failed");
        let previous_toast = self.state.toast.clone();
        self.state.toast = Some(ToastNotification {
            kind: ToastKind::NeedsAttention,
            title: "commander".to_string(),
            context: trouble,
            target: None,
        });
        self.sync_toast_deadline(previous_toast);
        None
    }

    /// Keep the frame clock running exactly while the Commander has something
    /// moving, or a line waiting on a step to settle.
    pub(crate) fn sync_commander_clock(&mut self, now: Instant) {
        let waiting = self.state.commander.paragraph.as_ref().is_some_and(|p| {
            p.steps
                .iter()
                .any(|s| matches!(s.phase, Phase::Settling(_)))
        });
        if self.state.commander.is_animating() {
            self.commander_frame_deadline
                .get_or_insert(now + super::ANIMATION_INTERVAL);
        } else if waiting {
            self.commander_frame_deadline
                .get_or_insert(now + SETTLE_POLL);
            self.commander_last_frame = None;
        } else {
            self.commander_frame_deadline = None;
            self.commander_last_frame = None;
        }
    }

    /// Advance the stars if a frame is due, and move a waiting line on.
    /// Returns whether anything changed.
    pub(crate) fn tick_commander(&mut self, now: Instant) -> bool {
        if self
            .commander_frame_deadline
            .is_none_or(|deadline| now < deadline)
        {
            return false;
        }
        let dt = self
            .commander_last_frame
            .map_or(super::ANIMATION_INTERVAL, |last| now.duration_since(last))
            .min(MAX_STEP);
        self.commander_last_frame = Some(now);
        self.commander_frame_deadline = None;
        let animating = !self.state.commander.flights.is_empty();

        let area = self.state.commander.frame;
        let mut landed = Vec::new();
        for index in 0..self.state.commander.flights.len() {
            if self.state.commander.flights[index].trail.is_none() {
                let to = self.aim_point(&self.state.commander.flights[index].aim);
                let flight = &mut self.state.commander.flights[index];
                flight.trail = Some(Trail::new(flight.from, to, area));
            }
            let flight = &mut self.state.commander.flights[index];
            let arrived = flight
                .trail
                .as_mut()
                .is_some_and(|trail| trail.update(dt.as_secs_f32(), area.width, area.height));
            if arrived {
                if let Some(cargo) = flight.cargo.take() {
                    landed.push((flight.id, cargo));
                }
            }
        }
        for (id, cargo) in landed {
            let box_open = self.state.mode == Mode::Commander;
            let made = self.land(cargo);
            self.landed(id, made, box_open);
        }
        self.state
            .commander
            .flights
            .retain(|f| f.cargo.is_some() || f.is_visible());
        let advanced = self.pump_paragraph(now);
        self.sync_commander_clock(now);
        animating || advanced
    }

    /// Paste the message into its pane and submit it. The pane is found again
    /// by its space's id, because spaces may have moved while the star flew.
    fn deliver_commander_message(&mut self, delivery: Delivery) {
        let pane = self
            .state
            .workspaces
            .iter()
            .position(|ws| ws.id == delivery.workspace_id)
            .and_then(|ws_idx| {
                self.lookup_runtime_sender(ws_idx, delivery.pane_id)
                    .map(|runtime| super::api_helpers::send_prompt(runtime, &delivery.text))
            });
        let trouble = match pane {
            Some(Ok(())) => return,
            Some(Err((_, message))) => message,
            None => format!("{} closed before the message arrived", delivery.label),
        };
        tracing::warn!(target = %delivery.label, "commander delivery failed");
        self.commander_trouble::<()>(trouble);
    }
}

/// The one space, tab or pane a command acts on, whose space is found again
/// by id when its star lands.
fn command_entry(command: &Command) -> Option<&intent::Entry> {
    match command {
        Command::Go(entry)
        | Command::Close(entry)
        | Command::RemoveWorktree(entry)
        | Command::LandWorktree(entry) => Some(entry),
        Command::Rename { target, .. } => Some(target),
        Command::Send { to, .. } | Command::Type { to, .. } | Command::Keys { to, .. } => Some(to),
        Command::NewTab { space, .. }
        | Command::StartAgent { space, .. }
        | Command::NewWorktree { space, .. } => Some(space),
        Command::View { space, .. } => space.as_ref(),
        _ => None,
    }
}

fn command_entry_mut(command: &mut Command) -> Option<&mut intent::Entry> {
    match command {
        Command::Go(entry)
        | Command::Close(entry)
        | Command::RemoveWorktree(entry)
        | Command::LandWorktree(entry) => Some(entry),
        Command::Rename { target, .. } => Some(target),
        Command::Send { to, .. } | Command::Type { to, .. } | Command::Keys { to, .. } => Some(to),
        Command::NewTab { space, .. }
        | Command::StartAgent { space, .. }
        | Command::NewWorktree { space, .. } => Some(space),
        Command::View { space, .. } => space.as_mut(),
        _ => None,
    }
}
