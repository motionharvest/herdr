# Continuity

## Goal

Explain how a Grok session in Windows Terminal updates the tab title, and make herdr's Summary column follow that same live title.

## Plans

- 2026-08-27T12:55Z [USER] Wire OSC 0/2 window titles from pane PTYs into Summary, matching Grok's host-tab updates.

## Decisions

- 2026-08-27T12:55Z [CODE] Grok sets the host tab with crossterm `SetTitle` (`ESC ] 0 ; … BEL`) from `[ui.notifications.title]`. Default items: action-required, spinner, activity, session-name, grok.
- 2026-08-27T12:55Z [CODE] libghostty-vt already stores OSC 0/2 as `GHOSTTY_TERMINAL_DATA_TITLE`. Herdr now reads it after each PTY write instead of adding another OSC scanner.
- 2026-08-27T12:55Z [DECISION] Presentation order: manual summary > harness metadata title > live OSC title > probed session title. Claude statusline titles still win. Grok has no hook title, so OSC fills Summary while it works.
- 2026-08-27T12:55Z [DECISION] Strip Braille spinner glyphs and a trailing `grok` label so the agent table does not flicker every spinner frame. Activity changes still replace Summary.

## Progress

- 2026-08-27T12:55Z [CODE] Implemented getter, PTY change detection, `OscTitleChanged` event, `TerminalState.osc_title`, docs, changelog.

## Discoveries

- 2026-08-27T12:55Z [TOOL] Grok binary strings: `crossterm SetTitle produces valid UTF-8`, `crates/codegen/xai-grok-pager/src/notifications/title.rs`, default `[ui.notifications.title] items = ["action-required", "spinner", "activity", "session-name", "grok"]`. Also OSC 9;4 progress bar, unused here.

## Outcomes

- 2026-08-27T13:20Z [TOOL] Unit tests 2382 passed. Clippy and fmt clean. Three headless integration tests (`cross_area_agent_process_survives_detach_and_reattach`, `wait_agent_status_exits_when_idle_status_matches`, `events_subscribe_streams_output_and_agent_status_events`) fail on this checkout's HEAD without the change as well.
