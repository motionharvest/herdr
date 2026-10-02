//! The Commander's frame clock, and delivering the message its star carries.
//!
//! While a star is waiting to leave, in the air, or still fading, the app
//! ticks it about sixty times a second. The first tick after a message is sent
//! aims the star, because only then has a frame been laid out with the target
//! pane on screen. The tick on which the star lands pastes the message into
//! the pane and presses Enter.

use std::time::{Duration, Instant};

use super::App;
use crate::app::state::{ToastKind, ToastNotification};
use crate::commander::{trail::Trail, trail::V2, Delivery};

/// A slow frame steps the star at most this far, so a stall does not make it
/// jump across the screen.
const MAX_STEP: Duration = Duration::from_millis(50);

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

    fn submit_commander(&mut self) {
        super::input::submit_commander(&mut self.state, &mut self.terminal_runtimes);
        self.sync_commander_clock(Instant::now());
    }

    /// Keep the frame clock running exactly while the Commander has something
    /// moving.
    pub(crate) fn sync_commander_clock(&mut self, now: Instant) {
        if self.state.commander.is_animating() {
            self.commander_frame_deadline
                .get_or_insert(now + super::ANIMATION_INTERVAL);
        } else {
            self.commander_frame_deadline = None;
            self.commander_last_frame = None;
        }
    }

    /// Advance the star if a frame is due. Returns whether anything changed.
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

        if let Some(launch) = self.state.commander.launch.take() {
            let to = self
                .state
                .pane_info_by_id(launch.delivery.pane_id)
                .map(|info| V2::center_of(info.inner_rect))
                .unwrap_or_else(|| V2::center_of(self.state.view.terminal_area));
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_nanos() as u64);
            self.state.commander.trail = Some(Trail::new(
                launch.from,
                to,
                self.state.commander.frame,
                seed,
            ));
            self.state.commander.delivery = Some(launch.delivery);
        }

        let area = self.state.commander.frame;
        let landed = self
            .state
            .commander
            .trail
            .as_mut()
            .is_some_and(|trail| trail.update(dt.as_secs_f32(), area.width, area.height));
        if landed {
            if let Some(delivery) = self.state.commander.delivery.take() {
                self.deliver_commander_message(delivery);
            }
        }
        if !self
            .state
            .commander
            .trail
            .as_ref()
            .is_some_and(Trail::is_visible)
        {
            self.state.commander.trail = None;
        }
        self.sync_commander_clock(now);
        true
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
        tracing::warn!(target = %delivery.label, error = %trouble, "commander delivery failed");
        let previous_toast = self.state.toast.clone();
        self.state.toast = Some(ToastNotification {
            kind: ToastKind::NeedsAttention,
            title: "commander".to_string(),
            context: trouble,
            target: None,
        });
        self.sync_toast_deadline(previous_toast);
    }
}
