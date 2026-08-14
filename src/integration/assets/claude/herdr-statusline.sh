#!/bin/sh
# installed by herdr
# managed by herdr; reinstalling or updating the integration overwrites this file.
# the statusline that was configured before herdr wrapped it is preserved in
# herdr-statusline-delegate.json beside this file and still renders the display.
# HERDR_INTEGRATION_ID=claude
# HERDR_INTEGRATION_VERSION=6

set -eu

script_dir="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
delegate_file="$script_dir/herdr-statusline-delegate.json"

input_file="$(mktemp "${TMPDIR:-/tmp}/herdr-claude-statusline.XXXXXX")" || exit 0
trap 'rm -f "$input_file"' EXIT HUP INT TERM
cat >"$input_file" 2>/dev/null || true

if [ "${HERDR_ENV:-}" = "1" ] \
  && [ -n "${HERDR_SOCKET_PATH:-}" ] \
  && [ -n "${HERDR_PANE_ID:-}" ] \
  && command -v python3 >/dev/null 2>&1; then
  HERDR_STATUSLINE_INPUT_FILE="$input_file" python3 - <<'PY' || true
import json
import os
import random
import socket
import time

source = "herdr:claude-statusline"
pane_id = os.environ.get("HERDR_PANE_ID")
socket_path = os.environ.get("HERDR_SOCKET_PATH")
input_file = os.environ.get("HERDR_STATUSLINE_INPUT_FILE")

if not pane_id or not socket_path:
    raise SystemExit(0)

payload = {}
if input_file:
    try:
        with open(input_file, encoding="utf-8") as handle:
            content = handle.read()
        if content.strip():
            payload = json.loads(content)
    except Exception:
        payload = {}

params = {
    "pane_id": pane_id,
    "source": source,
    "seq": time.time_ns(),
}
# a session has no name between /clear (or startup) and the title claude
# generates after the first prompt. mirror that: no name clears the title,
# so a pane never wears the previous session's name.
session_name = payload.get("session_name")
if isinstance(session_name, str) and session_name.strip():
    params["title"] = session_name.strip()
else:
    params["clear_title"] = True

request = {
    "id": f"{source}:{int(time.time() * 1000)}:{random.randrange(1_000_000):06d}",
    "method": "pane.report_metadata",
    "params": params,
}

try:
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(0.5)
    client.connect(socket_path)
    client.sendall((json.dumps(request) + "\n").encode())
    try:
        client.recv(4096)
    except Exception:
        pass
    client.close()
except Exception:
    pass
PY
fi

if [ -f "$delegate_file" ] && command -v python3 >/dev/null 2>&1; then
  delegate_command="$(python3 - "$delegate_file" <<'PY' 2>/dev/null || true
import json
import sys

try:
    with open(sys.argv[1], encoding="utf-8") as handle:
        value = json.load(handle)
except Exception:
    raise SystemExit(0)

command = value.get("command") if isinstance(value, dict) else None
if isinstance(command, str) and command.strip():
    print(command)
PY
)"
  if [ -n "$delegate_command" ]; then
    sh -c "$delegate_command" <"$input_file" || true
  fi
fi

exit 0
