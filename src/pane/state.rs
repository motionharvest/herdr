use crate::terminal::TerminalId;

/// Viewport state for a pane.
///
/// Terminal identity, cwd, labels, and agent metadata live in TerminalState.
#[derive(Clone)]
pub struct PaneState {
    pub attached_terminal_id: TerminalId,
    /// Whether the finish has been acknowledged. False = "Done": the agent
    /// finished and nobody has said so out loud yet. Only clicking the marker
    /// sets this, because looking at a pane is something that happens by
    /// accident and losing the marker to an accident is losing the answer to
    /// "which one finished". Clicking the check turns it back into the done
    /// dot. A later run that finishes puts the dot back on, even if the last
    /// finish was acknowledged.
    pub seen: bool,
    /// Whether the agent has finished a run since it last worked. Together with
    /// `seen` this is the whole marker: not finished shows nothing, finished
    /// and unacknowledged shows the done dot, finished and acknowledged shows
    /// the check until it is clicked again or the agent finishes another run.
    pub completed: bool,
    /// Render all cell colors as the muted palette color (user toggled Dim).
    pub dimmed: bool,
}

impl PaneState {
    pub fn new(attached_terminal_id: TerminalId) -> Self {
        Self {
            attached_terminal_id,
            seen: true,
            completed: false,
            dimmed: false,
        }
    }
}
