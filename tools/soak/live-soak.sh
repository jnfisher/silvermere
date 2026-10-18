#!/bin/bash
# The whole party soak in one `live.yml` run (the servers live only as long as the run):
# creates Brynja and Isolde on both servers (the m3 differential setups; both are Human
# Warriors), runs partycheck and SEGMENTS segments of MINUTES each against silvermere ($NEW_PORT, default
# 2325) and against the original ($ORIG_PORT, default 2323) with the bots vendored in tools/soak/vendor/play, and compares.
# Reports go under soak-*/ and *.log so the run uploads them.
#
#   command: "$CI_SCRIPTS/start-servers.sh" && tools/soak/live-soak.sh
#
# About 2 x (1 + SEGMENTS x MINUTES) minutes: 105 for the defaults; timeout_minutes 150.
# Exit status: the gates of compare.py (no disconnect, no crash, no unknown line only on
# silvermere); exp/hr, deaths and fallbacks are advisory.
set -euo pipefail
SEGMENTS=${SEGMENTS:-2}; MINUTES=${MINUTES:-25}
ORIG_PORT=${ORIG_PORT:-2323}; NEW_PORT=${NEW_PORT:-2325}   # as the runner this run is on
cd "$(dirname "$0")/../.."
# The bots read the world (rooms, monsters, shops, items) from the .db files MBBSEmu made from
# the distribution; they are in the original's container.
cid=$(docker ps -q --filter "name=silvermere-orig-${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-0}" | head -1)   # this run's original (other runners have theirs)
[ -n "$cid" ] || { echo "no original container (start-servers.sh first)"; exit 1; }
rm -rf soak-world && docker cp "$cid:/mbbsemu/run/WCCMMUD" soak-world
export MMUD_DATA=$PWD/soak-world
ls "$MMUD_DATA"/WCCMP001.db >/dev/null
# accounts and characters on both sides (m3/party makes Aldric and Brynja, m3/bank makes Isolde)
for s in party bank; do
  python3 -I tools/differential/differential.py run --scenario "m3/$s" \
      --orig "127.0.0.1:$ORIG_PORT" --new "127.0.0.1:$NEW_PORT" --report soak-setup || true
done
for side in "new:$NEW_PORT" "orig:$ORIG_PORT"; do
  name=${side%%:*}; export SOAK_PORT=${side##*:}
  tools/soak/soak.sh check "soak-$name" || true
  for _ in $(seq "$SEGMENTS"); do tools/soak/soak.sh segment "soak-$name" "$MINUTES" || true; done
done
# what each bot said (the first lines of each segment's output), so a stuck bot shows in the job log
for f in soak-*/seg-*/Brynja.out soak-*/seg-*/Isolde.out; do
  [ -f "$f" ] || continue
  echo "=== $f ($(wc -l < "$f") lines, $(grep -c 'position ambiguous' "$f") position-ambiguous)"
  head -25 "$f"
done 2>&1 | tee soak-bots.log | grep '^===' || true
python3 -I tools/soak/compare.py soak-new soak-orig --serve-log silvermere-serve.log 2>&1 | tee soak-compare.log
exit "${PIPESTATUS[0]}"
