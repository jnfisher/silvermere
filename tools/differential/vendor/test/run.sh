#!/bin/bash
# Copied from the project's MBBSEmu test harness; edit here; see vendor/SOURCE.md.
# Start a staged MBBSEmu instance (orig|built) in a detached tmux session of the same name.
# MBBSEmu has only a full-screen console UI, so it needs a terminal; tmux provides one.
#   test/run.sh orig        start          tmux attach -t orig     watch the console
#   test/run.sh orig stop   stop           test/run.sh orig screen print the console
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
I="${1:?orig|built}"; RUN="$ROOT/test/run/$I"
case "${2:-start}" in
  stop)   tmux kill-session -t "$I" 2>/dev/null || true; exit 0 ;;
  screen) tmux capture-pane -p -J -t "$I" | sed 's/[[:space:]]*$//' | grep -v '^$'; exit 0 ;;
esac
[ -d "$RUN" ] || { echo "stage first: test/stage.sh"; exit 1; }
# first start creates the BBS databases; the sysop password is the local test value "sysop"
RESET=""; [ -f "$RUN/mbbsemu.db" ] || RESET="-DBRESET sysop"
tmux kill-session -t "$I" 2>/dev/null || true
tmux new-session -d -s "$I" -x 160 -y 50 -c "$RUN" \
  "TERM=xterm-256color '$ROOT/tools/mbbsemu/MBBSEmu' -S appsettings.json -C modules.json $RESET 2>console.err; sleep 86400"
echo "started $I (tmux session '$I')"
