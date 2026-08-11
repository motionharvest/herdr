#!/usr/bin/env bash
# Stand up an isolated Herdr session holding the three README scenes, and leave
# it running so it can be screenshotted from a real terminal.
#
# The session gets its own HOME, config dir, and socket, so nothing here reads
# or touches the live session on this machine. Every workspace, repo, pane, and
# agent state is staged: the demo repos are empty git repos created under the
# fake HOME, which is what makes pane titles read "~/lab/webapp" rather than a
# real path.
#
# Usage:
#   scripts/stage_readme_demo.sh            # stage it, print how to attach
#   scripts/stage_readme_demo.sh --refresh  # redraw the panes at the attached size
#   scripts/stage_readme_demo.sh --stop     # tear the staged session down
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "$script_dir/.." && pwd)"
bin="${HERDR_SHOT_BIN:-$repo_dir/target/release/herdr}"

stage="${HERDR_SHOT_STAGE:-/tmp/herdr-readme-demo}"
export HERDR_SHOT_HOME="$stage/home"
export XDG_CONFIG_HOME="$stage/config"
export HERDR_SOCKET_PATH="$stage/herdr.sock"

COLS="${HERDR_SHOT_COLS:-200}"
ROWS="${HERDR_SHOT_ROWS:-50}"

# Panes seeded before a client attaches lay their text out at a placeholder
# size. Once a real window is attached the panes resize, and anything longer
# than the new height keeps its old scroll position with the opening lines
# above the viewport. Redrawing at the attached size fixes the scroll and the
# wrapping in one step.
if [[ "${1:-}" == "--refresh" ]]; then
  if [[ ! -f "$stage/panes.txt" ]]; then
    echo "no staged session found; run $0 first" >&2
    exit 1
  fi
  # shellcheck source=/dev/null
  source "$stage/panes.txt"
  redraw() {
    # The pane is parked on `sleep infinity`, which would swallow the command
    # as stdin rather than run it. Interrupt back to a prompt first.
    HOME="$HERDR_SHOT_HOME" "$bin" pane send-keys "$1" C-c >/dev/null
    sleep 0.3
    HOME="$HERDR_SHOT_HOME" "$bin" pane run "$1" \
      "clear; cat '$stage/transcripts/$2'; sleep infinity" >/dev/null
  }
  redraw "$HERDR_CLAUDE" claude.txt
  redraw "$HERDR_SHELL" shell.txt
  redraw "$WEB_CLAUDE" blocked.txt
  redraw "$API_CODEX" api-agent.txt
  redraw "$API_DEV" devserver.txt
  echo "panes redrawn at the attached size"
  exit 0
fi

if [[ "${1:-}" == "--stop" ]]; then
  HOME="$HERDR_SHOT_HOME" "$bin" server stop >/dev/null 2>&1 || true
  rm -rf "$stage"
  echo "staged demo stopped and removed"
  exit 0
fi

if [[ ! -x "$bin" ]]; then
  echo "build the release binary first: cargo build --release" >&2
  exit 1
fi

# Start from scratch so a rerun never inherits a half-built scene.
HOME="$HERDR_SHOT_HOME" "$bin" server stop >/dev/null 2>&1 || true
rm -rf "$stage"
mkdir -p "$HERDR_SHOT_HOME/lab" "$XDG_CONFIG_HOME/herdr"

# Match the interface the screenshots are meant to show. Kept in the script
# rather than copied from the live config so the images do not drift with
# whatever the person running this happens to have set.
cat >"$XDG_CONFIG_HOME/herdr/config.toml" <<'TOML'
onboarding = false

[theme]
name = "synthwave"

[ui]
agent_panel_scope = "all"
show_agent_labels_on_pane_borders = true

[ui.toast]
delivery = "terminal"

[ui.sound]
enabled = false
TOML

# A plain prompt in the staged HOME, so shell panes do not render whatever
# prompt the host machine uses.
for rc in .bashrc .zshrc; do
  cat >"$HERDR_SHOT_HOME/$rc" <<'RC'
PS1='$ '
PROMPT='$ '
RC
done

# Boot the server by attaching a client and detaching it again. The server
# inherits the staged HOME, which is what makes pane titles read "~/lab/...".
HOME="$HERDR_SHOT_HOME" python3 - "$bin" "$COLS" "$ROWS" <<'PY'
import fcntl, os, pty, select, signal, struct, sys, termios, time

binary, cols, rows = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
env = dict(os.environ)
env["TERM"] = "xterm-256color"
for key in ("HERDR_ENV", "HERDR_PANE_ID"):
    env.pop(key, None)

pid, fd = pty.fork()
if pid == 0:
    os.execvpe(binary, [binary], env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

deadline = time.time() + 15
while time.time() < deadline:
    readable, _, _ = select.select([fd], [], [], 0.3)
    if readable:
        try:
            os.read(fd, 1 << 16)
        except OSError:
            break
os.kill(pid, signal.SIGHUP)
time.sleep(1.0)
PY

if ! HOME="$HERDR_SHOT_HOME" "$bin" status server | grep -q '^status: running'; then
  echo "staged herdr server did not come up" >&2
  exit 1
fi

export HERDR_SHOT_TRANSCRIPTS="$stage/transcripts"
ids="$(HOME="$HERDR_SHOT_HOME" "$script_dir/seed_readme_screenshots.sh")"
eval "$ids"
printf '%s\n' "$ids" >"$stage/panes.txt"

# Booting the server leaves one workspace behind for the client that started
# it. Close anything that is not a staged scene, so the sidebar holds only the
# three spaces the screenshots are about.
HOME="$HERDR_SHOT_HOME" "$bin" workspace list \
  | python3 -c 'import sys, json; print("\n".join(w["workspace_id"] for w in json.load(sys.stdin)["result"]["workspaces"]))' \
  | while read -r ws; do
      case "$ws" in
        "$HERDR_WS" | "$WEB_WS" | "$API_WS") ;;
        *) HOME="$HERDR_SHOT_HOME" "$bin" workspace close "$ws" >/dev/null 2>&1 || true ;;
      esac
    done

HOME="$HERDR_SHOT_HOME" "$bin" workspace focus "$HERDR_WS" >/dev/null

# A one-line launcher, so attaching does not mean retyping three env vars.
cat >"$stage/attach.sh" <<LAUNCH
#!/usr/bin/env bash
# Attach to the staged README demo session.
exec env HOME="$HERDR_SHOT_HOME" XDG_CONFIG_HOME="$XDG_CONFIG_HOME" \\
  HERDR_SOCKET_PATH="$HERDR_SOCKET_PATH" "$bin"
LAUNCH
chmod +x "$stage/attach.sh"

cat >"$stage/scenes.txt" <<SCENES
overview        $HERDR_WS   herdr space: a working agent beside a shell
blocked         $WEB_WS   webapp space: an agent waiting on an answer
api-workspace   $API_WS   api space: a dev server under an agent
SCENES

cat <<INFO

staged demo is running.

attach with:
  $stage/attach.sh

the three scenes, switch with prefix+w or by clicking the sidebar:
$(sed 's/^/  /' "$stage/scenes.txt")

once attached, redraw the panes at your window size before shooting:
  $script_dir/stage_readme_demo.sh --refresh

save screenshots as overview.png, blocked.png, api-workspace.png

tear it down with:
  $script_dir/stage_readme_demo.sh --stop
INFO
