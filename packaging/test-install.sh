#!/bin/sh
# Tests install.sh against a fake release in a temporary directory (T-116): no network, no
# Rust. The fake `silvermere` records its arguments, answers --version and makes a fake backup;
# fake systemctl and journalctl stand in for systemd in the upgrade's restart (T-149). The stock distribution is served from a
# local directory too: the real package, from the cache tools/differential/fetch-stock.sh fills
# (T-120), since install.sh checks it against the pinned SHA-256. Run as a normal user
# (install.sh refuses root without --system): sh packaging/test-install.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
install=$here/install.sh
[ "$(id -u)" -ne 0 ] || { echo "run this as a normal user" >&2; exit 1; }
case $(uname -m) in aarch64|arm64) arch=aarch64 ;; *) arch=x86_64 ;; esac

t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fails=0
ok() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1"; fails=$((fails + 1)); }
lacks() { ! grep -q "$1" "$2"; }
check() { # check NAME COMMAND...
  name=$1; shift
  if "$@"; then ok "$name"; else bad "$name"; fi
}

# The release: the layout release.yml makes, one archive per Linux target, and SHA256SUMS.
mkrelease() { # mkrelease VERSION
  rel=$t/releases/download/v$1
  mkdir -p "$rel"
  for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
    d=$t/build/silvermere-$1-$target
    mkdir -p "$d"
    cat > "$d/silvermere" <<EOF
#!/bin/sh
case "\$1" in
  --version) echo "silvermere $1"; exit 0 ;;
  serve) [ "\$2" != --print-config ] || { echo 'url = "sqlite://./data/silvermere.db"'; exit 0; } ;;
  backup)
    # 1.3.0 stands for a version from before the server backed up on its own (T-150)
    if [ "\${2:-}" = --help ]; then [ "$1" = 1.3.0 ] || echo "      --list  show the backups"; exit 0; fi
    echo "silvermere $1 backup\${SILVERMERE_DATABASE_URL:+ url=\$SILVERMERE_DATABASE_URL}" >> "\$HOME/calls"
    [ -z "\${FAKE_BACKUP_FAIL:-}" ] || { echo "backup refused: disk full" >&2; exit 1; }
    mkdir -p data/backups && : > data/backups/silvermere-$1.db
    echo "./data/backups/silvermere-$1.db (0 bytes)"
    exit 0 ;;
esac
echo "silvermere $1 \$*" >> "\$HOME/calls"
EOF
    chmod 755 "$d/silvermere"
    echo '# template' > "$d/silvermere.toml"
    echo '# readme' > "$d/README.md"
    tar -C "$t/build" -czf "$rel/silvermere-$1-$target.tar.gz" "silvermere-$1-$target"
  done
  echo zip > "$rel/silvermere-$1-x86_64-pc-windows-msvc.zip"
  (cd "$rel" && sha256sum silvermere-* > SHA256SUMS)
}
mkrelease 1.0.0
mkrelease 1.1.0
mkrelease 1.2.0
mkrelease 1.3.0
mkrelease 1.10.0
mkdir -p "$t/releases/latest"
cp -r "$t/releases/download/v1.1.0" "$t/releases/latest/download"

stock_name=WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip
stock=${SILVERMERE_STOCK_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/silvermere/stock}/$stock_name
[ -f "$stock" ] || { echo "no $stock: run tools/differential/fetch-stock.sh first (it downloads the package once)" >&2; exit 1; }
mkdir -p "$t/stock"
cp "$stock" "$t/stock/$stock_name"
echo junk > "$t/stock/junk.zip"

export SILVERMERE_RELEASES="file://$t/releases"
export SILVERMERE_STOCK_URL="file://$t/stock/$stock_name"
export HOME="$t/home"
mkdir -p "$HOME"
# answers: go ahead, the database (empty: SQLite; p: hosted), the service
printf 'y\n\ny\n' > "$t/yes"
printf 'y\n\nn\n' > "$t/noservice"
printf 'y\np\ny\n' > "$t/hosted"

# 1. latest, answering yes twice: installed, first start, then the user service
p=$HOME/silvermere
SILVERMERE_TTY=$t/yes sh "$install" > "$t/out1" 2>&1 || cat "$t/out1"
check "installs the binary" test -x "$p/silvermere"
check "copies the config template" grep -q template "$p/silvermere.toml"
check "says what it will do" grep -q "check its SHA-256" "$t/out1"
check "downloads the stock data" cmp -s "$stock" "$p/data/cache/$stock_name"
check "says the stock data is checked" grep -q "Stock data downloaded and checked" "$t/out1"
check "picks the latest release" grep -q "Installed silvermere 1.1.0" "$t/out1"
check "says the first start asks how to back up" grep -q "ask how to back up (by default every day at 04:30, keeping the last 30)" "$t/out1"
check "runs setup, then service install --user" \
  test "$(cat "$HOME/calls")" = "silvermere 1.1.0 setup
silvermere 1.1.0 service install --user"

# 2. a pinned version elsewhere; declining the service
rm "$HOME/calls"
SILVERMERE_TTY=$t/noservice sh "$install" --version v1.0.0 --prefix "$t/declined" > "$t/out2a" 2>&1 || cat "$t/out2a"
check "installs --version" grep -q "Installed silvermere 1.0.0" "$t/out2a"
check "skips the service when declined" test "$(cat "$HOME/calls")" = "silvermere 1.0.0 setup"
check "a first install keeps no silvermere.previous" test ! -e "$t/declined/silvermere.previous"

# 2.1 an install stopped before the first start (--yes: a binary and data/, no world) is not an
#     upgrade: a re-run takes the first-install path again, with no backup
rm -f "$HOME/calls"
sh "$install" --yes --prefix "$t/unfinished" > "$t/out2u" 2>&1 || cat "$t/out2u"
sh "$install" --yes --prefix "$t/unfinished" > "$t/out2v" 2>&1 || cat "$t/out2v"
check "unfinished install: a re-run installs again" grep -qx "Installed silvermere 1.1.0 in $t/unfinished." "$t/out2v"
check "unfinished install: prints the first-start commands" grep -q "silvermere setup && ./silvermere service install --user" "$t/out2v"
check "unfinished install: no backup" test ! -e "$HOME/calls"
SILVERMERE_TTY=$t/noservice sh "$install" --prefix "$t/unfinished" > "$t/out2w" 2>&1 || cat "$t/out2w"
check "unfinished install: with a terminal, runs the first start" test "$(cat "$HOME/calls")" = "silvermere 1.1.0 setup"

# 2a. an upgrade over the first install (a binary and data/ are there): a backup by the
#     installed binary first, the old binary kept, the config, world and stock data kept; no
#     first start, no service offer, and with no unit nothing to restart
#     (T-150: and a crontab with the old docs' backup line: a note to remove it)
rm "$HOME/calls"
echo '# mine' > "$p/silvermere.toml"
echo world > "$p/data/silvermere.db"
mkdir -p "$t/cron"
cat > "$t/cron/crontab" <<'CRON'
#!/bin/sh
[ "$1" = -l ] || exit 1
[ -n "${FAKE_CRONTAB:-}" ] || { echo "no crontab for you" >&2; exit 1; }
printf '%s\n' "# m h dom mon dow command" "$FAKE_CRONTAB"
CRON
chmod 755 "$t/cron/crontab"
FAKE_CRONTAB='0 4 * * * cd ~/silvermere && ./silvermere backup --keep 14' PATH="$t/cron:$PATH" \
  SILVERMERE_TTY=$t/yes sh "$install" --version v1.2.0 > "$t/out2" 2>&1 || cat "$t/out2"
check "upgrade: says both versions" grep -q "silvermere 1.1.0 is installed in $p. This upgrades it to 1.2.0" "$t/out2"
check "upgrade: installs the new version" grep -q "Installed silvermere 1.2.0 in $p (was 1.1.0, kept as silvermere.previous)" "$t/out2"
check "upgrade: the new binary" test "$("$p/silvermere" --version)" = "silvermere 1.2.0"
check "upgrade: keeps the old binary as silvermere.previous" test "$("$p/silvermere.previous" --version)" = "silvermere 1.1.0"
check "upgrade: backs up, then no setup and no service install" test "$(cat "$HOME/calls")" = "silvermere 1.1.0 backup"
check "upgrade: the backup is made" test -e "$p/data/backups/silvermere-1.1.0.db"
check "upgrade: names the backup" grep -q "The backup: $p/data/backups/silvermere-1.1.0.db" "$t/out2"
check "upgrade: says to restart a running server" grep -q "stop it and start it again to run 1.2.0" "$t/out2"
check "upgrade: keeps silvermere.toml" grep -q mine "$p/silvermere.toml"
check "upgrade: keeps the world" grep -q world "$p/data/silvermere.db"
check "upgrade: keeps the checked stock data" grep -q "Stock data already there and checked" "$t/out2"
check "upgrade: says backups are on now" grep -q "Backups: the server now backs itself up every day" "$t/out2"
check "upgrade: finds the old cron backup line" grep -q "Note: a crontab still runs silvermere backup:" "$t/out2"
check "upgrade: shows it" grep -qF "  0 4 * * * cd ~/silvermere && ./silvermere backup --keep 14" "$t/out2"
check "upgrade: says how to remove it" grep -q "Remove that line (crontab -e)" "$t/out2"
check "upgrade: the crontab's comment line is not shown" lacks "m h dom mon dow" "$t/out2"

# 2b. the same version again: nothing done; --force reinstalls, with a backup
rm "$HOME/calls"
sh "$install" --yes --version 1.2.0 > "$t/out2b" 2>&1 || cat "$t/out2b"
check "same version: says so" grep -q "silvermere 1.2.0 is already installed in $p; nothing to do" "$t/out2b"
check "same version: no backup, nothing run" test ! -e "$HOME/calls"
check "same version: silvermere.previous untouched" test "$("$p/silvermere.previous" --version)" = "silvermere 1.1.0"
# an off-site copy of the backups folder is not the old backup line
FAKE_CRONTAB='30 5 * * * rclone sync ~/silvermere/data/backups b2:sm/silvermere-backups' PATH="$t/cron:$PATH" \
  sh "$install" --yes --force --version 1.2.0 > "$t/out2c" 2>&1 || cat "$t/out2c"
check "an off-site copy line: no note" lacks "a crontab still runs" "$t/out2c"
check "--force: reinstalls" grep -q "Reinstalled silvermere 1.2.0 in $p (silvermere.previous is unchanged)" "$t/out2c"
check "--force: backs up first" test "$(cat "$HOME/calls")" = "silvermere 1.2.0 backup"
check "--force: keeps the older silvermere.previous" test "$("$p/silvermere.previous" --version)" = "silvermere 1.1.0"

# 2c. a failing backup stops it before the binary changes
rm "$HOME/calls"
if FAKE_BACKUP_FAIL=1 sh "$install" --yes --version 1.3.0 > "$t/out2d" 2>&1; then
  bad "a failing backup stops the upgrade"
else
  check "a failing backup stops the upgrade" grep -q "the backup failed, so nothing was changed" "$t/out2d"
fi
check "a failing backup: shows why" grep -q "disk full" "$t/out2d"
check "a failing backup: the binary is unchanged" test "$("$p/silvermere" --version)" = "silvermere 1.2.0"
check "a failing backup: silvermere.previous is unchanged" test "$("$p/silvermere.previous" --version)" = "silvermere 1.1.0"

# 2d. a user unit for this prefix, and a hosted database's env file: the backup gets the URL,
#     the unit is restarted and waited for (fake systemctl and journalctl)
sd=$t/systemd
mkdir -p "$sd" "$HOME/.config/systemd/user" "$HOME/.config/silvermere"
cat > "$sd/systemctl" <<'EOF'
#!/bin/sh
echo "systemctl $*" >> "$HOME/calls"
case "$*" in
  *is-active*) echo "${FAKE_STATE:-active}" ;;
  *status*) echo '● silvermere.service - silvermere (user unit)' ;;
esac
EOF
cat > "$sd/journalctl" <<'EOF'
#!/bin/sh
[ -n "${FAKE_STATE:-}" ] || echo 'silvermere: telnet listening on 0.0.0.0:2323'
EOF
chmod 755 "$sd/systemctl" "$sd/journalctl"
unit=$HOME/.config/systemd/user/silvermere.service
printf '[Service]\nWorkingDirectory=%s\nExecStart=%s/silvermere serve\n' "$p" "$p" > "$unit"
echo "SILVERMERE_DATABASE_URL='postgres://db.example/sm'" > "$HOME/.config/silvermere/env"
rm "$HOME/calls"
PATH="$sd:$PATH" sh "$install" --yes --version 1.3.0 > "$t/out2e" 2>&1 || cat "$t/out2e"
check "service: backs up with the unit's URL, restarts, prints the status" \
  test "$(cat "$HOME/calls")" = "silvermere 1.2.0 backup url=postgres://db.example/sm
systemctl --user restart silvermere
systemctl --user status silvermere --no-pager"
check "service: waits for the listening line" grep -q "telnet listening on 0.0.0.0:2323" "$t/out2e"
check "service: shows the status" grep -q "● silvermere.service" "$t/out2e"
check "service: says it is running" grep -q "silvermere 1.3.0 is running" "$t/out2e"

# 2e. the service does not come back: the log command and the way back, and a failure
if FAKE_STATE=failed PATH="$sd:$PATH" sh "$install" --yes --version 1.10.0 > "$t/out2f" 2>&1; then
  bad "service down: fails"
else
  check "service down: fails, saying so" grep -q "The service did not come back (failed)" "$t/out2f"
fi
check "service down: the log command" grep -q "journalctl --user -u silvermere -e" "$t/out2f"
check "service down: the previous binary" grep -q "mv -f silvermere.previous silvermere" "$t/out2f"
check "service down: the backup" grep -q "restore $p/data/backups/silvermere-1.3.0.db" "$t/out2f"

# 2f. a unit that runs another folder is left alone
printf '[Service]\nWorkingDirectory=/srv/other\n' > "$unit"
rm "$HOME/calls"
PATH="$sd:$PATH" sh "$install" --yes --force --version 1.10.0 > "$t/out2g" 2>&1 || cat "$t/out2g"
check "another folder's unit: says so" grep -q "runs silvermere from /srv/other, not $p" "$t/out2g"
check "another folder's unit: not restarted" lacks systemctl "$HOME/calls"
rm -r "$HOME/.config"

# 2g. an older version is a downgrade: refused without --force, with nothing done; with it,
#     installed, saying which backup to restore if the older binary refuses the database
rm -f "$HOME/calls"
if sh "$install" --yes --version 1.3.0 > "$t/out2h" 2>&1; then
  bad "downgrade: refused without --force"
else
  check "downgrade: refused without --force" grep -q "1.3.0 is older, so this is a downgrade" "$t/out2h"
fi
check "downgrade: says why" grep -q "1.3.0 may refuse the database" "$t/out2h"
check "downgrade: nothing done" test ! -e "$HOME/calls"
check "downgrade: the binary is unchanged" test "$("$p/silvermere" --version)" = "silvermere 1.10.0"
sh "$install" --yes --force --version 1.3.0 > "$t/out2i" 2>&1 || cat "$t/out2i"
check "downgrade --force: says so" grep -q "This DOWNGRADES it to 1.3.0" "$t/out2i"
check "downgrade --force: installs it" test "$("$p/silvermere" --version)" = "silvermere 1.3.0"
check "downgrade --force: backs up first" test "$(cat "$HOME/calls")" = "silvermere 1.10.0 backup"
check "downgrade --force: the backup to restore" grep -q "restore a backup made before 1.10.0 was installed" "$t/out2i"
check "a version without scheduled backups: no backups line" lacks "Backups: the server now" "$t/out2i"

# 3. --prefix, with no terminal: installs and prints the rest
rm -f "$HOME/calls"
SILVERMERE_TTY=$t/missing sh "$install" --prefix "$t/elsewhere" > "$t/out3" 2>&1 || cat "$t/out3"
check "--prefix" test -x "$t/elsewhere/silvermere"
check "no terminal: runs nothing" test ! -e "$HOME/calls"
check "no terminal: prints the commands" grep -q "silvermere setup && ./silvermere service install --user" "$t/out3"
check "no terminal: the backup defaults stand" grep -q "Backups: on by default, every day at 04:30 keeping the last 30" "$t/out3"
check "no terminal: no backup settings written" lacks "backup" "$t/elsewhere/silvermere.toml"

# 3b. a hosted database: no setup on SQLite; service install takes the URL and does the first start
rm -f "$HOME/calls"
SILVERMERE_TTY=$t/hosted sh "$install" --prefix "$t/hosted-p" > "$t/out3b" 2>&1 || cat "$t/out3b"
check "hosted: no setup, service install with the URL" \
  test "$(cat "$HOME/calls")" = "silvermere 1.1.0 service install --user --database-url-from-stdin"
rm -f "$HOME/calls"
sh "$install" --yes --hosted --prefix "$t/hosted-q" > "$t/out3c" 2>&1 || cat "$t/out3c"
check "--hosted --yes: prints the service command" \
  grep -q "./silvermere service install --user --database-url-from-stdin" "$t/out3c"
check "--hosted --yes: no setup command" lacks "silvermere setup" "$t/out3c"

# 4. a tampered archive is refused and nothing is installed
rel=$t/releases/download/v1.0.0
mkdir -p "$t/evil" && echo junk > "$t/evil/x"
tar -C "$t/evil" -czf "$rel/silvermere-1.0.0-$arch-unknown-linux-musl.tar.gz" x
if SILVERMERE_TTY=$t/yes sh "$install" --version 1.0.0 --prefix "$t/tampered" > "$t/out4" 2>&1; then
  bad "refuses a checksum mismatch"
else
  check "refuses a checksum mismatch" grep -q "SHA-256 mismatch" "$t/out4"
fi
check "installs nothing on a mismatch" test ! -e "$t/tampered/silvermere"

# 4b. a stock download that is not the package is refused and not kept; so is a missing one
if SILVERMERE_STOCK_URL="file://$t/stock/junk.zip" sh "$install" --yes --prefix "$t/badstock" > "$t/out4b" 2>&1; then
  bad "refuses a wrong stock download"
else
  check "refuses a wrong stock download" grep -q "SHA-256 mismatch for $stock_name" "$t/out4b"
fi
check "keeps no wrong stock data" test -z "$(ls -A "$t/badstock/data/cache")"
if SILVERMERE_STOCK_URL="file://$t/stock/missing.zip" sh "$install" --yes --prefix "$t/nostock" > "$t/out4c" 2>&1; then
  bad "fails on a missing stock download"
else
  check "fails on a missing stock download, saying how to fetch it" grep -q "download it by hand" "$t/out4c"
fi

# 5. a missing release, and --system without root
if sh "$install" --yes --version 9.9.9 > "$t/out5" 2>&1; then bad "fails on a missing release"; else ok "fails on a missing release"; fi
if sh "$install" --yes --system > "$t/out6" 2>&1; then
  bad "--system needs root"
else
  check "--system needs root" grep -q "run it with sudo" "$t/out6"
fi

# 6. root without --system, through a fake id
mkdir -p "$t/fakeid"
printf '#!/bin/sh\necho 0\n' > "$t/fakeid/id"
chmod 755 "$t/fakeid/id"
if PATH="$t/fakeid:$PATH" sh "$install" --yes > "$t/out7" 2>&1; then
  bad "refuses root without --system"
else
  check "refuses root without --system" grep -q "refusing to run as root" "$t/out7"
fi

# 7. no curl and no sha256sum: wget and shasum instead (fakes that read file:// URLs)
fb=$t/fallback
mkdir -p "$fb"
for tool in id uname grep head cut mktemp rm tar gzip mkdir cp chmod mv cat ls; do
  ln -s "$(command -v "$tool")" "$fb/$tool"
done
cat > "$fb/wget" <<EOF
#!/bin/sh
# wget -q URL -O FILE
echo wget >> "$t/fallback.log"
exec cp "\${2#file://}" "\$4"
EOF
cat > "$fb/shasum" <<EOF
#!/bin/sh
# shasum -a 256 FILE
echo shasum >> "$t/fallback.log"
exec $(command -v sha256sum) "\$3"
EOF
chmod 755 "$fb/wget" "$fb/shasum"
shell=$(command -v sh)
PATH=$fb "$shell" "$install" --yes --prefix "$t/fallback-p" > "$t/out8" 2>&1 || cat "$t/out8"
check "wget and shasum: installs" test -x "$t/fallback-p/silvermere"
check "wget and shasum: stock data" cmp -s "$stock" "$t/fallback-p/data/cache/$stock_name"
check "wget used" grep -q wget "$t/fallback.log"
check "shasum used" grep -q shasum "$t/fallback.log"

if [ "$fails" -ne 0 ]; then echo "$fails failed"; exit 1; fi
echo "all passed"
