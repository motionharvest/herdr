//! The Commander: a one-line box at the bottom of the frame that does what it
//! is told.
//!
//! A hotkey opens it from anywhere, with the keyboard already in it. What is
//! typed is read by [`intent`] into one of herdr's existing actions, and the
//! line under the field says what that reading is before anything happens.
//! `Enter` does it; a paste does it at once, because a pasted line is already
//! finished. A command that changes the screen sends a star: [`trail`] draws
//! it from the box to what the command acts on, such as a pane's cursor for a
//! message or a tab's label for a switch. A command acting on something already on
//! screen happens when its star lands; one that makes something or moves the
//! keyboard happens at once, and its star shows where.
//!
//! With a TypeSafe key set, [`plan`] reads the line instead: it asks Jev, in
//! two requests, which of everything herdr and its CLI can do the line wants,
//! and fills in what that needs from the line's own words. A confident reading
//! of something that cannot lose work is carried out at once; anything else is
//! said under the field and waits for `Enter`.
//!
//! A line can hold several commands. Jev cuts it into them, [`order`] asks
//! Claude Code running Opus which must wait for which, and the commands then
//! run as that allows: ones that wait for nothing run at the same time.
//!
//! Nothing here touches a terminal or draws anything. This is what the box
//! holds; `app::commander` acts on it and `ui::commander` draws it.

pub mod intent;
pub mod jev;
pub mod order;
pub mod plan;
pub mod trail;

use ratatui::layout::Rect;

use crate::composer::TextField;
use crate::layout::PaneId;

/// A message on its way to a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// The space by id rather than by position, because a space opened or
    /// closed while the star is in the air moves every position after it.
    pub workspace_id: String,
    pub pane_id: PaneId,
    pub text: String,
    /// Who it is for, as the Commander named them.
    pub label: String,
}

/// Where a star flies: the spot on screen of what its command acts on. It is
/// found on the first frame tick after the star is sent, from the frame laid
/// out last. Each falls back to a wider place that is on screen: a pane not
/// shown is aimed at its tab, a tab not shown at its space's card, and a
/// space card the sidebar hides at the middle of the panes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aim {
    /// A pane's name in its top border.
    Pane(PaneId),
    /// Where a pane takes what is typed into it: its cursor, or the middle
    /// of the pane when the cursor is not inside it.
    Input(PaneId),
    /// A tab's label in the tab bar, by its space's id and its position.
    Tab { workspace_id: String, tab: usize },
    /// A space's card in the sidebar.
    Space { workspace_id: String },
    /// The tab bar's `+`, which opens a tab in that space.
    NewTab { workspace_id: String },
    /// The sidebar's `+ new`, which opens a space.
    NewSpace,
    /// The top of the sidebar, where it and its spaces section fold.
    Sidebar,
    /// The agent table's heading, where it folds.
    AgentTable,
}

/// What a star carries, which happens when it lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cargo {
    /// A message pasted into a pane and submitted.
    Message(Delivery),
    /// A command carried out, such as a switch or a rename. The space it
    /// acts on is kept by id, since spaces opened or closed while the star
    /// flies move every position after them.
    Command {
        command: intent::Command,
        workspace_id: String,
    },
}

/// A star on its way. Several can be in the air at once, since commands of a
/// line that do not depend on each other run together.
#[derive(Debug, Clone)]
pub struct Flight {
    /// Tells this flight apart, so a command can wait for its own star.
    pub id: u64,
    /// Where the star leaves from.
    pub from: trail::V2,
    pub aim: Aim,
    /// The star, once aimed; `None` until the first frame tick after it is
    /// sent.
    pub trail: Option<trail::Trail>,
    /// What happens when it lands, until it does. A star sent to show where
    /// something already happened carries nothing. The light it leaves keeps
    /// fading after it lands.
    pub cargo: Option<Cargo>,
}

impl Flight {
    pub fn is_visible(&self) -> bool {
        self.trail.as_ref().is_none_or(trail::Trail::is_visible)
    }
}

#[derive(Debug, Clone, Default)]
pub struct CommanderState {
    pub field: TextField,
    /// Where the box sits, as of the last frame laid out while it was open.
    /// The star leaves from here.
    pub area: Rect,
    /// The whole frame, as of the last layout. The star flies within it.
    pub frame: Rect,
    /// What submitting the field would do, or why it cannot be read, or how a
    /// paragraph is getting on. Worked out when something changes rather than
    /// where it is drawn, because reading a line walks every space, tab and
    /// pane.
    pub reading: Option<Result<String, String>>,
    /// Stars on their way.
    pub flights: Vec<Flight>,
    /// Counts the stars sent, to give each its id.
    pub flights_sent: u64,
    /// Counts the requests made for the box, so an answer for a line that has
    /// since been edited or abandoned is recognised and dropped.
    pub generation: u64,
    /// The request cutting the line into commands, by its generation.
    pub asking: Option<u64>,
    /// The line being carried out, command by command.
    pub paragraph: Option<Paragraph>,
}

/// A line of one or more commands, carried out as their order allows.
///
/// Each command is read only when the commands it waits for are done, against
/// what is on screen by then and what those commands did, so `switch to the
/// fifth space and add an agent` adds the agent where the first command went,
/// and `then prompt it` finds the agent the second one started. Commands that
/// wait for nothing run at the same time.
#[derive(Debug, Clone)]
pub struct Paragraph {
    /// The line as typed. Editing the field abandons what has not run.
    pub line: String,
    pub steps: Vec<Step>,
    /// For each step, the earlier steps it waits for. `None` while the
    /// planner is still working it out; the first step needs nothing and
    /// starts anyway.
    pub after: Option<order::After>,
    /// The planner request in flight, by its generation.
    pub planning: Option<u64>,
    /// The step carried out most recently, for saying what was just done.
    pub last: Option<usize>,
}

/// One command of a paragraph.
#[derive(Debug, Clone)]
pub struct Step {
    /// The words it was cut from.
    pub said: String,
    pub phase: Phase,
    /// What was done, once it was.
    pub did: Option<String>,
    /// The pane it started, if it started an agent.
    pub made: Option<PaneId>,
}

/// Where a step has got to.
#[derive(Debug, Clone)]
pub enum Phase {
    /// Waiting for the steps before it, or for the plan.
    Waiting,
    /// Jev is reading it; the request's generation.
    Reading(u64),
    /// Read, and waiting for `Enter`, because the reading is unsure or the
    /// command could lose work. `said` is the sentence shown for it.
    Held {
        command: intent::Command,
        said: String,
    },
    /// Could not be read; `Enter` asks again.
    Failed(String),
    /// Carried out, and settling before what waits for it may start.
    Settling(Settle),
    Done,
}

impl Paragraph {
    pub fn new(line: String, clauses: Vec<String>) -> Self {
        Self {
            line,
            steps: clauses
                .into_iter()
                .map(|said| Step {
                    said,
                    phase: Phase::Waiting,
                    did: None,
                    made: None,
                })
                .collect(),
            after: None,
            planning: None,
            last: None,
        }
    }

    /// The steps step `i` waits for, or `None` while that is not yet known.
    pub fn waits_for(&self, i: usize) -> Option<&[usize]> {
        match &self.after {
            Some(after) => after.get(i).map(Vec::as_slice),
            None if i == 0 => Some(&[]),
            None => None,
        }
    }

    /// Whether step `i` may be read now.
    pub fn is_ready(&self, i: usize) -> bool {
        matches!(self.steps[i].phase, Phase::Waiting)
            && self
                .waits_for(i)
                .is_some_and(|deps| deps.iter().all(|&d| self.is_done(d)))
    }

    pub fn is_done(&self, i: usize) -> bool {
        matches!(self.steps[i].phase, Phase::Done)
    }

    pub fn is_finished(&self) -> bool {
        (0..self.steps.len()).all(|i| self.is_done(i))
    }

    /// `step 2 of 4: ` for a paragraph of several commands, nothing for one.
    pub fn step_prefix(&self, i: usize) -> String {
        if self.steps.len() > 1 {
            format!("step {} of {}: ", i + 1, self.steps.len())
        } else {
            String::new()
        }
    }

    /// The first step held for `Enter`, which `Enter` answers.
    pub fn first_held(&self) -> Option<usize> {
        self.steps
            .iter()
            .position(|s| matches!(s.phase, Phase::Held { .. }))
    }

    /// The step a request's answer belongs to.
    pub fn reading(&self, generation: u64) -> Option<usize> {
        self.steps
            .iter()
            .position(|s| matches!(s.phase, Phase::Reading(g) if g == generation))
    }

    /// What the steps step `i` waits for did, for reading it in context, and
    /// the pane the latest of them started.
    pub fn context(&self, i: usize) -> (Vec<plan::Done>, Option<PaneId>) {
        let deps = self.waits_for(i).unwrap_or(&[]);
        let earlier = (0..i)
            .filter(|&d| self.is_done(d))
            .filter_map(|d| {
                Some(plan::Done {
                    said: self.steps[d].said.clone(),
                    did: self.steps[d].did.clone()?,
                })
            })
            .collect();
        let made = deps.iter().rev().find_map(|&d| self.steps[d].made);
        (earlier, made)
    }
}

/// What a carried-out step waits for before the steps after it may start.
#[derive(Debug, Clone, Copy)]
pub struct Settle {
    /// A short pause, so a switch or a split has happened before the next
    /// command is read against the screen.
    pub not_before: std::time::Instant,
    /// The star carrying the step, by its id, until it lands.
    pub flight: Option<u64>,
    /// A started agent, which takes nothing until it is at its prompt.
    pub agent: Option<PaneId>,
    /// When to stop waiting for that agent.
    pub give_up: std::time::Instant,
}

impl CommanderState {
    /// Whether anything needs the frame clock: a star waiting to leave, one in
    /// the air, or light still fading.
    pub fn is_animating(&self) -> bool {
        self.flights.iter().any(Flight::is_visible)
    }

    /// Whether the star `id` still carries something that has not happened.
    pub fn is_carrying(&self, id: u64) -> bool {
        self.flights.iter().any(|f| f.id == id && f.cargo.is_some())
    }

    /// Send a star from `from` to `aim`, carrying `cargo`. Returns its id.
    pub fn launch(&mut self, from: trail::V2, aim: Aim, cargo: Option<Cargo>) -> u64 {
        self.flights_sent += 1;
        let id = self.flights_sent;
        self.flights.push(Flight {
            id,
            from,
            aim,
            trail: None,
            cargo,
        });
        id
    }

    /// Let go of everything about the line: the field, what was said about
    /// it, any request awaited, and what of the paragraph has not run.
    pub fn finish(&mut self) {
        self.field.clear();
        self.reading = None;
        self.asking = None;
        self.paragraph = None;
    }
}
