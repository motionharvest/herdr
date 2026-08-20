# Changelog

## Unreleased

### Fixed
- `Delete agent + worktree` now follows that agent's folder, the same way `Land on <branch>` already does. The click used to require saved worktree membership on the space holding the row, so an agent Herdr had started in a linked checkout could raise `This workspace is not a Herdr-managed worktree checkout.` after a rebuild, a hide, or a dock into another space. The confirmation removes that agent's checkout and ends that agent. A space that still holds another agent is left in place.

### Changed
- The agent table's Directory column now shows the parent folder and the current folder as `parent/current`, so `~/lab/herdr` reads `lab/herdr` instead of only `herdr`. A path with no parent still shows the current name.
- A composer start with Worktree checked creates the checkout and peeks that agent over the current layout. The agent is hidden, so the splits underneath do not change and the row is muted. Starting used to open a space of its own for that checkout. Clearing Worktree still cuts the agent in beside the pane that had focus.
- Double-clicking an agent row (anywhere but the name) now peeks that agent in FOCUS over the current layout. The splits underneath do not change, so a shell running `npm run dev` keeps running. BACK, HIDE, or double-clicking that row again leaves the peek. A single click on a hidden agent selects the row and no longer docks it over the focused pane, which is what ended the shell. Dragging still places the agent: an edge splits, the middle replaces. Double-clicking the name still renames.
- Visible agent rows use the default foreground for name, summary, harness, run, and idle. Hidden rows use the muted color for those cells, including the name, so a hidden agent is visibly quieter than one on screen. Folder color, git status, and the gutter marks are unchanged.
- The pane chrome on an agent pane now says `HIDE` instead of `✕`. Clicking it still takes the pane out of the layout and leaves the agent running in the table, so you can open it again from its row. A shell pane still says `✕` and still closes. Closing used to drop the agent from the table when Herdr no longer counted the pane as an agent at that moment, even though a reported session or a named row still meant there was an agent to keep.
- The agent-table row menu now says `Delete agent + worktree` instead of `Delete agent / worktree...`.
- Git Status in the agent table is now green when that checkout is clean and yellow when it is dirty. The column used to write every branch in mauve.

## [0.10.0] - 2026-08-19

### Added
- Grok panes now resume their native conversation after a Herdr server restart. Herdr already restored the pane and relaunched `grok`, which starts a new conversation; Claude Code resumed because an official hook reported a session id. `herdr integration install grok` writes a SessionStart hook that reports that id, and restore then runs `grok --resume <id>`. A pane that never reported an id still restarts from its original launch command.
- Agents started from the composer work in Herdr-managed linked worktrees when the Worktree box next to Task is checked, regardless of harness. The box starts checked. Clearing it starts the agent in the chosen directory. Herdr creates the checkout itself instead of passing Claude Code's `-w`, so Codex and every other editing harness receive the same isolation and the resulting work has one lifecycle. `herdr agent start <name> --worktree [branch] -- <argv...>` exposes the same fast path to scripts. A folder outside a repository still starts in place. If the start directory is already a linked worktree, composer start and `agent start --worktree` reuse that checkout instead of creating another, and an already-open workspace for it is reused too. `Auto` remains a router rather than creating an empty checkout for itself.
- Managed worktrees can land by agent name with `herdr agent worktree land [name]` or with `herdr worktree land --all`. Landing rebases onto the parent checkout's branch, runs the optional `[worktrees].verify` argv, and fast-forwards the parent; failures stay with the originating checkout instead of leaving the parent half-merged. The agent table's `Land on main` action prompts that agent to do the work rather than running git from the click. `[worktrees].auto_land` still runs the mechanical path when an agent becomes idle and defaults off.
- The agent table now shows run duration and idle age in their own columns. Run time counts while an agent works and freezes when it finishes; idle age starts at that transition and survives session restore, so retained agents reveal both how long their last turn took and how old the result is.
- Clicking an agent-table heading sorts the rows by that column. Directory, name, summary, kind, and git status go A–Z. Run and Idle go longest first. The sort rewrites the session-wide row order, so a drag still moves a row from there and new agents still append at the bottom.
- Clicking an agent's row and pressing `Delete` now removes that agent, after a question. The click already focuses the agent, so the row holds the keyboard for exactly one key: `Delete` — or `Backspace`, which is what the key labeled delete sends on a Mac keyboard — asks about the row that was clicked, and any other key releases the hold and goes to the agent as it always did, so clicking a row and typing still types into it. The question names the agent and answers to `Enter` for remove and `Esc` for cancel, with a button for each. Removing does what the row's own `Delete agent` menu entry does: an agent in a pane is ended and that pane is closed, and a set-down agent is dropped outright.
- A Codex agent's row now says what it is working on. Claude Code names its own sessions and herdr's statusline reports that name, which is what fills the summary column; Codex never names a session and its hook carries only the session id, so the column stayed blank for it. What Codex does record is the prompt, so the newest one typed into the session is read out of its rollout on disk and stands in as the title, folded to one line and cut to fit. The `AGENTS.md` preamble and the context blocks Codex writes into the same role are skipped, because those are the harness briefing its model rather than a task. Each scan starts where the previous one stopped, so a session that has written megabytes since is read only for what it added. A title a harness reports for itself still wins outright, so a Claude Code pane is unaffected.
- A Grok agent's row now shows the session title Grok already wrote. The official hook reports the session id at SessionStart, which is before Grok names the conversation, so the summary column stayed blank even after the title appeared in the prompt border and the dashboard. That name lives in the session's `summary.json` as `generated_title`, and herdr now reads it the same way it reads a Codex rollout: the file is resolved from the reported id, and a change to it updates the column. The same read also takes `current_model_id` and `reasoning_effort` for the model column. A session that has not been named yet still shows no summary until Grok writes one.
- Directory and agent dropdown rows now fill with the accent color while the mouse is over them, so the item a click will choose is visible before the click. Hovering only previews the target; it does not move the keyboard selection or change the value shown in the control.
- Double-clicking an agent's name in the agent table now opens the existing rename dialog for that agent. A single click still focuses the agent and a drag still carries it; other cells in the row keep those behaviors without becoming rename shortcuts.
- The frame is now three bands, read top to bottom: the composer, the agent table, and the panes. The agent table replaces the spaces sidebar — one row per agent, written across instead of down: its name, what it reports it is doing, the folder it is in, its harness, how long it has been running or idle, and its branch. The margin beside a row says what wants you: a spinner turning is work happening, a dot is an agent that finished while you were not looking, a diamond is an agent stopped on a question. The table takes the rows it needs and no more, up to a third of the frame, because the panes below it are the part actually being watched; agents past what one column of rows holds start a second group of columns to the right, so the table grows sideways into room it already has before it starts scrolling. Clicking a row focuses that agent, right-clicking it opens the agent's menu — which now carries its space's actions too, since the rows are the only chrome spaces have left — and the global menu hangs off the right end of the heading row.
- Closing a pane no longer closes the agent inside it. The agent is set down instead: it keeps running, keeps its table row — written a shade quieter, since nothing on screen is showing it — and keeps reporting its state, and a completion it reaches while set down wears the done marker like any background agent. Clicking its row opens it over the focused pane; dragging it gives exact placement, cutting a pane at an edge or taking it whole over the middle. An agent displaced from the middle is set down in its turn, so the two motions compose without anything being lost. Closing a space sets its agents down the same way, and so do `herdr pane close` and `herdr tab close`. Only `Delete agent`, on the row's own menu, actually ends one: it ends the session and closes the pane the agent was in. An agent is saved with the session and comes back with it whether it is set down or open in a pane; with agent restore enabled, Herdr resumes its native session when one was reported and restarts its original command when none was. A live handoff carries its terminal across an update the same way it carries a pane's.
- An agent started from the composer is shown as soon as it starts. The keyboard leaves the band, so typing a task and sending it lands you on the agent that received it. A managed worktree stays hidden and opens in peek over the current layout. An agent started without a worktree is cut in beside the pane that had focus.
- A composer now sits above the tab bar, on every space and every layout: three rows at the top of the frame holding where to work, who works, and what to do. Type a path, `Enter`, type a task, `Enter`, and an agent is running. It takes no `Tab` at all — the keyboard starts wherever the first thing left to do is, and settling a directory or an agent hands it on to the task. `prefix+/` reaches the band, a click anywhere in it does the same, and Esc leaves what you typed where it is for later. Starting an agent used to mean opening a pane, changing directory in it, and remembering the harness's own flag for handing over a prompt.
- The agent list offers `Auto` first, which is the row that is not a harness. It stands for whichever agent already owns the work, and it answers that by starting Claude Code on `/who <task>` — the skill behind that command reads the running agents, picks the one whose transcript shows it did this work, hands the message over and switches to its pane. Choosing `Auto` writes the `/who` in front of the prompt, so the field reads as the thing that will actually be sent rather than hiding what the row does. Routing lives in the skill because the evidence lives in the transcripts, and an agent is already the thing that can read them. `Terminal` is the other non-agent row: it opens the configured interactive shell in the chosen directory and runs the task as a command, then leaves the shell alive to dock and keep using.
- The two dropdowns are lists you rarely need to open. Closed, `↑` and `↓` change the value outright, because the value is what is being chosen and opening a list to choose it answers nothing; `Enter` opens the list when you want to see it, and typing at the folder control opens it on a path field instead. The folder list is read from the panes at the moment it is opened rather than kept warm, and it hangs over the panes rather than pushing them down — a band that changed height would resize every pane under it, and resizing a pane resizes the agent's terminal.
- A path being typed at the directory control now lists the folders it could mean, up to five of them, in the box that opens under it. Anything holding a `/`, or beginning with `~`, names a place to look in, so the folders on disk under that place are offered and each further letter narrows them; a bare word names nowhere on disk yet, so it is matched against the directories herdr already knows, and `herdr` finds `~/lab/herdr` without any of the path in front of it. Hidden folders wait for the dot that hides them, because a list led by `.cache` and `.git` is a list with the answer pushed off the end of it. What is typed stays the value until `↓` steps into the list — then a row is lit, `↑` off the top row steps back into the text, and another letter does the same, since a row picked against earlier letters was picked against a different question. `Enter` takes the row pointed at, or the path typed when it names a folder outright, or else the folder at the top of the list, so a few letters and `Enter` are a whole answer. A click on a row takes it the way it always has. Typing a directory used to mean typing all of it, exactly, with no way to see whether it existed until `Enter` said it did not.
- A task goes on the harness's command line rather than being typed into it once it is up, so the agent is working the moment it draws and there is no prompt that was not listening yet to swallow the message. How a task reaches a harness is written down per harness, because the differences are real: `opencode "fix the tests"` opens a directory called `fix the tests`, since opencode's first argument is a project. `Shift-Enter` and `Ctrl-J` write another line, so a task can be as long as it needs to be and `Enter` still means send.
- Only agents this machine can actually start are listed. The list is read from `PATH` once, at startup, and the agent you left showing comes back on the next run — saved by name rather than by row, so a name still means the same agent when the list it was chosen from has changed.
- Dragging a pane onto another pane's edge now cuts that pane in two and pins the dragged one to that edge. Where the pointer sits decides what the drop means: over the middle of a pane the two trade places, as they always have, and within a quarter of one of its edges the pane is split and the dragged pane takes the half against that edge. The four distances are measured as shares of the span they cross, so a wide short pane answers the way it looks rather than the way its numbers read, and a pane too small to be cut a given way answers that edge as its middle instead of making half a pane nobody can read. The drop is drawn before you let go — a bordered box over exactly the room the pane would take, reading `Drop to split` or `Drop to swap` — because the same movement means two different things and nothing else on screen says which. Rearranging a tab used to mean closing a pane and splitting again somewhere else, which lost the pane's scrollback with it; the pinned pane keeps its id, so its terminal, its history, and its name all move with it.
- Agent rows now keep one session-wide order independent of their spaces, tabs, and pane geometry. Dragging a row moves it in that list and an accent marker shows the exact before-or-after slot before release; the arrangement is saved with the session, and newly detected agents append at the bottom. A set-down agent can still be dragged out of the table onto a pane to dock it.

### Changed
- Choosing `Land on <branch>` now prompts the agent to commit outstanding work, then run `herdr agent worktree land`. That command lands the linked worktree of the agent's current folder. The previous prompt ran `herdr worktree land`, which used the space's saved membership, so a herdr agent sitting in a Fifthseason space was offered `Land on aaron-vibe`.
- `herdr agent worktree land [target]` lands the linked worktree of that agent's folder onto its parent checkout. With no target it uses the process cwd. `herdr worktree land <agent-name>` now does the same. Workspace ids and `--all` still land from space membership.
- The agent table's summary column is only as wide as the longest title in that group. It used to take every leftover cell, which pushed Directory and the columns after it to the far side of the table even when the titles were short.
- The agent-table row menu now says `Delete agent` for ordinary agents, which ends the agent and closes the pane it was in. `Delete agent / worktree...` stays at the bottom of every agent row menu and is grayed out when that agent's directory is not a linked worktree, so a parent checkout or a plain folder cannot be removed from the menu. Linked-worktree rows still offer it enabled because that action removes the checkout. `Land on <branch>` stays on linked-worktree rows and prompts that agent to commit and land onto the parent checkout, instead of herdr running git behind a status overlay. It is grayed out when that checkout already shares the parent branch's commit and has nothing left to land, or when the agent is not in the worktree directory. `herdr worktree land` still runs the mechanical rebase, verify, and fast-forward.
- The composer's directory list no longer shows linked worktree checkouts. Agents started from the composer already work in those checkouts, so every new one added another row and the parent repository was the one that got hard to find. A worktree is listed as that parent instead, and two checkouts of the same repository stay one folder.
- New managed worktrees now land inside the source repository at `.herdr/worktrees/<branch-slug>` instead of under `~/.herdr/worktrees/<repo>/<branch-slug>`. Because the checkout sits inside the repo, the agent table shows the repository path and `worktree <branch>` the same way it already did for `.claude/worktrees`. Herdr writes the relative directory into that repo's `.git/info/exclude` so the parent checkout does not see the nested files as untracked. An explicit absolute or `~/` `[worktrees].directory` keeps the previous global layout.
- The linked-worktree row menu now names the parent of that agent's folder (`Land on main`, `Land on release`, or `Land on parent` if that branch cannot be read). Choosing it prompts that agent to commit and land onto that parent branch. `herdr agent worktree land` still commits a dirty worktree, rebases, runs verify, and fast-forwards the parent. The `!` in Git Status means that checkout has uncommitted files; a dirty parent checkout blocks the fast-forward. A failed CLI land still leaves a red ✕ on the row until the next attempt.
- Deleting a linked agent space now means `Delete agent / worktree...`: Herdr refuses uncommitted files or commits that have not landed before showing the destructive confirmation, then removes both the checkout and its local branch. The previous action removed only the checkout, left branches accumulating, and did not distinguish landed work from commits that would be lost.
- An agent working in a linked worktree nested inside its own repository now displays the repository path with `worktree` before its branch instead of repeating `.claude/worktrees/<name>` and the same name again. External sibling worktrees keep their own path.
- Pane borders now show only the pane name in their title, without the terminal or agent type, working directory, branch, or git status. The focused pane's top edge now uses the same single-line rule as the rest of its border instead of switching that edge to a double line.
- The focused agent row now uses the theme's recessed surface instead of its brighter raised surface, keeping the selection close to black on dark themes without hard-coding a color that breaks light themes.
- Tabs are gone. A space holds one pane layout, and the agent table is the index of everything running — a tab bar over each space said the same thing a second way, and the keybinds, CLI dialogs, and chrome that maintained it were dead weight. Saved sessions that carry several tabs still restore: every pane comes back, reachable through its agent's table row.
- Spaces keep no chrome of their own: the sidebar, its cards, its folder groupings, and the spaces section of the mobile switcher are gone with it. A space is still what holds a pane layout and its git identity — switching, renaming, closing, and the worktree actions all live on, reached through the table rows and the same keybinds as before. The `ui.sidebar_width` family, `ui.agent_panel_scope`, `ui.prompt_new_tab_name`, the tab keybinds, and `keys.toggle_sidebar` are no longer read from config.
- A config warning now draws over every overlay instead of under them, so a broken config file cannot hide behind the onboarding screen.
- The done marker now waits to be clicked. It used to come off the moment its agent was looked at — focusing the pane, switching to the tab holding it, or returning to a herdr that was in the background cleared every marker on the way past — so a click aimed at a pane took the answer to "which one finished" with it. Now looking shows an agent without acknowledging it, and clicking the marker itself is the one thing that clears it. Clicking leaves a check in the marker's place, so an agent that finished still reads differently from one that never ran, and the check stays until that agent works again. The marker's cell and the gutter beside it are the button, on the table row and on the mobile switcher's row alike, and clicking it does not also jump to the agent.

### Fixed
- The agent-table row menu no longer offers `Move to new space`. That item appeared when an agent had more than one pane and would peel it into a new space. Spaces are an internal layout, not something you create, so the leftover is gone.
- Clicking `Land on <branch>` now runs `herdr agent prompt <Agent Name>` with the land instructions. The click queued a prompt, but the running `herdr server` loop never drained it, and the send used the pane rather than the agent name.
- The yellow config warning now has a ✕ at its far right. Clicking that cross dismisses the banner, so a one-shot message such as `This workspace is not a Herdr-managed worktree checkout.` can be put away after you have read it instead of staying on screen after you leave the agent that raised it.
- Clicking away from a directory path being typed now settles it the way Enter and Tab already did. The typed text used to be thrown away the moment the field lost the click, so a finished path vanished if you clicked the task, the agent, or a pane instead of pressing Enter.
- Grok panes no longer chime done and show the unseen marker while the turn is still drawing. The local detector still used a short tool-verb list and the retired `Ctrl+c:cancel` footer; current Grok 4.6 working chrome is a spinner line ending in `[stop]` plus `Esc:cancel`. The screen rules now follow `herdrdev/herdr` `grok.toml` 2026.07.16.2, with `Ctrl+x:shortcuts` accepted as the live alias of that file's `Ctrl+.:shortcuts`.
- Grok panes no longer chime done while background work or a live wait is still on screen. After a turn printed `Worked for`, Grok 1.0.5 keeps the idle footer and shows `◎ 1 command still running`, `Waiting for response…`, or `Thinking…` above the prompt; the detector treated that footer as finished. Those lines, the `[stop]` chip even when more text follows it, and `Ctrl+b:send to bg` now keep the pane working.
- The done marker now survives a restart. It was saved for a set-down agent and not for one in a pane, so a session that came back brought its agents without their markers: a row of checks earned by acknowledging each finish read as a row of agents that had never run, and a finish nobody had looked at yet lost the dot asking to be looked at. The marker is saved with the pane it belongs to now, both halves of it — whether the agent has finished and whether that finish was acknowledged — and a session written before it was saved comes back the way it always did, with no marker at all.
- A test that pointed `PATH` at a scratch folder put it back afterwards; it used to leave the whole test run without `git`, which is what made a dozen unrelated tests fail whenever it ran first.

## [0.9.1] - 2026-08-14

### Added
- The context menu on an agent — on its sidebar row or on its pane — now offers `Reset agent`, which starts a fresh session inside the running process by typing the harness's own reset command: `/clear` for Claude Code, Gemini, and Copilot, `/new` for Codex, opencode, and Amp. Escape is sent first, so a half-typed prompt is not left prefixed to the command and a working agent is interrupted before the reset lands. Harnesses whose reset command herdr does not know show no such entry, rather than typing a guess into the pane.
- The same menus offer `Close agent`, which ends the agent and leaves you the pane. Ending an agent used to mean closing its pane and losing the split with it, or reaching into the pane and quitting the agent by hand. An agent running as a job under the pane's shell is signaled on its own and the shell keeps the terminal; an agent that is the terminal's own child gets a shell respawned in its place. The signal ladder is the one a closing pane already uses — hangup, then terminate, then kill — and the waits between the steps run off the input thread, so a slow exit never stalls typing.
- A sidebar agent row now carries what the agent reports it is doing: the session title its harness set, then any custom status it announced, wrapped under the row's name and state. That is the same text `herdr agent status` prints, and reading it meant leaving the sidebar for a shell. The text is capped at three rows, with everything past the cap folded into the last one and cut, so a long-winded agent cannot push the rest of the list off screen.

### Changed
- Sidebar agent rows now head with the folder the agent is working in — the pane's cwd, with its git branch and dirty marker trailing in a dimmer tone the way they do on the pane's own title bar — instead of the name of the tab holding the pane. A tab name only promises that the panes under it share a folder, and nothing makes that true, so the row said less about whether this was the agent you meant to prompt than the pane's title bar did. Agents working in the same folder share one header and are indented under it, so a space now reads as a list of folders rather than a flat list of rows. A sidebar too narrow for the whole label drops the path's leading folders first, marked `…/`, so the folder the agent is actually in and its branch both survive as long as they can; the branch goes only when the last folder and the branch will not sit together. A pane with no cwd yet falls back to its tab name.
- Dragging in the sidebar now moves rows within that arrangement: an agent reorders among the agents it shares a folder with, and dragging a folder header moves the whole folder, and every agent under it, among its space's folders. An agent used to be free to land anywhere in its space's list, including under a folder heading it was not working in, which left the list stating something untrue about the pane.
- A space card whose agent list is folded away now wears a hollow dot in place of its state dot. A closed card and a card with nothing under it looked identical, so an empty list gave no way to tell a space with no agents from one with its agents put away.

## [0.9.0] - 2026-08-14

### Added
- A pane's assigned name — the `Olivia` or `Mei` the sidebar already shows — now works anywhere a pane id does, in every CLI command and API method that takes one, case-insensitively. Ids compact when panes close, so a script that captured `1-3` could be talking to a different pane a minute later; the name stays with its pane for the life of the terminal. Names also survive restarts now: the session file saves each pane's terminal identity, which the names are derived from, so a restored session comes back wearing the same names it shut down with. Pane responses in the API carry the name as a `name` field, and colliding names are suffixed `Olivia-2` rather than `Olivia 2`, because a name with a space in it would need quoting in the very commands the name is for.
- `herdr agent prompt <target> <text>` delivers a message to the agent in another pane: the text is pasted, then Enter is pressed, so it arrives and submits as one message. `agent send` types text without submitting and `pane run` sends command text plus Enter for shells, but neither was right for prompting an agent — send left the message sitting unsubmitted in the composer, and run's per-line writes could hand a multi-line prompt to the agent as several messages.
- `herdr agent status [target]` prints one plain line per agent — its name, its status, and the title of the session it is working on. Seeing what the agents were doing meant `agent list` and reading JSON, which is the wrong shape for the most common question. The title comes from the agent's reported session metadata and is absent until the agent reports one.
- The Claude integration now installs a statusline script that reports the session's title to its pane, so the sidebar and `agent status` can name what each agent is working on rather than only whether it is working. A statusline that was already configured is preserved and still renders the display: herdr's script reports the title, then feeds the same input to the previous command, and uninstalling puts the original configuration back.

## [0.8.4] - 2026-08-11

### Changed
- Pane titles no longer draw the two Nerd Font icons: the pane glyph in front of the name, and the branch glyph in front of the repo path. Both live in the Unicode Private Use Area, so a terminal running an unpatched font drew a replacement box — or nothing — in the title bar of every pane, while the layout still reserved the columns for them. The name now starts two columns earlier and truncates two columns later. The repo path and branch keep their separate colors: the renderer used to find where the git section began by searching the title for that branch glyph, and it now takes the boundary from the agent label's closing brace, which is the same place the pane name already ends.

## [0.8.3] - 2026-08-11

### Added
- Dragging a space in the sidebar now draws the same drop line an agent row does, so you can see which slot the card will land in before you let go. The slot was always being tracked — the drag knew where it would drop — it simply had nothing drawn for it, and space cards are identical in shape, so the only feedback was the reorder itself. The line is heavier than a card border on purpose: a space slot usually falls on a row a card's edge or the current space's outline already occupies, and a light line there would read as that border rather than as the marker.

### Fixed
- The end-of-list drop slot now sits below the last space's agents rather than below its card. The slot was measured from the cards alone, which put it inside the agent rows hanging under the last one — invisible until there was a line drawn on it, and off by the height of that agent list.

## [0.8.2] - 2026-08-11

### Changed
- A Herdr window that has lost focus now looks like it. Every mark that claims your typing mutes together — the focused pane's border and title, the outline around the space you are in, the band under the agent you are typing to, and both of their names — and the focused pane's cursor block disappears. A Herdr sitting on a second screen no longer presents the same "typing lands here" chrome as the window you are actually in, which is the difference dictation and blind typing were getting wrong. Herdr already knew: terminals report focus over the same channel as keystrokes, and that state was being used to hold back notifications while you were away. It was simply never drawn. Terminals that do not report focus, and multiplexers not passing focus events through, keep the normal focused chrome rather than muting on a signal that never arrives.
- The muted color is derived from your theme rather than fixed: a focus color gives up most of its chroma, which costs no legibility, and then recedes toward the panel behind it as far as that palette can afford while staying readable. Dark themes recede fully; a light theme whose accent is already close to its background recedes little or not at all and leans on losing its color instead. The labels on the focused agent's band follow the band — knocked out of a bright fill as before, sitting on top of a muted one, where a knockout would be invisible.

### Added
- Added `ui.hide_cursor_when_unfocused` to control whether the focused pane drops its cursor while the terminal window is unfocused. It defaults to `true`; set it to `false` to keep the cursor and rely on your terminal's own unfocused cursor style.

## [0.8.1] - 2026-08-10

### Fixed
- Dragging a pane by its title now swaps panes even when the pane you drag across is reporting mouse events. An agent or editor that turns on mouse tracking claimed every motion that crossed it, so the gesture was handed to that pane's own program before it could become a swap and died on its first cell of travel. A press that lands on herdr's own chrome — a pane title, a tab, a sidebar row — now owns the gesture until you release it. Terminals that track button presses but not motion no longer swallow the drag either: an encoding that comes back empty now reads as "this terminal is not tracking that event" instead of being forwarded as zero bytes.

## [0.8.0] - 2026-08-10

### Changed
- A space's card in the sidebar now tallies the agents inside it instead of its panes. The list under a card only ever showed agents, so a card reading `3 panes` above two rows was counting something you could not see, and the number said nothing about whether the card was worth opening. A space whose panes are all plain shells now reads `no agents`, which says outright that there is nothing folded away under it.
- The focused agent's band in the sidebar now fills with the same accent that outlines the space around it, its labels knocked out of the fill the way an active tab's label sits on its own. The muted surface it used before read as a different kind of mark than the outline; sharing one color lets focus read at both scales at once — the space you are in, and the agent inside it.

## [0.7.6] - 2026-08-09

### Fixed
- The sidebar now opens folded the way you left it. Collapsing the sidebar to its narrow strip, folding the spaces section shut, or folding a space's agent list away was forgotten as soon as Herdr restarted, so every session started with everything expanded again. All three folds are saved with the rest of the session now. Folds recorded for spaces that did not come back are dropped rather than left in the session file forever.

### Changed
- The agent pane you are typing into now reads as a filled band in the sidebar: the two lines under its name sit on a muted surface with their text inverted, so the eye lands on the whole row instead of on one tinted word. The tab name stays off the band and keeps the accent, heading the row the way a selected space's name heads its card, and the animating status bar down the left edge stays clear of it so it keeps reading in its own state color.

## [0.7.5] - 2026-08-08

### Fixed
- Claude Code panes no longer stay stuck on "working" after the agent has finished. Claude's end-of-turn summary reports leftover background shells (`✻ Brewed for 6m 33s · 3 shells still running`), and Herdr read that as work in progress — so a pane where the agent had started a dev server stayed "working" for as long as the server ran, which for a dev server is forever. The live footer, `esc to interrupt` and the spinner, is what marks a turn as in flight; a turn that has ended now reads idle whatever it left running behind it.
- The sidebar's spaces and agents list now stays where you scrolled it. Every render reset the scroll offset to the top, so with enough spaces and agents to overflow the window the list snapped back before you saw it move — and an animating agent repaints several times a second, so the wheel looked dead.
- Scrolling the sidebar now stops with the last entry at the bottom of the list instead of continuing until a single entry is left on screen.
- The sidebar's scroll limit and scrollbar thumb no longer wobble as you scroll. Entries are different heights, and measuring the viewport from wherever the list happened to sit changed the limit from notch to notch.

### Changed
- The sidebar's `+ new` button now pins to the bottom of the sidebar once the list is long enough to scroll, instead of trailing whichever entry landed last on screen. Lists short enough to fit still keep the button tucked under the last entry.
- The agent status bar only bounces while an agent is working, and that bounce now runs four cells past the top and bottom of the bar — resting a beat at each end — so its gradient sweeps off and back instead of snapping around on the last row. A finished-but-unlooked-at agent and a blocked one now pulse in place — the whole bar breathing between its color and a dim wash — because they are waiting on you, not going anywhere.
- The focused agent's tab name in the sidebar now reads in the same accent as the selected space's name, so the sidebar's highlights agree with each other.

## [0.7.4] - 2026-08-07

### Changed
- The status bar down the left edge of a sidebar agent row now animates while the agent is live: the lit cell walks down the three rows and back up, one row at a time, with the cells behind it fading toward the background. Working and finished-but-unseen agents bounce; settled ones hold a flat bar, so a quiet sidebar stays still.

## [0.7.3] - 2026-08-07

### Added
- The sound an agent plays when it finishes is now a choice of six: the original chime, plus a bell, an arpeggio, a ping, a blip, and a knock. Pick one in the sound section of the settings panel — moving through the list plays each sound, and the row you press `enter` on is saved to `[ui.sound] done` and used from then on. Saving also clears any stale `done_path`, so the sound you picked is the one you hear.
- Added `[ui] notify_active_tab = true` for being alerted about the agent you are already looking at. Herdr normally stays silent for the active tab of a focused terminal; with this on, a finished or blocked agent in that tab plays its sound, raises its popup, and lights up in the sidebar like a background one.

## [0.7.2] - 2026-08-06

### Added
- Right-clicking an agent row in the sidebar opens a context menu with "Rename pane", which runs the same rename dialog as the pane's own right-click menu.

### Changed
- Sidebar agent rows now lead with the tab they belong to, with the pane's own name below it. The working directory and branch that used to sit there are still on the pane's title.
- The space you are working in is now drawn as one outlined group in the sidebar: the box runs from the space card down around every agent listed under it, instead of marking only the focused agent row.
- Space cards no longer repeat the git branch. It is already on the pane, where the work happens, so the card is just the space name.
- Space cards show how many panes the space holds, right-aligned on the card in the accent color.
- Clicking a space you are not in only switches to it; its agent list keeps the fold state you left it in. Clicking the space you are already in is what folds its agents open or closed.

## [0.7.1] - 2026-08-04

### Added
- Agent rows in the sidebar can be dragged to reorder them within their space. The order is display only — tabs and the pane layout are untouched — and it survives session restore.
- A space's agent list can be collapsed in the sidebar, so spaces you are not working in fold away to a single row.
- Panes now carry a stable human name (Ada, Milo, Nadia, …) derived from their terminal id, so a sidebar full of agents is scannable instead of reading "Pane 1" over and over. The same pane keeps its name across renders and restores.
- Agent rows show the model and reasoning effort the session is actually running, read from the agent's own session log (Claude Code transcripts, Codex rollouts). This stays correct across mid-session model switches.

### Changed
- Redesigned the sidebar's agent rows around the new names, model info, and status.

## [0.7.0] - 2026-08-02

### Fixed
- The documented default pane-focus keybindings now match what Herdr actually ships. The configuration reference listed `focus_pane_left/down/up/right` as `prefix+h/j/k/l`; the real defaults are `ctrl+left/down/up/right`. No behavior changed — only the documentation and its tests were wrong.
- Background panes no longer play the "agent done" sound while the agent is still working. Dismissing a permission prompt, a single foreground-process probe race, and mid-repaint frames that showed neither the prompt box nor working chrome could each be read as completion.
- Pane output now renders flag emoji and other multi-codepoint grapheme clusters as complete symbols instead of blank cells. (#243)
- Starting Herdr with no restored workspaces, or closing the last workspace, now opens a default workspace instead of leaving the client on an empty screen where direct keybindings such as `cmd+n` were shown but ignored. (#366)
- Resizing restored panes no longer aborts the server when libghostty-vt reflows a terminal whose pre-resize cursor row is past the new height. (#465)
- Full-screen TUIs such as Neovim now receive resize-generated terminal responses after Herdr internal pane resizes, so grown panes redraw without waiting for extra input. (#471)

### Added
- Added `herdr integration install droid` for Factory Droid hooks that report session ids through Herdr's socket API. When native agent session restore is enabled, Herdr can resume Droid panes with `droid --resume <id>`.
- Panes now follow the working directory a shell reports with OSC 7. This is what makes PowerShell on WSL work: `powershell.exe` runs behind a relay stub whose `/proc` directory never moves, so new panes used to open where the pane started rather than where PowerShell is. Run `herdr shell-init powershell` for the profile snippet, and see [PowerShell on WSL](https://herdr.dev/docs/configuration/) for the setup.
- Splitting a pane that is running `powershell.exe`, `pwsh.exe`, or `cmd.exe` through WSL interop now opens the new pane in that same Windows shell instead of falling back to the configured Linux shell.

## [0.6.10] - 2026-07-07

### Fixed
- Fixed a pane freeze where a zero-length PTY write could permanently wedge the input queue, blocking all further keyboard input and terminal query responses for that pane.
- Applications that enable synchronized output (`CSI ?2026h`) but never disable it no longer freeze the pane's display: render suppression is now capped at 1 second per batch, with a fallback repaint if no further output arrives.
- A panic inside pane terminal processing no longer permanently kills the pane. Poisoned terminal locks now recover with a logged warning instead of silently dropping all subsequent output, input state, and renders.

## [0.6.9] - 2026-07-06

### Added
- Added tabs on the sidebar for spaces, making it easier to switch between workspaces.

### Changed
- Redesigned the sidebar workspace layout with more room for agents.

### Fixed
- Fixed the agent toggle in the sidebar.
- The entire pane title is now muted on unfocused panes.
- Agent label color in the pane title stays distinct when git info is shown.

## [0.6.8] - 2026-06-04

This is a hotfix release for v0.6.7, prioritizing a server-crash fix for panes that print complex Unicode or emoji output.

### Fixed
- Fixed a Herdr server crash triggered by pane output containing complex Unicode, emoji, or decomposed accent graphemes. Affected sessions could lose running pane processes or crash again after restore if the same saved pane output was replayed. (#453)
- Direct installs managed by mise now update through the mise install path instead of failing to replace the active binary.
- Claude Code panes that are actively thinking or streaming no longer flicker to blocked because of custom status text. (#409)
- Claude Code panes now detect running shell-command status more reliably.
- OpenCode installed through pnpm is now detected as `opencode` instead of being missed because the packaged executable is named `opencode.exe`. (#447)

### Added
- Added opt-in macOS input-source switching during prefix mode with `experimental.switch_ascii_input_source_in_prefix`, so users typing with a non-Latin IME can run prefix commands through an ASCII-capable input source and return to the previous input source when prefix mode ends. (#400, #434, thanks @sf-jin-ku)

## [0.6.7] - 2026-06-03

### Added
- Added a compact collapse control to the expanded sidebar so mouse users can collapse and expand the sidebar from visible controls. (#278, #291, thanks @turgaybulut)
- Added an opt-in preview update channel with `herdr channel set preview`, `[update].channel`, automated preview manifests, and GitHub prerelease publishing for users who want fixes before stable releases as Herdr transitions toward less frequent, more stable releases.
- Added a remote SSH bridge keepalive fallback. `herdr --remote` now generates a temporary SSH config that includes the user's SSH config first, then adds `ServerAliveInterval` and `ServerAliveCountMax` only when the user has not already configured keepalives. Set `[remote].manage_ssh_config = false` to disable this. (#354, #355, thanks @SunskyXH)
- Added `ui.right_click_passthrough_modifier` so a configured modifier such as `ctrl` can forward right-click hold and drag gestures to mouse-reporting pane apps while normal right-click still opens Herdr's pane menu. (#148)
- Added Kilo Code CLI automatic detection for idle, working, and blocked terminal states. (#270)
- Added `herdr integration install copilot` for GitHub Copilot CLI hooks that report prompt, tool, post-approval progress, permission, `ask_user`, `exit_plan_mode`, idle, session-exit state, and session ids through Herdr's socket API. When native agent session restore is enabled, Herdr can resume Copilot panes with `copilot --resume=<id>`. (#232, #386, thanks @LaneBirmingham)

### Changed
- Native agent session restore is now enabled by default for supported panes with current official integrations. Set `[session] resume_agents_on_restore = false` to disable it.
- Claude Code, Codex, and OpenCode integrations now report session identity only. Native state for those agents comes from Herdr's screen detection, while Pi, OMP, GitHub Copilot CLI, Hermes Agent, Qoder CLI, and custom socket integrations can still report state.

### Fixed
- Large long-running sessions no longer hit the frame-streaming crash fixed by the vendored libghostty-vt update. (#276)
- Copy mode now preserves linewise selection after `shift+v` while moving the cursor. (#360, #389, thanks @reobin)
- Leaving copy mode now restores the previous scroll position, or returns to the bottom when copy mode started at the bottom. (#398, #410, thanks @reobin)
- Git branch labels now resolve correctly in repositories that use Git's reftable ref format instead of showing `.invalid`. (#384, #423, thanks @LaneBirmingham)
- The official Nix flake now builds on macOS by providing Darwin SDK discovery helpers and Darwin cctools to the vendored libghostty-vt build. (#405, #407, thanks @DeevsDeevs)
- Commands launched after `--`, such as `herdr agent start ... -- opencode --session <id>`, now preserve child argv flags instead of parsing them as Herdr flags. (#383)
- Pane apps that request any-motion mouse tracking now receive hover/move events, making Textual-style TUI mouse interaction more reliable inside Herdr. (#419)
- Claude Code background-agent wait text in scrollback no longer keeps an idle pane marked working after the background agent has completed.
- Claude Code and Codex transcript or expanded-detail viewers no longer publish a false idle state while the pane is still showing active agent status.
- Claude Code question prompts that use the arrow-glyph selector are now detected as blocked.
- Kiro sub-agent tool approval prompts are now detected as blocked instead of working. (#388)
- Shift-letter prefix bindings such as `prefix+shift+n` now work in legacy SSH terminal sessions that send uppercase letters without separate Shift metadata. (#312)
- Idle panes now avoid repeated full foreground-process scans, reducing idle CPU on sessions with many panes. (#439)
- Restored native agent sessions now resume across background workspaces and tabs after the first client provides terminal context instead of waiting until each pane is focused.
- Pane input no longer waits behind the PTY actor's idle read poll, restoring responsive typing at quiet shell prompts. (#379)
- Pane apps that query OSC 4 ANSI palette colors now receive the active terminal palette response, so OpenCode and similar TUIs can enable system-theme behavior inside Herdr. (#387)
- Pane apps that query terminal capabilities with XTGETTCAP now receive supported capability responses, improving feature detection in Neovim and similar terminal apps. (#393)
- Pane text selection now derives its highlight colors from the host terminal or active Herdr palette instead of forcing the theme's blue accent. (#298)
- `herdr channel set preview` and `herdr channel set stable` now update direct installs from the selected channel immediately, reject preview on Homebrew and Nix installs before changing config, and show package-manager guidance for managed installs.
- Plain `herdr update` and remote binary replacement now ask before stopping running sessions, avoid protocol-heavy prompt text, and leave the current install untouched when the user chooses not to stop active pane processes. Explicit `--handoff` update flows try live handoff without a second handoff prompt.
- Remote bootstrap now uses the remote shell only for PATH discovery and runs internal probes through `/bin/sh`, so `herdr --remote` can detect existing installs when the remote login shell is fish. (#396)

## [0.6.6] - 2026-05-31

### Added
- Custom command keybindings now accept an optional `description` field to provide user-defined descriptions shown in the keybind help panel instead of the default `'custom command'` label. (#362)

### Fixed
- The OpenCode integration no longer treats `session.created` or `session.updated` plugin events as idle signals, so active sessions stay marked working until OpenCode reports `session.status` or `session.idle`. (#351)
- New interactive panes now use login-shell startup on macOS by default so Homebrew and other login PATH setup is available, with `terminal.shell_mode = "non_login"` as an opt-out. (#350)
- Claude Code panes no longer stay blocked after stale permission-prompt reports when the visible screen has returned to idle or working state. (#349)
- Codex panes no longer stay working because stale `esc to interrupt` text remains above a visible idle prompt, and visible approval-review work is now preserved as working. (#352)
- Sidebar Git status refresh now deduplicates workspaces from the same checkout and reuses cached ahead/behind results when refs have not changed, reducing idle CPU from repeated `git` polling. (#353)
- Update prompts, toasts, and docs now distinguish installing a new binary from stopping or reattaching a running Herdr session to use it.
- Large restored sessions no longer leave restored or newly split panes without shells after startup, and live handoff keeps PTY ownership bounded to one master fd per pane. (#357)
- Pane shutdown no longer warns that a pane is still alive after the direct child has already exited and been reaped. (#338)
- Closing the last pane or tab in a parent worktree workspace now shows the existing confirmation before closing the whole worktree group. (#369)

## [0.6.5] - 2026-05-29

### Added
- Added pane copy mode at `prefix+[` with keyboard navigation, visual selection, and clipboard yank support. (#231)
- Added `foreground_cwd` to pane and agent API/CLI responses so integrations can inspect the active foreground process directory without changing the existing pane/workspace `cwd` semantics. (#345)
- Added read-only `agent_session` metadata to pane and agent API/CLI responses when official integrations report native session references.

### Fixed
- Live handoff now preserves terminal state when transferring supported running panes to a replacement server.
- WSL clipboard writes now prefer OSC 52 before WSLg clipboard tools, so mouse selection and double-click copy populate Windows clipboard history in Windows Terminal. (#333)
- Incomplete host terminal OSC default-color replies no longer get misread as Alt-key input and forwarded into panes, preventing interactive prompts such as `gh auth login --web` from aborting on split `ESC ]` input. (#279, #306, #344)
- Workspace rename prompts and background notifications now use live cwd-derived workspace labels instead of stale session labels. (#332)
- `herdr session stop` no longer fails on zero-duration socket timeouts when the stop deadline is nearly exhausted.
- Update preview instructions now wrap long package-manager commands instead of truncating the shell command suffix.
- Restored native agent resume panes now fall back to a shell when the resumed agent exits instead of closing the whole pane.

## [0.6.4] - 2026-05-27

### Fixed
- Fixed macOS server startup with large restored sessions by raising the server file descriptor soft limit, preventing new panes from failing with `dup of fd N failed` or `Too many open files` around 40 live panes. (#327)

This is a hotfix for v0.6.3. See the v0.6.3 notes for the full feature release.

## [0.6.3] - 2026-05-27

### Added
- Added native agent session restore behind `[session] resume_agents_on_restore`, allowing supported Pi, Claude Code, Codex, OpenCode, and Hermes panes with current official integrations to restart into their previous agent conversation after a Herdr server restart. (#233)
- Added opt-in pane screen history across full server restarts with `[experimental] pane_history = true` and Settings > Experiments > pane screen history. (#217, #248, thanks @icedac)
- Added a session navigator at `prefix+g` with a searchable workspace/tab/pane tree, agent state filters, mouse switching, and keyboard navigation. (#157)
- Added configurable navigate-mode movement bindings for workspace and pane navigation keys. (#193)
- Added a configurable `last_pane` keybinding action for tmux-style back-and-forth navigation to the last focused pane across workspaces and tabs. It is unset by default. (#287)
- Added scrollback support to direct agent terminal attaches. Mouse wheel and plain PageUp/PageDown now scroll the attached terminal viewport, while terminal apps that request mouse or alternate-scroll input still receive those events. The client/server protocol is now version 11.
- Added `ui.redraw_on_focus_gained` to keep the existing full redraw on outer-terminal focus gain by default while allowing users to opt out of the visible refresh. (#282)
- Added `ui.mobile_width_threshold` to configure the terminal width at which Herdr switches to the mobile single-column layout. (#317)
- Added `--handoff` for `herdr update` and `herdr --remote` to opt into live server handoff for supported running servers. Plain update and remote attach use the normal restart/stop flow by default.
- Added `pane.report_metadata` and `herdr pane report-metadata` so user hooks can customize pane titles, displayed agent names, compact status labels, and visible state labels without taking over integration-owned lifecycle or session state. (#36)
- Added tmux-style double-click token copy in panes, with temporary copy feedback and mouse passthrough preserved for terminal apps that request mouse input. (#142, #296, thanks @babymastodon)
- Added Ctrl-click URL opening inside panes for OSC 8 hyperlinks and visible `http://` or `https://` URLs when the host terminal sends the modified click to Herdr. (#307)
- Added Qoder CLI detection, terminal state heuristics, and `herdr integration install qodercli` hook support. (#308, #309, thanks @wayneleelwc)

### Fixed
- Remote bootstrap now downloads exact-version release assets for Homebrew and Nix clients instead of copying package-manager-managed local binaries into `~/.local/bin/herdr`.
- `website/latest.json` now stores asset URLs for archived releases under `releases[version].assets`, so remote bootstrap can fetch the current client version even when Homebrew and the top-level latest release are temporarily out of sync.
- App and server event queues no longer stall under load, improving delivery of pane and agent state updates. (#265)
- Agent status subscriptions now deliver already-matching states and event-hub notifications reliably for waits and automation. (#288, #295)
- Codex background terminal waits are detected more reliably, and idle agent checking uses less CPU. (#300)
- Split OSC 10/11 host color replies are buffered correctly, so terminal apps still receive host foreground/background color responses when replies arrive in chunks. (#306, #310)
- `herdr session stop` is more reliable when the server closes the socket early or stops without sending a full response.
- The OpenCode integration now releases pane ownership on plugin dispose, preventing stale integration state after OpenCode exits. (#314)
- Linux sound alerts no longer fall back to `aplay` for mp3 files, preventing static noise on systems without `paplay`. Herdr now tries mp3-capable players such as `pw-play`, `ffplay`, `mpg123`, and `mpv` instead. (#290)

## [0.6.2] - 2026-05-23

### Added
- Added optional Nix flake support for building, running, installing, and developing Herdr with Nix. (#208, #221, #264)
- Added `terminal.new_cwd` to choose whether new panes, tabs, and workspaces follow the source pane/workspace, start in `$HOME`, use Herdr's process directory, or use a fixed path.
- Added `herdr integration install omp` for OMP's `.omp` extension directory. The extension reports OMP pane state through Herdr's socket API without relying on native `omp` process detection.
- Added CLI and socket API support for Git worktrees with `herdr worktree list/create/open/remove`, optional worktree provenance on workspace responses, and client/server protocol version 10.

### Fixed
- GitHub Copilot CLI sessions now use tested terminal heuristics for approval prompts, freeform input, plan review, and thinking states in the Agents panel. (#232, #256, thanks @LaneBirmingham)
- Kiro approval prompts are now detected as blocked in the Agents panel. (#255)
- Workspace labels now follow the live pane working directory after directory changes.
- Remote clients using local keybindings no longer show stale server keybinding warnings from the remote host.

## [0.6.1] - 2026-05-22

### Added
- Added `ui.mouse_scroll_lines` to configure how many pane scrollback lines each mouse wheel notch scrolls. The default remains 3. (#236)
- Added `--remote-keybindings local|server` for `herdr --remote`. Remote attach now uses the launching client's local keybindings by default without copying config files to the remote host; use `--remote-keybindings server` to keep the remote server's keybindings. The client/server protocol is now version 9.
- Added `experimental.reveal_hidden_cursor_for_cjk_ime = false` (opt-in), `experimental.cjk_ime_agents = []` (optional allow-list), and `experimental.cjk_ime_cursor_shape = "steady_block"` to expose the focused pane's cursor anchor to the outer terminal even when the pane requested `?25l`, restoring macOS IME candidate-window tracking for TUIs that paint their own cursor (Claude Code, pi, codex). When `cjk_ime_agents` is non-empty, the reveal applies only to focused panes whose detected agent matches one of the listed names. When the pane reports no cursor position, the anchor falls back to the pane's top-left so a stable IME hint is always available. Trade-off when enabled: an extra hardware cursor may appear in the outer terminal for apps that hide the cursor without painting a replacement. (#149, thanks @ChihGodlee)
- Added explicit sidebar Git worktree groups plus native worktree creation, existing checkout open, and safe checkout cleanup flows, configured by `[worktrees].directory`, `keys.new_worktree`, optional `keys.open_worktree`, and optional `keys.remove_worktree`. (#137)
- Added named-session reattach and stop command hints so detach and update guidance point back to the active session. (#199, thanks @Golden-Pigeon)

### Fixed
- Pane apps that query OSC 10/11 default foreground/background colors now receive the host terminal colors, so OpenCode and similar TUIs can detect light terminal themes inside Herdr. (#253)
- Codex Plan mode question prompts now override stale integration `working` reports when the visible terminal UI is clearly waiting for an answer, stale hook authority is cleared when foreground process detection sees Codex exit back to the shell, and Claude Code cancellations now recover from stale hook `working` reports when the idle prompt returns. (#249)
- Keybinding parsing now accepts non-ASCII printable keys such as `ö`, `é`, and `ğ`, including UTF-8 Alt chords. (#247)
- Kimi Code CLI sessions now use structural terminal detection for approval prompts and live thinking/tool status, improving working and blocked state reporting in the Agents panel. (#215)
- Antigravity CLI (`agy`) sessions are now detected, and their terminal UI now reports working and blocked states in the Agents panel. (#207)
- Cursor Agent sessions launched as `cursor-agent` or symlink aliases such as `agent` are now detected, and their terminal UI now reports working and blocked states in the Agents panel. (#225)
- Agent detection now ignores runtime argument strings when identifying foreground processes, reducing false positives from helper commands and wrapped processes. (#238)
- In-app notifications now stay below interactive floating overlays, so dialogs and menus remain readable and clickable while a toast is visible. (#228)
- `herdr --remote` now offers to restart the remote server after installing or replacing a remote binary, or when the running server version differs, even if the client/server protocol is still compatible.

## [0.6.0] - 2026-05-20

### Added
- Added keybinding v2 with explicit `prefix+...` syntax, array bindings per action, configurable prefix-mode pane focus, tab switching, and direct modified chords for users who opt in. (#154, #201, #202, #219)
- Added `herdr config reset-keys` to back up `config.toml` and remove custom keybindings so built-in v2 defaults apply on restart or config reload. (#154)
- Added an integrations tab in settings and first-run onboarding so users can install recommended agent integrations from inside Herdr.
- Added update badges on the sidebar menu, settings menu item, and integrations settings tab when installed integrations are outdated.
- Added `terminal.default_shell` to choose the executable used for new interactive panes. When unset, Herdr still falls back to `$SHELL`, then `/bin/sh`. (#196)
- Added native Kiro CLI detection with idle and working state heuristics. (#185)

### Fixed
- Keybinding conflict warnings now stay visible and show one readable yellow row per conflicting binding.
- Update prompts that need to stop a running server now default Enter to yes and show `[Y/n]`.
- Pending release notes no longer open automatically on startup; the latest notes remain available from the menu.
- Running `herdr server` directly now prints socket and log paths and explains that normal TUI users should run `herdr`.
- Kitty graphics virtual Unicode placeholders now render image placements instead of leaving placeholder cells behind. (#136)
- Clipboard image reads are now capped to Herdr's image payload limit, preventing oversized local clipboard images from being read into memory.
- The install script now reads Herdr's public latest-release manifest, so fresh installs use the same binary URLs as `herdr update`.
- The Claude Code integration no longer lets subagent completion hooks report durable `working`, preventing delayed recap or subagent completion events from reviving an idle pane. (#198)
- Remote clients now bridge local clipboard images into the remote pane by staging them as temporary image files and pasting the remote path, so Claude Code image paste works over `herdr --remote`. (#205)

### Breaking Changes
- Removed the separate `keys.quit` binding. Use `keys.detach`, which detaches in server mode and exits in `--no-session` mode. The default detach binding is now `prefix+q`.
- Keybindings now use explicit trigger syntax: `prefix+c` means prefix mode, while `ctrl+alt+c` is direct. Bare printable direct bindings such as `new_tab = "c"` are rejected with diagnostics because they intercept normal typing. The default keymap now gives tmux-style tab actions to `prefix+c`, `prefix+n`/`prefix+p`, and `prefix+1..9`, uses `prefix+w` for workspace navigation, and moves pane focus to `prefix+h/j/k/l`. (#154)
- The client/server protocol is now version 8. Stop and restart any running v0.5.12 server before attaching with this release.

## [0.5.12] - 2026-05-19

### Fixed
- The Claude Code integration no longer reports successful or failed post-tool hooks as `working`, and installing the updated integration removes Herdr's deprecated post-tool hook entries from existing Claude settings. (#198)
- The Codex integration now reports native `PermissionRequest` hooks as `blocked`, so permission prompts no longer stay pinned as `working` after a tool-use hook. (#198)
- Workspace and tab rename prompts now handle Backspace, Ctrl+Backspace, Alt+Backspace, Cmd+Backspace, Ctrl+H, Ctrl+W, and Ctrl+U as editing shortcuts instead of inserting stray characters or clearing unexpectedly. (#204)

## [0.5.11] - 2026-05-19

### Added
- Added the `terminal` built-in theme, which uses the host terminal's ANSI palette for Herdr UI colors. (#140, #146, thanks @babymastodon)
- Added Hermes Agent foreground-process detection with basic idle, working, and blocked heuristics. (#144)
- Added a Hermes Agent plugin integration for direct state reporting. (#144)
- Added `ui.sidebar_min_width` and `ui.sidebar_max_width` to configure the sidebar's expanded resize bounds. Defaults remain 18 and 36 columns; existing configs are unchanged. (#132, #135, thanks @ChihGodlee)

### Fixed
- Running the internal `herdr client` command from inside Herdr now respects the nested-launch guard, and the command is no longer advertised in root help. (#187)
- The Herdr agent skill now refuses to claim pane ownership unless it is running inside Herdr. (#152)
- Terminal-style docs code blocks now keep their copy button in the top-right corner. (#190)
- The sidebar `new` workspace button now aligns with the sidebar's left padding. (#189)
- Herdr now preserves `session.json` symlinks when saving persistent session state. (#139, #147, thanks @cloudmanic)
- Alt+Backspace is now preserved when forwarded into panes. (#155, #165)
- Directional pane focus now works while a tab is zoomed. (#151, #167)
- Agent detection now prefers the foreground process group leader, reducing false matches from child helper processes. (#161, #172)
- Remote attach now uses a matching `herdr` already available on the remote `PATH` before installing a new copy. (#170)
- Modified Enter input such as Shift+Enter is now preserved in supported terminals. (#168)
- Sidebar agent entries now show user-assigned agent names when available. (#145)

### Breaking Changes
- The client/server protocol is now version 7. Stop and restart any running v0.5.10 server before attaching with this release.

## [0.5.10] - 2026-05-17

### Added
- Added indexed keybind families under `[keys.indexed]` for jumping directly to workspace, tab, or visible agent positions 1-9.
- Added hook-owned custom agent status labels, so integrations can show short visual states like `indexing` without changing semantic agent status.
- Added terminal-backed agent commands and socket API methods for listing, reading, sending to, renaming, focusing, waiting on, attaching to, and starting agent terminals.
- Added direct terminal attach with `herdr agent attach <target>` and `herdr terminal attach <terminal_id>`.
- Added `ui.prompt_new_tab_name = false` for creating new tabs immediately with generated names instead of opening the rename dialog. (#123)
- Added optional `keys.edit_scrollback` to open the focused pane's retained scrollback in `$EDITOR` inside a temporary zoomed pane. (#122)

### Changed
- Renamed the focused pane fullscreen keybinding to `keys.zoom`; `keys.fullscreen` remains supported as a legacy alias.

### Fixed
- Grok Build is now detected as `grok`, with basic working, blocked, and idle state detection. Conflicting known-agent hook labels are ignored once native foreground-process detection identifies a different known agent. (#133)
- Terminal cursor shapes now forward through attached clients. (#116)
- Herdr now redraws immediately when the outer terminal regains focus.
- GitHub Copilot is now correctly detected when its process name is `copilot`. (#118)
- Integration installs now respect `PI_CODING_AGENT_DIR`, `CLAUDE_CONFIG_DIR`, and `CODEX_HOME` when choosing Pi, Claude Code, and Codex config paths. (#121)
- Split pane resize hit areas no longer overlap the first content column or row, making text selection work from the start of right and bottom panes. (#120)
- Dragging text selections near pane edges now autoscrolls into scrollback, and selection state now clears correctly when switching workspaces, tabs, or panes. (#128, #129, thanks @leeeanh)
- Zoomed panes now keep their border visible in tabs that contain multiple panes. (#115)

## [0.5.9] - 2026-05-15

### Added
- Added experimental Kitty graphics rendering for local panes and attached clients behind `experimental.kitty_graphics`, including support for larger graphics frames.
- Added `ui.toast.delivery = "system"` for OS-level background notifications, using `notify-send` on Linux and `terminal-notifier` or `osascript` on macOS.
- Added light variants for Catppuccin, Tokyo Night, Gruvbox, One, Solarized, Kanagawa, and Rosé Pine themes.
- Added `ui.mouse_capture = false` for tmux-style mouse behavior, letting the terminal handle normal clicks while still forwarding mouse input to pane apps that request it.

### Changed
- Moved experimental settings into `[experimental]`.

### Fixed
- PageUp and PageDown now scroll Herdr pane scrollback for normal panes while still forwarding keys to full-screen or mouse-reporting apps.
- Enhanced tilde key sequences now parse correctly, improving compatibility with terminals that emit them.
- `herdr integration install codex` now enables the current Codex `[features] hooks = true` flag and migrates the deprecated top-level `codex_hooks` flag.

### Breaking Changes
- `advanced.allow_nested` has moved to `experimental.allow_nested`; update configs that allow nested Herdr launches.
- The client/server protocol is now version 5. Stop and restart any running v0.5.8 server before attaching with this release.

## [0.5.8] - 2026-05-12

### Added
- Added manual pane labels through `herdr pane rename`, the `pane.rename` socket API, an optional `keys.rename_pane` binding, and the right-click pane menu.
- Added `ui.show_agent_labels_on_pane_borders`, which can show detected or reported agent names in split pane borders when no manual pane label is set.
- Added `herdr integration status [--outdated-only]` so installed agent integrations can be checked for legacy or outdated versions.
- Added an optional `keys.open_notification_target` binding for jumping to the pane behind the current notification.
- Added optional `keys.previous_agent` and `keys.next_agent` bindings for cycling through sidebar agent entries.

### Changed
- Scrolling over the tab bar now switches tabs directly, including overflowing tab bars.

### Fixed
- Indexed terminal palette colors now render correctly for 256-color terminal apps.
- Hook-based agent integrations now reject stale out-of-order reports and base notifications on effective agent state, reducing duplicate or stuck state changes.
- Background tabs now resize when the outer terminal size changes, preventing stale pane dimensions when switching back to them.
- Client shutdown now drains queued control messages more reliably.
- Pane cursors are now hidden while scrolled back, and omitted while the mobile switcher is open.
- Mobile agent switcher entries now include tab context, making agents easier to identify on narrow terminals.
- macOS foreground job detection now uses process groups, improving agent state tracking for foreground commands.
- Remote SSH no longer fails before connecting when macOS temporary bridge socket paths exceed Unix socket length limits. (#103, thanks @moonsphere)
- Nix-wrapped agent commands are now detected by their underlying agent entrypoint.
- Pane renames made through the socket API now rerender immediately.

## [0.5.7] - 2026-05-10

### Added
- Added ANSI-formatted pane reads to the CLI and socket API with `herdr pane read --format ansi` / `--ansi`, preserving colors and styles for visible and recent pane output.

### Changed
- The agents panel now highlights the currently focused agent entry, matching the active workspace styling. (#84, thanks @soomtong)

### Fixed
- Git branch and ahead/behind refreshes now run off the main loop, preventing slow Git status checks from freezing the UI.
- Update and startup flows now detect incompatible running servers earlier and give clear stop/restart guidance instead of trying to attach with a mismatched client/server protocol.
- `herdr update` now downloads and prepares the new binary before stopping a running server, reducing the chance of interrupting an active session when download or install preparation fails.

## [0.5.6] - 2026-05-09

### Added
- Added the `vesper` built-in theme. (#71, thanks @nexxeln)
- Added `herdr --remote <ssh-target>`, so you can use Herdr as a thin client for remote servers without SSHing in first. Herdr connects over SSH, bootstraps a matching remote `herdr` binary when needed, starts the remote server automatically, and streams an efficient terminal view back to your local terminal.

### Changed
- Updated the bundled `libghostty-vt` engine and removed the custom Linux C++ runtime link workaround from static builds.
- CLI workspace, tab, and pane creation now preserve the current focus by default; pass `--focus` to switch to the newly created item.

### Fixed
- OSC 8 hyperlinks emitted inside panes now remain clickable after Herdr renders them, including titled markdown-style links.
- Agent panel scope now defaults to `all` and is saved to config when changed, so choosing `current` or `all` survives session resets and upgrades.
- Native agent hook state now clears when the detected native agent exits, preventing stale hook-reported status from sticking to a pane.
- Clicking an in-app agent toast now jumps to the relevant pane and clears the toast after focus.

## [0.5.5] - 2026-05-06

### Added
- Added a mobile layout for narrow terminals, making it practical to SSH into your machine and run herdr from your phone.

### Fixed
- Non-ASCII terminal input is no longer dropped when UTF-8 characters arrive split across multiple reads.
- Native agent detection now clears agents after their foreground process exits and control returns to the shell, preventing stale agent status in the sidebar.
- Pane contents no longer shift horizontally when scrollback appears, keeping the scrollbar gutter stable.

## [0.5.4] - 2026-05-03

### Fixed
- Visible active-tab panes that finish while the outer terminal is unfocused are now marked as seen when you return to herdr, preventing stale done/attention indicators.
- IME candidate windows and mobile SSH cursor tracking now stay anchored to the focused pane during client redraws, including apps that hide the cursor, instead of drifting to sidebar or repaint positions.

## [0.5.3] - 2026-04-30

### Added
- Added named persistent sessions, so you can keep separate herdr environments for different projects or contexts while sharing the same global config. See the docs for the full session CLI. (#57, thanks @fbettag)
- Added `herdr status`, `herdr status server`, and `herdr status client` to inspect the local client, running server, protocol compatibility, socket path, and whether a restart is needed.

### Changed
- Focused panes can now still alert you through terminal notifications when the herdr terminal window is unfocused, so active work does not go quiet just because you switched to another app.

### Fixed
- Dragging pane split borders now works when the app inside the pane has mouse reporting enabled, including Claude Code no-flicker mode. (#61, thanks @EYH0602)
- Pressing the prefix key twice now forwards a literal prefix key into the focused pane in client mode again.
- `herdr integration install` and `herdr integration uninstall` now work without requiring a running herdr server.
- Pane PTYs now keep their last attached size while detached, preventing detached output from being resized or rewrapped to fallback dimensions.

## [0.5.2] - 2026-04-27

### Added
- Config can now be reloaded in the running app/server from the global menu or with `herdr server reload-config`, applying safe live settings without restarting the persistent server.

### Fixed
- Persistent server startup now surfaces config diagnostics in attached clients instead of silently hiding parse or validation errors.
- Pane backgrounds now stay transparent when the host terminal background color is unknown, while explicit terminal cell backgrounds still render correctly.
- Persistent-session toast and sound notifications now target the foreground attached client instead of firing across every connected client.
- Claude Code subagent hook events no longer make the parent Claude pane look idle or released when a subagent finishes, and permissioned tool-call completion keeps the pane in the correct working state.

## [0.5.1] - 2026-04-25

### Added
- Toast notifications can now be delivered through the outer terminal as desktop notifications. Configure this with `ui.toast.delivery = "terminal"`; see the [configuration docs](https://herdr.dev/docs/configuration/) for details.
- Herdr now writes separate capped support logs for app, client, and server modes, making persistent-session issue reports easier to diagnose without unbounded log growth.
- The bundled opencode plugin now reports question prompts as blocked while waiting for user input, then returns to working or idle when answered or dismissed. Question prompts are also detected by the default terminal-screen heuristics. (#51, thanks @mspiegel31)

### Changed
- Routine API request traces now log at debug level by default, making normal support logs smaller and easier to read while preserving detailed traces when debug logging is enabled.

### Fixed
- Pasted text and other reverse-video terminal content now stays readable when pane backgrounds are transparent. (#45, thanks @EYH0602)
- Panes now advertise a stable `TERM=xterm-256color` and `COLORTERM=truecolor` by default, improving redraw and cursor behavior in shells and remote sessions.
- Pane scrollbars once again reserve their own rightmost column instead of overlaying terminal content in persistent session mode.
- Terminal-delivered toast notifications now use the server-approved delivery decision in persistent session mode, so attaching clients do not incorrectly suppress them.
- In-app toast delivery now stays inside herdr instead of also forwarding a terminal/desktop notification.

## [0.5.0] - 2026-04-21

### Breaking Changes Please Read
- herdr now defaults to a persistent server/client session model. running `herdr` starts or reattaches to a background session server instead of launching the old single-process UI.
- quitting the UI in default mode now detaches the current client and leaves the shared session running. use `herdr server stop` to stop the background server explicitly.
- the old monolithic behavior is still available as an escape hatch with `herdr --no-session`.

### Added
- Persistent sessions are now the default product behavior. You can detach and reattach without stopping pane processes.
- Added the thin client and headless server as first-class product components, including auto-detect launch, explicit `herdr client`, and `herdr server stop`.
- Sessions now restore cleanly after full restart, preserving workspaces, tabs, panes, and running process state.
- Multi-client attach is now supported. Multiple clients can connect to the same shared session.

### Changed
- In persistence mode, in-app quit actions now detach the current client by default instead of shutting down the whole background server.
- The current persistence model is a shared session view across attached clients. It is not yet full tmux-style per-client independent navigation.
- Restored sessions now land in terminal mode, while fresh sessions still start in navigate mode.

## [0.4.11] - 2026-04-16

### Breaking Changes Please Read
- The update flow changes in `0.4.11`. Herdr no longer installs updates silently in the background. Starting with this release, herdr only checks for updates and shows them in the UI. To install a new release, quit herdr and then run `herdr update` manually in your shell.
- This prepares the upcoming `0.5.0` persistence release. Herdr is moving from the old single-binary update model toward a persistent server/client session model, so your workspace can keep running while clients attach, detach, and reconnect.
- The reason for this change is upgrade safety. Herdr needs to stop the old running process cleanly before the new client/server model takes over, so manual update avoids mixed-version states during the transition.

### Added
- Hook-reported agent state can now use custom agent labels, so integrations are no longer limited to herdr’s built-in agent names. Custom labels now flow through pane/workspace UI and the socket API anywhere agent names are shown.

## [0.4.10] - 2026-04-14

### Added
- Prefix mode now supports custom command keybindings via `[[keys.command]]`, so you can launch detached shell helpers or open temporary overlay panes from inside herdr using the active workspace, tab, pane, and cwd context.
- Pressing the prefix key twice now forwards a literal prefix keystroke into the focused pane, which makes nested tools and terminal apps that use the same prefix easier to control.

### Fixed
- App-level key handling now normalizes enhanced keyboard reporting consistently, so shifted bindings and text like `?` and uppercase characters work correctly in navigate mode and text-entry UI.
- Ctrl+letter input is now encoded correctly when pane apps enable kitty keyboard mode, improving compatibility with terminal programs that expect CSI-u style key reporting.
- The collapsed sidebar now keeps the active workspace visibly highlighted even while you stay in terminal mode.
- Droid Mission Control screens are now treated as idle instead of active work, reducing false busy-state detection.

## [0.4.9] - 2026-04-13

### Fixed
- Droid's primary-screen redraws no longer erase pane scrollback inside herdr, while normal scrollback-clear behavior is preserved elsewhere.
- `q` is now dedicated to quitting in navigate mode instead of also acting as a generic cancel key in modals and overlays, reducing accidental quits.
- Tab bar scrolling is tighter: the scroll-right button and new-tab button now sit directly adjacent to the last visible tab without a gap, and manual scroll no longer overscrolls past the last tab.

## [0.4.8] - 2026-04-12

### Added
- Themes can now set `panel_bg = "reset"` to let herdr’s panel chrome inherit the host terminal background instead of painting an opaque panel fill. This also accepts the aliases `default`, `none`, and `transparent`.
- Ghostty-backed panes now preserve the host terminal’s default background when it matches the outer terminal theme, so terminal window transparency can show through pane content instead of being repainted as an opaque color.

### Fixed
- Clipboard writes now prefer native platform clipboard tools (`pbcopy`, `wl-copy`, `xclip`, or `xsel`) before falling back to OSC 52, which makes copy operations from panes more reliable across terminal setups.

## [0.4.7] - 2026-04-10

### Added
- The tab bar now handles large tab sets better: you can scroll overflowing tabs with the mouse controls or wheel, and reorder tabs by dragging them.
- `workspace create` and `tab create` now return the created root pane in their JSON response, so automation can act on the new pane immediately without an extra lookup.

### Fixed
- Background panes that start idle no longer show up as `done` or trigger finished-state attention until they have actually transitioned from working or blocked to idle.
- Left-click now focuses panes and right-click now opens the pane context menu even when the inner TUI has mouse reporting enabled, fixing apps like Claude Code. (#25, thanks @othavioquiliao)
- OSC 52 clipboard writes from apps running inside panes now reach the host clipboard correctly, including copy requests emitted by child processes inside the pane.
- `pane close` now removes only the targeted tab when other tabs still exist in the workspace, instead of closing the whole workspace.
- Amp approval prompts are now detected more reliably as blocked, including tool-call, command, and file edit/create approval screens.

### Breaking Changes
- Socket API clients that match `result.type` exactly need to handle `workspace_created` and `tab_created` for `workspace.create` and `tab.create`; these calls no longer return `workspace_info` and `tab_info`.

## [0.4.6] - 2026-04-09

### Fixed
- Agent state detection is now more reliable when panes are scrolled back, when Codex is running in narrow panes, and when Claude opens slash-command or settings menus, reducing false blocked or idle states.
- Mouse-driven terminal text selection now autoscrolls into pane scrollback and clears cleanly after copy, so selecting beyond the visible viewport works as expected.
- Pane terminal colors now return to the outer terminal theme after fullscreen TUIs exit, fixing cases like Droid leaving stale background colors behind. This restore path now also works correctly on macOS.

## [0.4.5] - 2026-04-09

### Added
- `herdr workspace create` and `herdr tab create` now support `--label`, so scripts and agents can name new workspaces and tabs immediately instead of creating them first and renaming them afterward.
- The global menu now includes a manual **reload keybinds** action, so you can apply `config.toml` keybinding changes without restarting herdr.
- The socket API and CLI now expose a `done` agent status, including `herdr wait agent-status --status done`, so automation can distinguish finished agent runs from panes that are merely idle.

### Changed
- Session state is now saved automatically with a debounce while you work, so recent workspace, tab, pane, and sidebar changes are preserved more reliably even if herdr exits unexpectedly.

### Fixed
- Only the focused pane now owns the terminal cursor, which removes stray cursor blocks from unfocused panes.
- In-app **What's New** / release notes now render inline code spans and fenced code blocks correctly.
- Default numbered tabs now stay auto-named when you keep or rename them back to their numeric label, so generated tab numbering stays compact and predictable.

## [0.4.4] - 2026-04-08

### Changed
- The expanded sidebar can now be split into resizable workspace and agent sections with a draggable divider, and that section sizing is preserved across restarts.

### Fixed
- IME input now works properly for Chinese and other UTF-8 input methods in pane terminals, so candidate selection no longer falls back to typing raw digit keys. (#9, thanks @Edmund-a7)
- `herdr pane run ...` now uses the bracketed-paste-aware input path, improving compatibility with shells and terminal apps that expect pasted command text to arrive atomically.
- The local socket API is more robust and secure: its Unix socket is now restricted to the current user, and long-running output waits and subscriptions stop cleanly on disconnect or shutdown instead of hanging indefinitely.

## [0.4.3] - 2026-04-07

### Fixed
- Update checks and in-app **What's New** release notes no longer depend on GitHub’s release API, which avoids the transient 403 failures from the previous update path.
- `herdr pane run ...` now submits the full command atomically in one request, fixing cases where scripted commands did not reliably execute because the final Enter was sent separately.
- Bare line-feed input is now preserved in raw terminal input instead of being normalized to Enter, fixing Linux terminal cases where inputs like Shift+Enter or Ctrl+J could be interpreted incorrectly.

## [0.4.2] - 2026-04-07

### Added
- The expanded sidebar agent panel can now switch between the current workspace and all workspaces, so you can scan and jump to agents across the whole session.
- The collapsed sidebar now shows compact per-pane agent indicators, so you can keep an eye on agent activity without reopening the full sidebar.

### Changed
- The sidebar now handles larger workspace sets more cleanly: the workspace section has headers, its own scrolling, better-aligned drag/drop slots, and manual width changes persist across restarts. Double-clicking the divider resets it to the configured default width.
- Pane scrollback is now configured with `advanced.scrollback_limit_bytes`, matching Ghostty's byte-based scrollback limit. Set it to `0` to disable pane scrollback entirely. The old `advanced.scrollback_lines` key is still accepted as an alias, but it now uses the same byte-based value.
- Linux release binaries now ship with libghostty SIMD enabled again without reintroducing the musl startup issue, restoring the optimized Linux build path.

### Fixed
- Typing in pane terminals on macOS is responsive again after the Ghostty migration, by keeping a persistent per-pane Ghostty key encoder instead of rebuilding it on every keypress.
- The collapsed sidebar expand toggle works again.
- Creating a new tab now waits until you confirm the dialog, so cancelling the new-tab flow no longer leaves behind an unwanted tab.
- Copying selected pane text now uses Ghostty's native selection extraction, which preserves wrapped text and wide characters more accurately.
- Session restore is more tolerant of older and current snapshot formats, including pre-tab session files.

## [0.4.1] - 2026-04-06

### Fixed
- Fixed Linux release binaries crashing on startup.

## [0.4.0] - 2026-04-05

### Major Changes
- Herdr now uses a Ghostty-backed terminal engine as its pane runtime.
- The legacy vt100 pane backend has been removed, making Ghostty the single terminal backend going forward.

### UX and Interaction
- Workspaces can now be reordered by dragging them in the sidebar.
- Notification sounds now support custom mp3 file overrides, with either one shared file or separate files for finished vs needs-attention alerts.

### API and Integration
- Workspace API ids are now stable, making socket and CLI automation more predictable across workspace changes and restores.

### Packaging and Runtime
- macOS builds now statically link the vendored `libghostty-vt`, preserving the single-binary install and update flow.

## [0.3.2] - 2026-04-03

### Changed
- The global launcher now surfaces update-related actions more clearly: when release notes are available you can open **What's New**, and when an update has been downloaded you can **quit to apply update** directly from the menu.
- Release notes are now retained as the latest available notes after you dismiss the startup modal, so you can reopen them later from the UI instead of only seeing them once.

### Fixed
- Fixed held-key repeat in terminal panes on macOS terminals that send explicit repeat events through the enhanced keyboard protocol, restoring continuous backspace, character, and arrow-key repeat without letting modal close/confirm key repeats leak into the shell.

## [0.3.1] - 2026-04-03

### Added
- New tabs now open directly into the rename flow, with the default tab name prefilled and replaced on first type so you can name tabs as you create them.

### Changed
- Polished modal layout and spacing across onboarding, settings, keybind help, and release notes so overlays feel more consistent and their content/actions line up more cleanly.
- Debug builds now use separate runtime/config paths from normal releases, which avoids local development sessions colliding with your main herdr install.

### Fixed
- Starting a second herdr instance against an active socket now fails fast with a clear error instead of clobbering the running session.
- Fixed pane and agent state updates being dropped under internal event queue pressure, which could leave a pane showing stale status after work finished.
- Fixed onboarding modal sizing and click targets, and corrected release-notes scroll calculations when a scrollbar is present.

## [0.3.0] - 2026-04-03

### Major Changes
- Added tabs within workspaces, so a single workspace can now hold multiple terminal tab contexts with their own pane layouts.
- Added first-class tab support to the local socket API and CLI wrappers, including `herdr tab ...` commands and tab ids like `1:2` alongside workspace-scoped pane ids.
- Added built-in direct integrations for pi, claude code, codex, and opencode, plus authoritative hook-driven state reporting so supported agents can report semantic state directly instead of relying only on screen heuristics.
- Added a post-update release-notes screen so herdr can explain what changed after an update is installed.

### UX and Controls
- Added optional direct pane-focus keybindings for terminal mode, so you can switch panes with modifier shortcuts like `alt+h` or `alt+right` without entering navigate mode first.
- Reworked keybind discoverability so the in-app keybind help now shows all supported actions, including optional bindings that are currently unset.
- Keybind help now uses a centered scrollable modal with mouse and keyboard scrolling, matching the release-notes interaction model more closely.
- Popups and action-button interactions now use more consistent modal geometry and button semantics across the UI.
- Polished the sidebar agent section so it focuses on detected agents only and uses clearer two-line agent cards with more breathing room.

### Behavior Fixes
- Hook-driven agent state updates now stay correct in tabbed workspaces.
- Modifier-only keypresses no longer leak into panes as stray input.
- Multi-tab agent labels now include tab names when that extra context matters.
- Workspace identity now follows the first tab's root pane again instead of stale creation-time cwd.
- Background notification suppression is now tab-aware rather than workspace-wide, so background tabs in the current workspace can still alert correctly.

### Documentation
- Updated the README, configuration guide, integrations guide, skill, and socket API docs to reflect tabs, direct integrations, unset optional keybindings, direct terminal-mode navigation examples, workspace-scoped pane ids, and the current workspace identity/sidebar model.

## [0.2.4] - 2026-04-01

### Fixed
- Fixed a macOS-only startup misdetection where pi could briefly appear as codex in the sidebar because process environment entries were being parsed as command-line arguments.

## [0.2.3] - 2026-03-31

### Changed
- Mouse wheel handling now follows the tmux/Ghostty model more closely: fullscreen apps receive wheel input when they own scrolling, while herdr keeps host scrollback for panes that are behaving like a normal terminal transcript.
- Pane scrollbars now only appear when herdr has real host scrollback for that pane, instead of implying a host-managed scroll position for app-owned scrolling.

### Fixed
- Fixed Codex and pi panes becoming unscrollable in herdr by preserving recoverable host history for top-anchored normal-screen output, without relying on alternate-screen scrollback retention.
- Fixed pane wheel routing so apps using mouse reporting or alternate-scroll behavior can receive scroll input directly instead of having herdr always intercept it.

## [0.2.2] - 2026-03-31

### Fixed
- Fixed pane scrollbars so they reserve their own lane instead of drawing over terminal content, which makes scrolling and scrollbar dragging behave more cleanly in narrow panes.
- Fixed alternate-screen scrollback handling so full-screen terminal apps can preserve recoverable history inside herdr panes instead of losing rows that scroll off.
- Fixed Codex in herdr panes losing transcript/history while running in alternate screen, so past output remains scrollable instead of disappearing as the session grows.
- Hid the rendered terminal cursor while a pane is scrolled back, avoiding stray cursor blocks appearing in the wrong place during history navigation.

## [0.2.1] - 2026-03-31

### Added
- Herdr now checks for updates at startup and periodically while it stays open, so long-running sessions can still discover new releases without a restart cycle.
- Added a lightweight bottom-right toast when an update has been downloaded and is ready, with a simple restart-to-use-it flow.

### Changed
- Rendering is now driven more directly by app events instead of relying as much on polling, which makes the UI feel snappier and cuts unnecessary redraw work.

### Fixed
- Restored smooth fast spinner animation for working agents.
- Closing a pane or workspace now reliably terminates the processes running inside that pane session instead of leaving shells or child processes behind.
- Fixed bracketed paste handling so incomplete paste sequences are preserved across read timeouts instead of being dropped or misread.

## [0.2.0] - 2026-03-30

### Added
- Added a local Unix socket API for controlling running herdr sessions, including workspace and pane management, pane reads, text/key input, pane splitting, and output waits.
- Added event subscriptions over the socket API for workspace and pane lifecycle events, pane output matches, and agent state changes.
- Added CLI wrappers on top of the socket API with `herdr workspace ...`, `herdr pane ...`, and `herdr wait ...`, using compact public ids like `1` and `1-2` for scripting and agent orchestration.
- Added a settings popup with mouse support for changing themes, sound alerts, and toast notifications from inside herdr.
- Added 9 built-in themes: catppuccin, tokyo night, dracula, nord, gruvbox, one dark, solarized, kanagawa, and rosé pine.
- Added interactive pane scrollbars, manual sidebar resizing, and upstream git ahead/behind indicators in the workspace sidebar.

### Changed
- Redesigned the sidebar into a two-section layout that separates workspace-level triage from per-agent detail, making it easier to supervise multiple agents in parallel.
- Agent state names exposed in the UI and integration surfaces now use `working` and `blocked`.
- Herdr now blocks nested launches by default when started inside a herdr-managed pane; set `advanced.allow_nested = true` to opt back in.

### Fixed
- Improved terminal keyboard protocol parsing and input forwarding across terminal variants, including better handling for shifted printable keys.
- Fixed Ghostty on macOS misparsing some arrow-key and modifier/enhanced key sequences.
- Refined sidebar rollups and pane ordering so workspace status and agent lists stay more stable and predictable.

### Documentation
- Refreshed the README, socket API reference, and reusable agent skill docs to better explain herdr's agent multiplexer model and integration surface.

## [0.1.2] - 2026-03-28

### Added
- Added first-run onboarding flow that lets you choose notification preferences (sound and toast) on startup.
- Added optional visual toast notifications in the top-right corner for background workspace events (completion and attention-needed alerts).
- Added configurable keybindings for all navigate mode actions: new workspace, rename workspace, close workspace, resize mode, and toggle sidebar. See the [configuration docs](https://herdr.dev/docs/configuration/) for the full key reference.
- Added configuration validation with startup diagnostics. Invalid key combinations or duplicate bindings now fall back to safe defaults with a visible warning.

### Changed
- **Breaking:** Default prefix key changed from `ctrl+s` to `ctrl+b` to avoid common terminal flow control conflicts.
- Workspaces now derive their identity from the repository or folder of their root pane, updating automatically as you navigate. Custom names act as overrides rather than static labels.
- Sidebar now shows workspace numbers again in expanded view.
- Refined sidebar presentation with consistent marker/name/state ordering and comma-separated agent summaries.
- Keybinding parser now accepts special keys (`enter`, `esc`, `tab`, `backspace`, `space`) and function keys (`f1`–`f12`).

### Documentation
- Split configuration reference into dedicated configuration docs with full keybinding documentation and config diagnostics explanation.

## [0.1.1] - 2026-03-28

### Added
- Added optional sound notifications for agent state changes, including a completion chime when background work finishes and an alert when an agent needs input.
- Added per-agent sound overrides under `[ui.sound.agents]`, so you can mute or enable notifications by agent instead of using one global setting. Droid notifications are muted by default.

### Changed
- Request alerts now play even when the agent is in the active workspace, while completion sounds remain limited to background workspaces.

### Fixed
- Improved foreground job detection on Linux and macOS so herdr can recognize agents that run through wrapper processes or generic runtimes, including cases like Codex running under `node`.
- Made Claude Code state detection more stable by handling more spinner variants and smoothing short busy/idle flicker during screen updates.

## [0.1.0] - 2026-03-27

### Added
- Initial release.
