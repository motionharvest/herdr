#!/usr/bin/env bash
# Seed a throwaway herdr server with the three scenes the README screenshots show.
#
# Every workspace, pane, and agent state here is staged. Nothing reads from a
# real session, so the images can be regenerated without publishing whatever
# happened to be on screen at the time.
#
# Usage:
#   export HERDR_SOCKET_PATH=/tmp/shot.sock
#   export XDG_CONFIG_HOME=/tmp/shot-config
#   scripts/seed_readme_screenshots.sh
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "$script_dir/.." && pwd)"
bin="${HERDR_SHOT_BIN:-$repo_dir/target/release/herdr}"

# The demo repos live under the fake HOME the server was started with, so pane
# titles read "~/lab/webapp" instead of whatever is on the machine that ran this.
demo_root="${HERDR_SHOT_HOME:-$HOME}/lab"
main_socket="${XDG_CONFIG_HOME:-$HOME/.config}/herdr/herdr.sock"

if [[ -z "${HERDR_SOCKET_PATH:-}" ]]; then
  echo "set HERDR_SOCKET_PATH to a throwaway socket first" >&2
  exit 2
fi
if [[ "$HERDR_SOCKET_PATH" == "$HOME/.config/herdr/herdr.sock" || "$HERDR_SOCKET_PATH" == "$main_socket" ]]; then
  echo "refusing to seed the main herdr session: $HERDR_SOCKET_PATH" >&2
  exit 1
fi

run() { "$bin" "$@"; }

mkrepo() {
  local dir="$demo_root/$1"
  [[ -d "$dir/.git" ]] && { printf '%s\n' "$dir"; return; }
  mkdir -p "$dir"
  git -C "$dir" init -q -b main
  git -C "$dir" -c user.email=demo@example.com -c user.name=demo \
    commit -q --allow-empty -m "initial commit"
  printf '%s\n' "$dir"
}
jqr() { python3 -c 'import sys,json;d=json.load(sys.stdin)["result"];print(" ".join(d[k][f] for k,f in zip(sys.argv[1::2],sys.argv[2::2])))' "$@"; }

mkws() {
  run workspace create --label "$1" --cwd "$(mkrepo "$1")" --no-focus \
    | jqr workspace workspace_id root_pane pane_id tab tab_id
}

split() {
  run pane split "$1" --direction "$2" --no-focus \
    | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"]["pane"]["pane_id"])'
}

report() {
  run pane report-agent "$1" --source readme-shot --agent "$2" --state "$3" \
    --custom-status "$4" --seq "${5:-1}" >/dev/null
}

show_agent() {
  # Replace the pane's screen with a staged transcript and park there, so no
  # shell prompt appears under an agent's output.
  run pane run "$1" "clear; cat '$2'; sleep infinity" >/dev/null
}

show_shell() {
  run pane run "$1" "clear; cat '$2'" >/dev/null
}

transcripts="${HERDR_SHOT_TRANSCRIPTS:-$(mktemp -d)}"
mkdir -p "$transcripts"

cat >"$transcripts/claude.txt" <<'EOF'

  claude  worktree: main

> the pane title truncates the repo path as soon as I split a pane. see if the
  prefix is costing more than it needs to.

  Reading src/ui/panes.rs.

  The title is built once as a string and then re-parsed at render time to
  color the repo path. Two private-use glyphs cost columns before the name:
  the pane icon, and the separator in front of the repo path. Both are Nerd
  Font codepoints, so a terminal without a patched font draws a replacement
  box while the layout still reserves the width.

  * Edit  src/ui/panes.rs
    +23 -26   drop both glyphs, split the git suffix on the agent label

  The separator was doing double duty: decoration, and the anchor the renderer
  searched for to find where the repo path started. That is now
  split_title_git_suffix, taking the boundary from the agent label's closing
  brace — the same place push_title_name_spans already ends the name.

  * Bash  cargo clippy --all-targets --locked -- -D warnings
    Finished `dev` profile in 8.76s

  * Bash  cargo nextest run ui::panes
     Summary [0.412s] 38 tests run: 38 passed

  Done. The name starts two columns earlier and truncates two columns later,
  and the repo path and branch keep their separate colors.

> _
EOF

cat >"$transcripts/shell.txt" <<'EOF'
$ git log --oneline -8
809c051 docs: head the readme install command with its platforms
f21d413 docs: lead the readme with the install command
ae22c95 release: v0.8.4
9808f44 docs: finalize release docs for v0.8.4
3530ca7 feat(panes): drop the nerd font icons from pane titles
ee85a37 release: v0.8.3
5641265 docs: finalize release docs for v0.8.3
5bc076d feat(sidebar): draw a drop line for space reorder drags

$ just check
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
    Finished `dev` profile in 17.97s
cargo nextest run --locked -E "all()"
    Starting 2159 tests across 10 binaries
     Summary [11.200s] 2159 tests run: 2159 passed, 0 skipped
python3 -m unittest scripts.test_changelog scripts.test_preview
    Ran 44 tests in 0.048s
OK

$ git status -sb
## main...origin/main
EOF

cat >"$transcripts/blocked.txt" <<'EOF'

  claude  worktree: main

> scaffold the settings page

  Looked through the repo first.

  * Read  package.json
  * Read  tsconfig.json
  * Bash  ls src/routes src/components
    ls: cannot access 'src/routes': No such file or directory

  There is no frontend framework committed yet. package.json has the test
  runner and the linter but nothing that renders, and tsconfig targets ES2022
  with jsx unset.

  That choice decides the routing, the build step, and how the settings form
  holds state, so it is not something I want to pick for you and quietly bake
  into thirty files.

  Which should I scaffold against?

    1. React with Vite     matches the existing tsconfig target, largest
                           ecosystem for the form controls this page needs
    2. SvelteKit           smallest bundle, routing comes with it, but it
                           pulls in its own build pipeline
    3. Plain TypeScript    no framework, hand-rolled routing, most work now
                           and least to maintain later

  I have written nothing yet. Say the number and I will scaffold it.

> _
EOF

cat >"$transcripts/api-agent.txt" <<'EOF'

  codex  worktree: main

> add rate limiting to the public endpoints

  Dispatch is one match over the parsed route, so the limiter sits in front of
  it rather than being repeated per handler.

  * Edit  src/api/limits.rs        +64 -0    token bucket keyed by client
  * Edit  src/api/mod.rs           +12 -3    wire the limiter into dispatch
  * Edit  src/api/tests/limits.rs  +88 -0    burst, refill, and reset cases

  * Bash  cargo nextest run api::limits
    Starting 14 tests across 1 binary
     Summary [0.438s] 14 tests run: 14 passed

  The bucket refills at 20 requests a second with a burst of 60, per client
  key, and a request that exceeds it gets a 429 with Retry-After rather than
  being dropped. Health checks are exempt, otherwise a monitor with a tight
  interval would rate-limit itself out of the thing it is monitoring.

  The dev server in the pane below is running the new middleware. The 429 in
  its log is the burst test, not a real client.

> _
EOF

cat >"$transcripts/devserver.txt" <<'EOF'
$ cargo run --bin api-dev
   Compiling api v0.4.1
    Finished dev profile in 6.21s
     Running `target/debug/api-dev`

  api-dev listening on http://127.0.0.1:8787
  rate limiter: 20 req/s, burst 60, health exempt

  GET  /v1/health           200    1.2ms
  GET  /v1/spaces           200    8.4ms
  POST /v1/spaces           201   21.7ms
  GET  /v1/spaces/42        200    3.1ms
  GET  /v1/spaces/42/panes  200    4.8ms
  POST /v1/spaces/42/sync   429    0.4ms   burst exceeded, retry-after 1s
  POST /v1/spaces/42/sync   429    0.3ms   burst exceeded, retry-after 1s
  POST /v1/spaces/42/sync   202   14.2ms
  GET  /v1/health           200    1.1ms
  GET  /v1/spaces           200    7.9ms
EOF

read -r HERDR_WS HERDR_CLAUDE HERDR_TAB < <(mkws herdr)
run tab rename "$HERDR_TAB" "pane titles" >/dev/null
HERDR_SHELL="$(split "$HERDR_CLAUDE" right)"
run pane rename "$HERDR_CLAUDE" "claude panes" >/dev/null
run pane rename "$HERDR_SHELL" "shell" >/dev/null
show_agent "$HERDR_CLAUDE" "$transcripts/claude.txt"
show_shell "$HERDR_SHELL" "$transcripts/shell.txt"

read -r WEB_WS WEB_CLAUDE WEB_TAB < <(mkws webapp)
run tab rename "$WEB_TAB" "settings page" >/dev/null
run pane rename "$WEB_CLAUDE" "claude settings" >/dev/null
show_agent "$WEB_CLAUDE" "$transcripts/blocked.txt"

read -r API_WS API_CODEX API_TAB < <(mkws api)
run tab rename "$API_TAB" "rate limits" >/dev/null
API_DEV="$(split "$API_CODEX" down)"
run pane rename "$API_CODEX" "codex rate limits" >/dev/null
run pane rename "$API_DEV" "dev server" >/dev/null
show_agent "$API_CODEX" "$transcripts/api-agent.txt"
show_agent "$API_DEV" "$transcripts/devserver.txt"

sleep 1.5

# Agent states last, so the transcripts above do not overwrite them.
report "$HERDR_CLAUDE" claude working "tightening pane titles"
report "$WEB_CLAUDE" claude blocked "which framework?"
report "$API_CODEX" codex working "rate limiting"
report "$API_CODEX" codex idle "rate limiting ready" 2

printf '%s\n' \
  "HERDR_WS=$HERDR_WS" "WEB_WS=$WEB_WS" "API_WS=$API_WS" \
  "HERDR_CLAUDE=$HERDR_CLAUDE" "HERDR_SHELL=$HERDR_SHELL" \
  "WEB_CLAUDE=$WEB_CLAUDE" "API_CODEX=$API_CODEX" "API_DEV=$API_DEV"
