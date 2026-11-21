#!/bin/bash
# The demo and stress run in one command: a world, 64 characters playing on it, then a stress
# report, the console's screenshots and videos, or both. Everything goes under WORK.
#
#   tools/demo/run.sh media  [WORK]           # screenshots, GIF and MP4 of the console
#   tools/demo/run.sh stress [WORK] [MINUTES] # the bots for MINUTES (default 5): report.json
#   tools/demo/run.sh all    [WORK]           # both
#
# WORK defaults to ./demo-run. A second run reuses its world and characters (delete WORK to
# start again). Needs: cargo, node 18+, ffmpeg; Linux for the bots' own addresses (elsewhere
# they all come from 127.0.0.1). The stock world is downloaded once into WORK/data/cache, or
# copied from SILVERMERE_STOCK_CACHE / ~/.cache/silvermere/stock when it is there.
#
# Settings: COUNT (characters, 64), SEED (7), TELNET_PORT (2623), ADMIN_PORT (8380),
# WEB_PORT (8388).
set -euo pipefail
MODE=${1:-all}
WORK=$(mkdir -p "${2:-demo-run}" && cd "${2:-demo-run}" && pwd)
MINUTES=${3:-5}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
COUNT=${COUNT:-64}
SEED=${SEED:-7}
TELNET_PORT=${TELNET_PORT:-2623}
ADMIN_PORT=${ADMIN_PORT:-8380}
WEB_PORT=${WEB_PORT:-8388}
BIN=$ROOT/target/release/silvermere
POP=$ROOT/target/release/examples/populate
DB="sqlite://$WORK/world.db"
say() { printf '\n== %s\n' "$*" >&2; }

say "building (release)"
(cd "$ROOT" && cargo build --release -p silvermere --bin silvermere --example populate)

export SILVERMERE_DATA_DIR="$WORK/data" SILVERMERE_DATABASE__URL="$DB"
mkdir -p "$WORK/data/cache"
for c in "${SILVERMERE_STOCK_CACHE:-}" "$HOME/.cache/silvermere/stock"; do
  if [ -n "$c" ] && ls "$c"/*.zip >/dev/null 2>&1; then cp -n "$c"/*.zip "$WORK/data/cache/"; break; fi
done

if [ ! -f "$WORK/population.json" ]; then
  say "importing the stock world"
  rm -f "$WORK/world.db"
  (cd "$WORK" && "$BIN" import)
  # the shared stock cache the test suite also reads, so the next run needs no download
  mkdir -p "$HOME/.cache/silvermere/stock" && cp -n "$WORK"/data/cache/*.zip "$HOME/.cache/silvermere/stock/" 2>/dev/null || true
  say "making $COUNT characters"
  "$POP" seed --database-url "$DB" --count "$COUNT" --seed "$SEED" --out "$WORK/population.json"
fi
PASSWORD_FILE="$WORK/sysop-password"
if [ ! -f "$PASSWORD_FILE" ]; then
  head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n' > "$PASSWORD_FILE"
  (cd "$WORK" && "$BIN" account create Demo --sysop --password-stdin < "$PASSWORD_FILE")
fi
TOKEN=$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')

say "starting the server"
(cd "$WORK" && SILVERMERE_ADMIN_TOKEN=$TOKEN exec "$BIN" serve \
  --telnet-bind "127.0.0.1:$TELNET_PORT" --admin-bind "127.0.0.1:$ADMIN_PORT" \
  --set "server.web_bind=\"127.0.0.1:$WEB_PORT\"" > "$WORK/serve.log" 2>&1) &
SERVER=$!
BOTS=
stop() {
  [ -n "$BOTS" ] && kill "$BOTS" 2>/dev/null || true
  kill -TERM "$SERVER" 2>/dev/null && wait "$SERVER" 2>/dev/null || true
}
trap stop EXIT
for _ in $(seq 120); do
  "$BIN" healthcheck --url "http://127.0.0.1:$ADMIN_PORT/healthz" 2>/dev/null && break
  sleep 1
done
"$BIN" healthcheck --url "http://127.0.0.1:$ADMIN_PORT/healthz"

FROM=127.0.1.1
[ "$(uname)" = Linux ] || FROM=same

if [ "$MODE" = stress ] || [ "$MODE" = all ]; then
  say "stress: $COUNT characters for $MINUTES minutes"
  "$POP" run --telnet "127.0.0.1:$TELNET_PORT" --population "$WORK/population.json" \
    --minutes "$MINUTES" --from "$FROM" --server-pid "$SERVER" --report "$WORK/report.json"
fi

if [ "$MODE" = media ] || [ "$MODE" = all ]; then
  say "media: the characters play while the console is recorded"
  "$POP" run --telnet "127.0.0.1:$TELNET_PORT" --population "$WORK/population.json" \
    --minutes 0 --from "$FROM" --report "$WORK/bots.json" > "$WORK/bots.log" 2>&1 &
  BOTS=$!
  sleep 45
  DEMO=$ROOT/tools/demo
  if [ ! -d "$DEMO/node_modules/playwright" ] && ! node -e 'require("playwright")' 2>/dev/null; then
    (cd "$DEMO" && npm install --no-audit --no-fund && npx playwright install chromium)
  fi
  NODE_PATH="$DEMO/node_modules:${NODE_PATH:-}" node "$DEMO/record.js" \
    --web "http://127.0.0.1:$WEB_PORT" --user Demo --password "$(cat "$PASSWORD_FILE")" \
    --out "$WORK/media" --shots
  "$DEMO/encode.sh" "$WORK/media"
  say "media in $WORK/media"
fi
