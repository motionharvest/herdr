//! The Commander: a one-line box at the bottom of the frame that does what it
//! is told.
//!
//! A hotkey opens it from anywhere, with the keyboard already in it. What is
//! typed is read by [`intent`] into one of herdr's existing actions, and the
//! line under the field says what that reading is before anything happens.
//! `Enter` does it; a paste does it at once, because a pasted line is already
//! finished. A message for a pane flies there first: [`trail`] draws a star
//! from the box to the middle of the pane, and the message is pasted and
//! submitted the moment it lands.
//!
//! Nothing here touches a terminal or draws anything. This is what the box
//! holds; `app::commander` acts on it and `ui::commander` draws it.

pub mod intent;
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

/// A message that has been sent off but whose star has not left yet. The star
/// cannot be aimed until the frame showing the target pane has been laid out,
/// so it leaves on the first frame tick after that.
#[derive(Debug, Clone)]
pub struct Launch {
    pub from: trail::V2,
    pub delivery: Delivery,
}

#[derive(Debug, Clone, Default)]
pub struct CommanderState {
    pub field: TextField,
    /// Where the box sits, as of the last frame laid out while it was open.
    /// The star leaves from here.
    pub area: Rect,
    /// The whole frame, as of the last layout. The star flies within it.
    pub frame: Rect,
    /// What submitting the field would do, or why it cannot be read. Worked
    /// out on each edit rather than where it is drawn, because reading it
    /// walks every space, tab and pane.
    pub reading: Option<Result<String, String>>,
    /// A message waiting for its star to leave.
    pub launch: Option<Launch>,
    /// The star in flight, and the light it leaves until that fades.
    pub trail: Option<trail::Trail>,
    /// The message the star in flight is carrying.
    pub delivery: Option<Delivery>,
}

impl CommanderState {
    /// Whether anything needs the frame clock: a star waiting to leave, one in
    /// the air, or light still fading.
    pub fn is_animating(&self) -> bool {
        self.launch.is_some() || self.trail.as_ref().is_some_and(trail::Trail::is_visible)
    }
}
