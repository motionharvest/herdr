# Continuity

## Goal

Peek mode chrome uses accent (cyan on synthwave), and the spaces sidebar deselects the docked agent while peek is active.

## Plans

- 2026-09-02T13:05Z [USER] Peek border = accent; sidebar agent deselected during peek.

## Decisions

- 2026-09-02T13:05Z [DECISION] Peek pane chrome (`PaneTitleMode::Peeking`) uses `palette.accent` (muted when host unfocused), not `focused_pane_border` / focus.
- 2026-09-02T13:05Z [DECISION] `focused_agent_row` in the spaces sidebar returns `None` while `agent_peek` is set, so layout focus under the overlay does not keep a docked agent highlighted. Space outline stays.

## Progress

- 2026-09-02T13:05Z [CODE] Pane chrome + sidebar selection updated; unit tests added for accent peek border and sidebar deselection.

## Discoveries

- 2026-09-02T13:05Z [CODE] Synthwave: accent `#36F9F6`, focus `#F445F7`. Focused docked panes stay pink; peek should read as cyan.

## Outcomes

- 2026-09-02T13:10Z [TOOL] fmt + clippy clean. New peek/sidebar tests pass. Full nextest: 2546 passed; same 3 pre-existing integration failures as before (`wait_agent_status_exits_when_idle_status_matches`, `cross_area_agent_process_survives_detach_and_reattach`, `events_subscribe_streams_output_and_agent_status_events`).
- 2026-09-02T13:05Z [TOOL] `just test-one peeked_pane_border` and `peeking_deselects_the_docked_agent` passed.
