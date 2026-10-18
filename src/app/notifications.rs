//! Cancelable agent notifications shared by local and headless runtimes.
use std::time::{Duration, Instant};

use crate::detect::{Agent, AgentState};
use crate::layout::PaneId;
use crate::terminal::{EffectiveStateChange, TerminalId};

use super::actions::{
    active_tab_suppresses_notifications, notification_context, notification_sound_for_state_change,
    notification_toast_for_state_change,
};
use super::state::{AppState, ToastKind, ToastNotification, ToastTarget};
use super::App;

pub(crate) struct PendingAgentNotification {
    workspace_id: String,
    terminal_id: TerminalId,
    agent_label: String,
    known_agent: Option<Agent>,
    previous_state: AgentState,
    state: AgentState,
    deadline: Instant,
}

pub(crate) struct AgentNotificationDelivery {
    pub toast: Option<ToastNotification>,
    pub sound: Option<crate::sound::Sound>,
    pub client_notification: Option<ToastNotification>,
}

impl AppState {
    pub(crate) fn record_agent_notification(
        &mut self,
        ws_idx: usize,
        pane_id: PaneId,
        change: &EffectiveStateChange,
    ) {
        // Every effective transition invalidates the previous pending alert.
        self.pending_agent_notifications.remove(&pane_id);
        let is_active = self.pane_is_in_active_tab(ws_idx, pane_id);
        let suppress = active_tab_suppresses_notifications(
            is_active,
            self.outer_terminal_focus,
            self.notify_active_tab,
        );
        if notification_sound_for_state_change(suppress, change.previous_state, change.state)
            .is_none()
            && notification_toast_for_state_change(suppress, change.previous_state, change.state)
                .is_none()
        {
            return;
        }
        let Some(agent_label) = change.agent_label.clone() else {
            return;
        };
        let Some(pane) = self.workspaces[ws_idx].pane_state(pane_id) else {
            return;
        };
        let now = Instant::now();
        let delay = Duration::from_secs(
            self.toast_config
                .delay_seconds
                .min(crate::config::MAX_TOAST_DELAY_SECONDS),
        );
        self.pending_agent_notifications.insert(
            pane_id,
            PendingAgentNotification {
                workspace_id: self.workspaces[ws_idx].id.clone(),
                terminal_id: pane.attached_terminal_id.clone(),
                agent_label,
                known_agent: change.known_agent,
                previous_state: change.previous_state,
                state: change.state,
                deadline: now.checked_add(delay).unwrap_or(now),
            },
        );
        if delay.is_zero() {
            self.drain_due_agent_notifications(now);
        }
    }

    pub(crate) fn next_pending_agent_notification_deadline(&self) -> Option<Instant> {
        self.pending_agent_notifications
            .values()
            .map(|pending| pending.deadline)
            .min()
    }

    pub(crate) fn drain_due_agent_notifications(&mut self, now: Instant) -> bool {
        let due: Vec<_> = self
            .pending_agent_notifications
            .iter()
            .filter_map(|(&pane, pending)| (pending.deadline <= now).then_some(pane))
            .collect();
        let mut changed = false;
        for pane_id in due {
            let Some(pending) = self.pending_agent_notifications.remove(&pane_id) else {
                continue;
            };
            let Some(ws_idx) = self
                .workspaces
                .iter()
                .position(|ws| ws.id == pending.workspace_id)
            else {
                continue;
            };
            let ws = &self.workspaces[ws_idx];
            let Some(pane) = ws.pane_state(pane_id) else {
                continue;
            };
            if pane.attached_terminal_id != pending.terminal_id {
                continue;
            }
            let Some(terminal) = self.terminals.get(&pane.attached_terminal_id) else {
                continue;
            };
            if terminal.state != pending.state
                || terminal.effective_agent_label() != Some(pending.agent_label.as_str())
                || terminal.effective_known_agent() != pending.known_agent
            {
                continue;
            }
            let is_active = self.pane_is_in_active_tab(ws_idx, pane_id);
            let suppress = active_tab_suppresses_notifications(
                is_active,
                self.outer_terminal_focus,
                self.notify_active_tab,
            );
            let sound = notification_sound_for_state_change(
                suppress,
                pending.previous_state,
                pending.state,
            )
            .filter(|_| self.sound.allows(pending.known_agent));
            let build_toast = |kind| ToastNotification {
                kind,
                title: format!(
                    "{} {}",
                    super::actions::toast_agent_label(&pending.agent_label),
                    match kind {
                        ToastKind::NeedsAttention => "needs attention",
                        ToastKind::Finished => "finished",
                        ToastKind::UpdateInstalled => "updated",
                    }
                ),
                context: notification_context(ws, &ws.display_name(), ws_idx, pane_id),
                target: Some(ToastTarget {
                    workspace_id: pending.workspace_id.clone(),
                    pane_id,
                }),
            };
            let toast = if matches!(
                self.toast_config.delivery,
                crate::config::ToastDelivery::Herdr
            ) {
                notification_toast_for_state_change(
                    is_active && !self.notify_active_tab,
                    pending.previous_state,
                    pending.state,
                )
                .map(build_toast)
            } else {
                None
            };
            if let Some(toast) = &toast {
                self.toast = Some(toast.clone());
                changed = true;
            }
            let client_notification = notification_toast_for_state_change(
                suppress,
                pending.previous_state,
                pending.state,
            )
            .map(build_toast);
            self.ready_agent_notifications
                .push(AgentNotificationDelivery {
                    toast,
                    sound,
                    client_notification,
                });
        }
        changed
    }
}

pub(crate) fn refresh_notification_context(
    state: &AppState,
    runtimes: &crate::terminal::TerminalRuntimeRegistry,
    toast: &mut ToastNotification,
) {
    let Some(target) = &toast.target else { return };
    let Some((ws_idx, ws)) = state
        .workspaces
        .iter()
        .enumerate()
        .find(|(_, ws)| ws.id == target.workspace_id)
    else {
        return;
    };
    let workspace_label = ws.display_name_from(&state.terminals, runtimes);
    toast.context = notification_context(ws, &workspace_label, ws_idx, target.pane_id);
}

impl App {
    /// Local clients consume the same deliveries that headless servers forward.
    pub(crate) fn flush_agent_notifications(&mut self, now: Instant) -> bool {
        let previous_toast = self.state.toast.clone();
        let changed = self.state.drain_due_agent_notifications(now);
        let mut deliveries = std::mem::take(&mut self.state.ready_agent_notifications);
        for delivery in &mut deliveries {
            if let Some(toast) = &mut delivery.client_notification {
                refresh_notification_context(&self.state, &self.terminal_runtimes, toast);
            }
        }
        for delivery in &mut deliveries {
            if let Some(toast) = &mut delivery.toast {
                refresh_notification_context(&self.state, &self.terminal_runtimes, toast);
                self.state.toast = Some(toast.clone());
            }
        }
        if self.state.local_sound_playback || self.local_terminal_notifications {
            for delivery in deliveries {
                if self.state.local_sound_playback {
                    if let Some(sound) = delivery.sound {
                        crate::sound::play(sound, &self.state.sound);
                    }
                }
                if self.local_terminal_notifications {
                    if let Some(toast) = delivery.client_notification {
                        match self.state.toast_config.delivery {
                            crate::config::ToastDelivery::Terminal => {
                                let _ = crate::terminal_notify::show_notification(
                                    &toast.title,
                                    Some(&toast.context),
                                );
                            }
                            crate::config::ToastDelivery::System => {
                                let _ = crate::platform::show_desktop_notification(
                                    &toast.title,
                                    Some(&toast.context),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
        } else {
            self.state.ready_agent_notifications = deliveries;
        }
        let changed = changed || self.state.toast != previous_toast;
        self.sync_toast_deadline(previous_toast);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::AppEvent;
    use crate::workspace::Workspace;

    fn background_agent() -> (AppState, PaneId) {
        let mut state = AppState::test_new();
        state.workspaces = vec![
            Workspace::test_new("active"),
            Workspace::test_new("background"),
        ];
        state.ensure_test_terminals();
        state.sound.enabled = true;
        state.active = Some(0);
        state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        let pane = state.workspaces[1].tabs[0].root_pane;
        report(&mut state, pane, AgentState::Working);
        (state, pane)
    }

    fn report(state: &mut AppState, pane_id: PaneId, next: AgentState) {
        state.handle_app_event(AppEvent::StateChanged {
            pane_id,
            agent: Some(Agent::Pi),
            state: next,
            visible_blocker: next == AgentState::Blocked,
            visible_idle: next == AgentState::Idle,
            visible_working: next == AgentState::Working,
            process_exited: false,
            observed_at: Instant::now(),
        });
    }

    #[test]
    fn transient_idle_is_canceled_when_work_resumes() {
        let (mut state, pane) = background_agent();
        report(&mut state, pane, AgentState::Idle);
        let deadline = state.next_pending_agent_notification_deadline().unwrap();
        assert!(state.toast.is_none());
        assert!(state.ready_agent_notifications.is_empty());
        report(&mut state, pane, AgentState::Working);
        state.drain_due_agent_notifications(deadline);
        assert!(state.next_pending_agent_notification_deadline().is_none());
        assert!(state.toast.is_none());
        assert!(state.ready_agent_notifications.is_empty());
    }

    #[test]
    fn stable_completion_delivers_once_at_deadline_even_with_repeated_idle_reports() {
        let (mut state, pane) = background_agent();
        report(&mut state, pane, AgentState::Idle);
        let deadline = state.next_pending_agent_notification_deadline().unwrap();
        report(&mut state, pane, AgentState::Idle);
        assert_eq!(
            state.next_pending_agent_notification_deadline(),
            Some(deadline)
        );
        assert!(!state.drain_due_agent_notifications(deadline - Duration::from_nanos(1)));
        assert!(state.drain_due_agent_notifications(deadline));
        assert_eq!(state.toast.as_ref().unwrap().kind, ToastKind::Finished);
        assert_eq!(state.ready_agent_notifications.len(), 1);
        assert_eq!(
            state.ready_agent_notifications[0].sound,
            Some(crate::sound::Sound::Done)
        );
        assert!(!state.drain_due_agent_notifications(deadline + Duration::from_secs(1)));
        assert_eq!(state.ready_agent_notifications.len(), 1);
    }

    #[test]
    fn pending_attention_is_canceled_on_resume_and_dialog_dismissal_is_not_completion() {
        for next in [AgentState::Working, AgentState::Idle] {
            let (mut state, pane) = background_agent();
            report(&mut state, pane, AgentState::Blocked);
            let deadline = state.next_pending_agent_notification_deadline().unwrap();
            report(&mut state, pane, next);
            state.drain_due_agent_notifications(deadline);
            assert!(state.ready_agent_notifications.is_empty());
            assert!(state.toast.is_none());
        }
    }

    #[test]
    fn delivery_rechecks_focus_and_notify_active_tab() {
        for notify_active in [false, true] {
            let (mut state, pane) = background_agent();
            report(&mut state, pane, AgentState::Idle);
            let deadline = state.next_pending_agent_notification_deadline().unwrap();
            state.active = Some(1);
            state.outer_terminal_focus = Some(true);
            state.notify_active_tab = notify_active;
            state.drain_due_agent_notifications(deadline);
            assert_eq!(state.toast.is_some(), notify_active);
            let delivered = &state.ready_agent_notifications[0];
            assert_eq!(delivered.sound.is_some(), notify_active);
            assert_eq!(delivered.client_notification.is_some(), notify_active);
        }
    }

    #[test]
    fn delivery_rechecks_sound_preferences_even_when_popups_are_disabled() {
        for enabled in [false, true] {
            let (mut state, pane) = background_agent();
            state.toast_config.delivery = crate::config::ToastDelivery::Off;
            report(&mut state, pane, AgentState::Idle);
            let deadline = state.next_pending_agent_notification_deadline().unwrap();
            state.sound.enabled = enabled;
            state.drain_due_agent_notifications(deadline);
            assert!(state.toast.is_none());
            assert_eq!(state.ready_agent_notifications[0].sound.is_some(), enabled);
        }
    }

    #[test]
    fn missing_pane_or_changed_terminal_or_agent_cannot_deliver_stale_alert() {
        for mutation in 0..3 {
            let (mut state, pane) = background_agent();
            report(&mut state, pane, AgentState::Idle);
            let deadline = state.next_pending_agent_notification_deadline().unwrap();
            let terminal_id = state.workspaces[1]
                .pane_state(pane)
                .unwrap()
                .attached_terminal_id
                .clone();
            match mutation {
                0 => {
                    state.workspaces.remove(1);
                }
                1 => {
                    state.terminals.remove(&terminal_id);
                }
                _ => {
                    state
                        .terminals
                        .get_mut(&terminal_id)
                        .unwrap()
                        .set_detected_state(Some(Agent::Codex), AgentState::Idle);
                }
            }
            state.drain_due_agent_notifications(deadline);
            assert!(state.toast.is_none());
            assert!(state.ready_agent_notifications.is_empty());
        }
    }

    #[test]
    fn zero_delay_preserves_immediate_delivery() {
        let (mut state, pane) = background_agent();
        state.toast_config.delay_seconds = 0;
        report(&mut state, pane, AgentState::Idle);
        assert!(state.next_pending_agent_notification_deadline().is_none());
        assert!(state.toast.is_some());
        assert_eq!(state.ready_agent_notifications.len(), 1);
    }
}
