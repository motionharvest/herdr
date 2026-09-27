# Continuity

## Goal

Restore per-space tab-strip chrome between the agent table and panes, and make the agent table collapsible to a one-row expand chrome (persisted in the session snapshot).

## Plans

- 2026-09-10T12:00Z [USER] Tabs back under agent panel; collapse agent panel for room. Plan: restore old tab chrome on existing `Workspace.tabs`; collapse leaves one-row chrome; persist collapse like `sidebar_collapsed`.
- 2026-09-02T13:05Z [USER] Peek border = accent; sidebar agent deselected during peek.

## Decisions

- 2026-09-10T12:00Z [USER] Tab strip sits between agent table and panes (not above the table). Reuse existing Tab model/CLI/API; do not invent a second tab list.
- 2026-09-10T12:00Z [USER] Collapsed agent table keeps a one-row expand control; composer stays visible.
- 2026-09-10T12:00Z [USER] `agent_table_collapsed` persists in session snapshot.
- 2026-09-02T13:05Z [DECISION] Peek pane chrome (`PaneTitleMode::Peeking`) uses `palette.accent` (muted when host unfocused), not `focused_pane_border` / focus.
- 2026-09-02T13:05Z [DECISION] `focused_agent_row` in the spaces sidebar returns `None` while `agent_peek` is set, so layout focus under the overlay does not keep a docked agent highlighted. Space outline stays.

## Progress

- 2026-09-10T20:10Z [CODE] Tab rename via double-click / right-click menu; sidebar lists Tab rows under each space with agents under the active tab.
- 2026-09-10T18:40Z [CODE] Tab strip restored under agent table; keybinds/mouse/dialogs wired; agent table collapse + snapshot persistence landed. Focused unit tests passing.
- 2026-09-10T12:00Z [CODE] Implementation starting: Phase 1 collapse, then restore `src/ui/tabs.rs` + input/keybinds.
- 2026-09-02T13:05Z [CODE] Pane chrome + sidebar selection updated; unit tests added for accent peek border and sidebar deselection.

## Discoveries

- 2026-09-10T12:00Z [CODE] Tab data plane never left (`Workspace.tabs`, API/CLI, snapshots). Only chrome/keybinds/mouse were removed in `5bf5481`.
- 2026-09-02T13:05Z [CODE] Synthwave: accent `#36F9F6`, focus `#F445F7`. Focused docked panes stay pink; peek should read as cyan.

## Outcomes

- 2026-09-10T18:40Z [TOOL] `cargo check` clean. `just test-one` passed for `collapsed_agent_table`, `inactive_tab_blinks`, `prompt_new_tab_name`, `capture_saves`. Integration binaries compile.
- 2026-09-02T13:10Z [TOOL] fmt + clippy clean. New peek/sidebar tests pass. Full nextest: 2546 passed; same 3 pre-existing integration failures as before (`wait_agent_status_exits_when_idle_status_matches`, `cross_area_agent_process_survives_detach_and_reattach`, `events_subscribe_streams_output_and_agent_status_events`).
- 2026-09-02T13:05Z [TOOL] `just test-one peeked_pane_border` and `peeking_deselects_the_docked_agent` passed.
