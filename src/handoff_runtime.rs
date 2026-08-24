use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HandoffRuntimeState {
    pub pane_id: u32,
    pub child_pid: u32,
    pub rows: u16,
    pub cols: u16,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
    #[serde(default)]
    pub keyboard_protocol_flags: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_protocol_ansi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_state: Option<crate::pane::InputState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_history_ansi: Option<String>,
}

impl HandoffRuntimeState {
    #[cfg(unix)]
    pub fn with_pane_id(mut self, pane_id: crate::layout::PaneId) -> Self {
        self.pane_id = pane_id.raw();
        self
    }
}

/// Runtime pieces imported from a previous server over the handoff socket.
///
/// The struct exists on every platform so restore signatures stay
/// platform-neutral, but only Unix can carry a real master fd; Windows
/// never populates an import and always takes the restart path.
// On Windows the import path never runs, so the payload stays unread.
#[derive(Debug)]
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) struct ImportedHandoffRuntime {
    #[cfg(unix)]
    pub master_fd: std::os::fd::RawFd,
    pub state: HandoffRuntimeState,
}
