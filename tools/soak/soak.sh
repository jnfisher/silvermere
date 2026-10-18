#!/bin/bash
# The party soak in segments (T-048). Each step runs alone; a restart of the machine loses at
# most the segment in progress, because the server's database and the characters survive and
# `compare.py` adds the finished segments together.
#
#   MMUD_DATA=...  tools/soak/soak.sh serve   RUN
#   MMUD_DATA=...  tools/soak/soak.sh check   RUN   # partycheck, ~1 min
#   MMUD_DATA=...  tools/soak/soak.sh segment RUN [MINUTES]  # default 25
#   python3 -I tools/soak/compare.py RUN [OTHER_RUN]
#
# RUN is a directory the run owns (it holds `data/`, `seg-NN/`). Against the original, set
# SOAK_PORT=2323 and skip `serve`; the bots and the bridge are the same. PLAY defaults to the bots
# vendored in tools/soak/vendor/play (they read the world from $MMUD_DATA). Needs bridge sessions "" (Brynja) and "cleric" (Isolde),
# which `parties.json` of the checkout names.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
PORT=${SOAK_PORT:-${NEW_PORT:-2325}}   # the server the bots play on: SOAK_PORT, else NEW_PORT, else 2325
cmd=${1:?serve|check|segment}; RUN=${2:?run directory}; mkdir -p "$RUN"
: "${PLAY:=$ROOT/tools/soak/vendor/play}"   # the vendored bots (provenance headers in each file)
export MMUD_DATA=${MMUD_DATA:-}   # the directory of the WCC*.db world files (mmud_data.py), see live-soak.sh

bridges() {   # (re)start both bridges on PORT; a dead machine leaves stale sockets
  for s in "" cleric; do
    ( cd "$PLAY" && python3 bridge.py ${s:+--session $s} stop >/dev/null 2>&1; rm -f "bridge${s:+-$s}.sock"
      python3 bridge.py ${s:+--session $s} start --port "$PORT" --name "soak${s:+-$s}" >/dev/null )
  done
}

say() {   # SESSION TEXT UNTIL TIMEOUT: one line on a bridge session, wait for UNTIL; fails when UNTIL never came
  ( cd "$PLAY" && python3 bridge.py ${1:+--session $1} send "$2" --until "$3" --timeout "${4:-30}" ) 2>/dev/null \
    | grep -qE "$3"
}
login() {   # SESSION NAME PASSWORD: the bridge is at the Username: prompt; ends in the realm
  ( cd "$PLAY" && python3 bridge.py ${1:+--session $1} read --until "Username:" --timeout 30 ) >/dev/null 2>&1
  say "$1" "$2" "Password:"; say "$1" "$3" "Make your selection"; say "$1" M '\[MAJORMUD\]:'
  say "$1" E '\[HP=-?[0-9]+[^]]*\]:' 40
}
logoff() {   # SESSION: the clean exit (a dropped socket would be punished at the next login). Each key
  # is sent only when the one before it got its answer: a Y typed at the wrong menu is a stray line.
  say "$1" x '\[MAJORMUD\]:' 90 && say "$1" X "Make your selection" && say "$1" X "Are you sure" \
    && say "$1" Y "Have a nice day|thanks for calling" 10
  return 0
}

case $cmd in
serve)   # import once (a second call after a restart keeps the database and the characters)
  D="$RUN/data"
  if [ ! -f "$D/silvermere.db" ]; then
    : "${DIST:?DIST = the WCCMMUD distribution directory}"
    "$ROOT/target/debug/silvermere" --data-dir "$D" import --from "$DIST" || exit 1
    echo "create Brynja (Warrior) and Isolde (Cleric) by hand over telnet; accounts:"
    printf 'brynja1\n' | "$ROOT/target/debug/silvermere" --data-dir "$D" account create Brynja --password-stdin
    printf 'isolde1\n' | "$ROOT/target/debug/silvermere" --data-dir "$D" account create Isolde --password-stdin
  fi
  exec "$ROOT/target/debug/silvermere" --config "$ROOT/tests/soak/silvermere.toml" --data-dir "$D" serve ;;
check)
  # partycheck drives two sessions that are already in the realm (it only sends telepaths)
  bridges
  login "" Brynja brynja1; login cleric Isolde isolde1
  ( cd "$PLAY" && python3 partycheck.py --a ":Brynja" --b "cleric:Isolde" --count 50 ) | tee "$RUN/partycheck.txt"
  logoff ""; logoff cleric ;;
segment)
  MIN=${3:-25}
  N=$(printf '%02d' $(( $(ls -d "$RUN"/seg-* 2>/dev/null | wc -l) + 1 )))
  S="$RUN/seg-$N"; mkdir -p "$S"
  bridges
  date +%s > "$S/start"
  for who in Brynja Isolde; do
    ( cd "$PLAY" && python3 -u progress.py --party duo --as $who --minutes "$MIN" ) > "$S/$who.out" 2>&1 &
  done
  wait
  date +%s > "$S/end"
  # the bridge log directories of this segment (raw.bin per character)
  for s in "" cleric; do ( cd "$PLAY" && python3 bridge.py ${s:+--session $s} status ) >> "$S/bridges.txt" 2>&1; done
  logoff "" || true; logoff cleric || true
  echo "segment $N done: $S" ;;
*) echo "serve|check|segment"; exit 2 ;;
esac
