#!/bin/sh
# Installs a silvermere release on Linux (T-116):
#   curl -fsSL https://raw.githubusercontent.com/jnfisher/silvermere/main/packaging/install.sh | sh
# Downloads the release archive for this machine, checks its SHA-256 against the release's
# SHA256SUMS, unpacks the binary to ~/silvermere (or --prefix), downloads the stock MajorMUD
# 1.11p distribution into data/cache and checks its SHA-256, runs the first start (the stock
# world, then the sysop prompt) and offers `silvermere service install`. No Rust toolchain
# needed. A re-run over an install (a binary and a world in the prefix) upgrades it instead (T-149):
# it backs up the database, keeps the old binary as silvermere.previous, installs the new one,
# keeps data/ and silvermere.toml, and restarts the systemd unit if there is one for this
# prefix; it stops with nothing done when the version is already the one installed.
# SILVERMERE_RELEASES and SILVERMERE_STOCK_URL override the two URLs (the tests point them at a
# local directory).
set -eu

usage() {
  cat <<'EOF'
Installs a silvermere release on Linux.

  curl -fsSL https://raw.githubusercontent.com/jnfisher/silvermere/main/packaging/install.sh | sh
  curl -fsSL .../install.sh | sh -s -- [OPTIONS]

Options:
  --version V   a release such as 0.7.0 (default: the latest)
  --prefix DIR  where to install (default ~/silvermere; /opt/silvermere with --system)
  --system      install as root for a system service run by the `silvermere` user
  --hosted      the server will use a hosted PostgreSQL: no first start on SQLite; the
                service install asks for the URL and builds the world there
  --yes         do not ask: install the binary, then print the remaining commands
  --force       reinstall the installed version, or install an older one (a downgrade)

Over an existing install it upgrades: a backup, the old binary kept as silvermere.previous,
the new one installed, the service restarted. data/ and silvermere.toml are kept.
EOF
}

releases=${SILVERMERE_RELEASES:-https://github.com/jnfisher/silvermere/releases}
# The stock MajorMUD 1.11p distribution (the MBBSEmu package): silvermere ships no game data, so
# it is downloaded here and checked. The same pins as crates/silvermere-import/src/stock.rs (a
# test keeps them equal).
stock_name=WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip
stock_sha=815f7b35cdbc6c4de8deb0b686031e34905da147f360655858058a1f642d3583
stock_url=${SILVERMERE_STOCK_URL:-https://download.mbbsemu.com/modules/WCCMMUD/$stock_name}
version=
prefix=
system=
yes=
hosted=
force=

say() { printf '%s\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case $1 in
    --version) [ $# -ge 2 ] || die "--version needs a value"; version=${2#v}; shift 2 ;;
    --version=*) version=${1#--version=}; version=${version#v}; shift ;;
    --prefix) [ $# -ge 2 ] || die "--prefix needs a value"; prefix=$2; shift 2 ;;
    --prefix=*) prefix=${1#--prefix=}; shift ;;
    --system) system=1; shift ;;
    --hosted) hosted=1; shift ;;
    --yes|-y) yes=1; shift ;;
    --force) force=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
done

uid=$(id -u)
if [ -n "$system" ]; then
  [ "$uid" -eq 0 ] || die "--system installs as root: run it with sudo"
  prefix=${prefix:-/opt/silvermere}
else
  [ "$uid" -ne 0 ] || die "refusing to run as root; run it as the user who will run the server, or pass --system for a system service"
  prefix=${prefix:-$HOME/silvermere}
fi

[ "$(uname -s)" = Linux ] || die "this script installs on Linux only; see docs/install.md for the other systems"
case $(uname -m) in
  x86_64|amd64) arch=x86_64 ;;
  aarch64|arm64) arch=aarch64 ;;
  *) die "no release build for $(uname -m); build from source (docs/install.md)" ;;
esac
target=$arch-unknown-linux-musl

# Answers come from the terminal: when piped from curl, stdin is this script.
tty=${SILVERMERE_TTY:-/dev/tty}
# (probed in a subshell: a failed `exec` redirection ends the shell itself)
if [ -z "$yes" ] && (exec 3< "$tty") 2>/dev/null; then
  exec 3< "$tty"
  interactive=1
else
  interactive=
fi
ask() { # ask PROMPT: true for yes (the default)
  [ -n "$interactive" ] || return 0
  printf '%s [Y/n] ' "$1"
  read -r answer <&3 || answer=n
  case $answer in ''|y|Y|yes|Yes) return 0 ;; *) return 1 ;; esac
}

if command -v curl > /dev/null; then
  fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget > /dev/null; then
  fetch() { wget -q "$1" -O "$2"; }
else
  die "needs curl or wget"
fi
if command -v sha256sum > /dev/null; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum > /dev/null; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  die "needs sha256sum or shasum"
fi

if [ -n "$version" ]; then
  base=$releases/download/v$version
else
  base=$releases/latest/download
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT TERM

fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "could not download $base/SHA256SUMS"
# The archive is named silvermere-VERSION-TARGET.tar.gz; the sums name the version.
line=$(grep -E "  silvermere-[^ ]+-$target\\.tar\\.gz\$" "$tmp/SHA256SUMS" | head -n 1 || true)
[ -n "$line" ] || die "the release has no archive for $target"
want=${line%% *}
archive=${line#*  }
version=${archive#silvermere-}
version=${version%-"$target".tar.gz}

# The unit `silvermere service install` writes, and the env file with a hosted database's URL.
if [ -n "$system" ]; then
  unit=/etc/systemd/system/silvermere.service
  envfile=/etc/silvermere/env
  u=''
  sudo='sudo '
else
  unit=$HOME/.config/systemd/user/silvermere.service
  envfile=$HOME/.config/silvermere/env
  u=' --user'
  sudo=''
fi
# A unit that runs from this prefix: `service install` ran here, so the world is there.
ours=
workdir=
if [ -f "$unit" ]; then
  workdir=$(sed -n 's/^WorkingDirectory=//p' "$unit" | head -n 1)
  if [ -d "$prefix" ] && { [ "$workdir" = "$prefix" ] || [ "$workdir" = "$(cd "$prefix" && pwd -P)" ]; }; then
    ours=1
  fi
fi

# Whether the prefix holds a world: a unit for it, a PostgreSQL URL in its settings, or a
# non-empty SQLite file where its settings put the database. An install stopped before the
# first start (--yes, or setup interrupted) has none and gets the first-install path again.
has_world() {
  [ -z "$ours" ] || return 0
  url=$( (cd "$prefix" && ./silvermere serve --print-config) 2> /dev/null |
    sed -n 's/^url = "\(.*\)"$/\1/p' | head -n 1)
  case $url in
    postgres://*|postgresql://*) return 0 ;;
    sqlite:*) db=${url#sqlite:}; db=${db#//}; db=${db%%\?*} ;;
    *) db=data/silvermere.db ;;
  esac
  case $db in
    /*) ;;
    *) db=$prefix/${db#./} ;;
  esac
  [ -s "$db" ]
}

# version_lt A B: A is older than B, comparing the dotted numbers (a suffix such as -rc1 is
# ignored).
version_lt() {
  awk -v a="$1" -v b="$2" 'BEGIN {
    n = split(a, x, "."); m = split(b, y, "."); k = n > m ? n : m
    i = 0
    while (++i <= k) { p = x[i] + 0; q = y[i] + 0; if (p < q) exit 0; if (p > q) exit 1 }
    exit 1
  }'
}

# An upgrade: the prefix already holds a binary and a world (T-149).
upgrade=
old=
if [ -x "$prefix/silvermere" ]; then
  old=$("$prefix/silvermere" --version 2> /dev/null | head -n 1 || true)
  old=${old#silvermere }
  [ -n "$old" ] || old=unknown
  if [ -d "$prefix/data" ] && has_world; then
    upgrade=1
    if [ "$old" = "$version" ] && [ -z "$force" ]; then
      say "silvermere $version is already installed in $prefix; nothing to do (--force reinstalls it)."
      exit 0
    fi
  fi
fi
# A downgrade: the newer version may have migrated the database, and an older binary refuses
# migrations it does not know.
downgrade=
if [ -n "$upgrade" ] && [ "$old" != unknown ] && version_lt "$version" "$old"; then
  downgrade=1
  if [ -z "$force" ]; then
    say "silvermere $old is installed in $prefix; $version is older, so this is a downgrade."
    say "If $old ran migrations, $version may refuse the database; then you restore a backup made"
    say "before $old was installed (in $prefix/data/backups). Nothing done: pass --force to go ahead."
    exit 1
  fi
fi

service=
if [ -n "$upgrade" ] && [ -f "$unit" ] && command -v systemctl > /dev/null; then
  if [ -n "$ours" ]; then
    service=1
  else
    say "Note: $unit runs silvermere from ${workdir:-elsewhere}, not $prefix; it is not restarted."
  fi
fi

if [ -n "$downgrade" ]; then
  say "silvermere $old is installed in $prefix. This DOWNGRADES it to $version (--force):"
elif [ -n "$upgrade" ]; then
  say "silvermere $old is installed in $prefix. This upgrades it to $version:"
fi
if [ -n "$upgrade" ]; then
  say "  - download $archive from $base and check its SHA-256"
  say "  - back up the database (silvermere backup) and stop, with nothing changed, if that fails"
  say "  - keep the current binary as silvermere.previous and install the new one;"
  say "    data/ and silvermere.toml are kept"
  if [ -n "$service" ]; then
    say "  - restart the silvermere service and wait until it listens again"
  else
    say "  - leave a running server on $old until you restart it"
  fi
  ask "Go ahead?" || die "nothing done"
else
  say "This will:"
  say "  - download $archive from $base"
  say "  - check its SHA-256 against the release's SHA256SUMS"
  say "  - install the silvermere binary into $prefix (data/ and silvermere.toml there are kept)"
  say "  - download the stock MajorMUD 1.11p distribution (6.7 MB) from $stock_url into"
  say "    $prefix/data/cache and check its SHA-256 (silvermere ships no game data)"
  say "  - ask whether the server uses SQLite (in $prefix/data) or a hosted PostgreSQL"
  say "  - run the first start in that database: build the world if there is none, ask for a sysop if there is none,"
  say "    and ask how to back up (by default every day at 04:30, keeping the last 30)"
  say "  - offer to install it as a systemd service (silvermere service install)"
  ask "Go ahead?" || die "nothing done"
fi

fetch "$base/$archive" "$tmp/$archive" || die "could not download $base/$archive"
have=$(sha256 "$tmp/$archive")
[ "$have" = "$want" ] || die "SHA-256 mismatch for $archive: expected $want, got $have; nothing installed"
say "SHA-256 checked: $have"

tar -C "$tmp" -xzf "$tmp/$archive"
src=$tmp/${archive%.tar.gz}
[ -x "$src/silvermere" ] || die "$archive holds no silvermere binary"

# The backup comes before anything changes, by the installed binary (it knows the database),
# with the unit's database URL and as the user the server runs as. It is safe while the server
# runs.
backup() {
  (
    cd "$prefix"
    if [ -f "$envfile" ]; then
      set -a
      # shellcheck disable=SC1090 # written by `service install`
      . "$envfile"
      set +a
    fi
    if [ -n "$system" ] && id silvermere > /dev/null 2>&1 && command -v runuser > /dev/null; then
      exec runuser -u silvermere -- ./silvermere backup
    fi
    exec ./silvermere backup
  )
}
backup_file=
if [ -n "$upgrade" ]; then
  if ! out=$(backup 2>&1); then
    say "$out"
    die "the backup failed, so nothing was changed; fix what it says (or run ./silvermere backup in $prefix yourself), then run this again"
  fi
  say "$out"
  backup_file=$(printf '%s\n' "$out" | sed -n 's/ ([0-9]* bytes)$//p' | head -n 1)
  case $backup_file in
    /*|'') ;;
    *) backup_file=$prefix/${backup_file#./} ;;
  esac
fi

mkdir -p "$prefix"
# One generation of the old binary, for the way back (a reinstall of the same version keeps
# the one there).
if [ -e "$prefix/silvermere" ] && [ "$old" != "$version" ]; then
  cp -p "$prefix/silvermere" "$prefix/.silvermere.previous.new"
  mv -f "$prefix/.silvermere.previous.new" "$prefix/silvermere.previous"
fi
# A new file renamed over the old one, so a running server keeps its binary until restarted.
cp "$src/silvermere" "$prefix/.silvermere.new"
chmod 755 "$prefix/.silvermere.new"
mv -f "$prefix/.silvermere.new" "$prefix/silvermere"
if [ -e "$prefix/silvermere.toml" ]; then
  : # yours, kept
elif [ -e "$src/silvermere.toml" ]; then
  cp "$src/silvermere.toml" "$prefix/silvermere.toml"
else
  say "Note: $archive has no silvermere.toml template; the defaults apply (docs/config.md)."
fi
[ ! -e "$src/README.md" ] || cp "$src/README.md" "$prefix/README.md"
if [ -n "$upgrade" ]; then
  if [ "$old" = "$version" ]; then
    say "Reinstalled silvermere $version in $prefix (silvermere.previous is unchanged)."
  else
    say "Installed silvermere $version in $prefix (was $old, kept as silvermere.previous)."
  fi
else
  say "Installed silvermere $version in $prefix."
fi

# The stock world's data: kept when already there and matching, else downloaded and checked,
# then put in place under its name (a partial or wrong file is never left in the cache).
cache=$prefix/data/cache
mkdir -p "$cache"
if [ -f "$cache/$stock_name" ] && [ "$(sha256 "$cache/$stock_name")" = "$stock_sha" ]; then
  say "Stock data already there and checked: $cache/$stock_name"
else
  fetch "$stock_url" "$tmp/$stock_name" ||
    die "could not download $stock_url; download it by hand and save it as $cache/$stock_name (SHA-256 $stock_sha)"
  have=$(sha256 "$tmp/$stock_name")
  [ "$have" = "$stock_sha" ] ||
    die "SHA-256 mismatch for $stock_name from $stock_url: expected $stock_sha, got $have; not kept. Download it by hand and save it as $cache/$stock_name"
  cp "$tmp/$stock_name" "$cache/.$stock_name.part"
  mv -f "$cache/.$stock_name.part" "$cache/$stock_name"
  say "Stock data downloaded and checked: $cache/$stock_name"
fi

cd "$prefix"

# The upgrade ends here: the world, the sysop and the service are already there.
if [ -n "$upgrade" ]; then
  # T-150: a version that backs up on its own, with no `backup.*` settings, takes the defaults;
  # a cron line from the old docs would then back up twice, and its --keep prune the folder
  if ./silvermere backup --help 2> /dev/null | grep -q -- '--list'; then
    say "Backups: the server now backs itself up every day (04:30 and the last 30 kept, unless"
    say "  silvermere.toml says otherwise); ${sudo:+sudo -u silvermere }./silvermere backup --list shows them."
    if command -v crontab > /dev/null; then
      # the silvermere program running its `backup` subcommand (after any global flags), not
      # a line that copies data/backups somewhere
      job='(^|[ /])silvermere( +-[^ ]+( +[^-][^ ]*)?)* +backup( |$)'
      cron=$(crontab -l 2> /dev/null | grep -v '^[[:space:]]*#' | grep -E "$job" || true)
      how="${sudo}crontab -e"
      if [ -n "$system" ] && [ -z "$cron" ]; then
        cron=$(crontab -l -u silvermere 2> /dev/null | grep -v '^[[:space:]]*#' | grep -E "$job" || true)
        how="sudo crontab -u silvermere -e"
      fi
      if [ -n "$cron" ]; then
        say "Note: a crontab still runs silvermere backup:"
        printf '%s\n' "$cron" | sed 's/^/  /'
        say "  Remove that line ($how): with it there are two backups a day, and its --keep"
        say "  deletes backups the server keeps (backup.keep)."
      fi
    fi
  fi
  if [ -n "$downgrade" ]; then
    say "This was a downgrade from $old. If $version refuses the database ($old migrated it), stop it"
    say "and restore a backup made before $old was installed, from $prefix/data/backups:"
    say "  ${sudo:+sudo -u silvermere }./silvermere restore data/backups/silvermere-YYYYMMDD-HHMMSS.db"
    [ -z "$backup_file" ] || say "The backup just taken, of $old's database, is $backup_file."
  fi
  way_back() {
    say "The way back:"
    say "  cd $prefix && mv -f silvermere.previous silvermere && ${sudo}systemctl$u restart silvermere"
    if [ -n "$backup_file" ]; then
      say "If $old then refuses the database (the new version may have migrated it), put the backup back with it:"
      say "  ${sudo}systemctl$u stop silvermere && ${sudo:+sudo -u silvermere }./silvermere restore $backup_file && ${sudo}systemctl$u start silvermere"
      say "  (on PostgreSQL: restore --force, with the database URL in SILVERMERE_DATABASE_URL)"
    fi
    say "More: docs/install.md, Upgrading."
  }
  if [ -z "$service" ]; then
    say "If the server is running, stop it and start it again to run $version (./silvermere serve)."
    [ -z "$backup_file" ] || say "The backup: $backup_file"
    exit 0
  fi
  set -- --user
  [ -z "$system" ] || set --
  since=$(date +%s)
  say "Restarting the silvermere service..."
  if ! systemctl "$@" restart silvermere; then
    say "systemctl$u restart silvermere failed. The log: ${sudo}journalctl$u -u silvermere -e"
    way_back
    exit 1
  fi
  wait=${SILVERMERE_WAIT:-60}
  n=0
  while :; do
    line=$(journalctl "$@" -u silvermere --no-pager -o cat --since=@"$since" 2> /dev/null |
      grep 'telnet listening on' | head -n 1 || true)
    [ -z "$line" ] || break
    state=$(systemctl "$@" is-active silvermere 2> /dev/null || true)
    if [ "$state" = failed ] || [ "$n" -ge "$wait" ]; then
      if [ "$state" = active ]; then
        line="The service is running, but its listening line did not show; check the log: ${sudo}journalctl$u -u silvermere -e"
        break
      fi
      say "The service did not come back (${state:-unknown}). The log: ${sudo}journalctl$u -u silvermere -e"
      way_back
      exit 1
    fi
    n=$((n + 1))
    sleep 1
  done
  say "$line"
  systemctl "$@" status silvermere --no-pager || true
  say "silvermere $version is running. The backup: ${backup_file:-(see above)}"
  exit 0
fi

# A hosted database gets its first start from `service install`, in that database: running
# `setup` here would build a world and a sysop in SQLite that the server never uses.
if [ -z "$hosted" ] && [ -n "$interactive" ]; then
  printf 'Database: SQLite in %s/data, or a hosted PostgreSQL (you paste its URL later)? [S/p] ' "$prefix"
  read -r answer <&3 || answer=
  case $answer in p|P|postgres|hosted) hosted=1 ;; esac
fi
sys=--user
[ -z "$system" ] || sys=--system
if [ -n "$hosted" ]; then
  url_flag=--database-url-from-stdin
  rest="./silvermere service install $sys $url_flag"
else
  url_flag=
  rest="./silvermere setup && ./silvermere service install $sys"
fi

if [ -z "$interactive" ]; then
  say "Not asking (--yes or no terminal). Finish with:"
  say "  cd $prefix && $rest"
  say "Backups: on by default, every day at 04:30 keeping the last 30 (the first start asks, on a"
  say "  terminal); change them with ./silvermere config set backup.at 03:15 (or backup.keep, backup.enabled)."
  exit 0
fi

[ -n "$hosted" ] || ./silvermere setup <&3

if ! command -v systemctl > /dev/null; then
  if [ -n "$hosted" ]; then
    say "No systemd here. With SILVERMERE_DATABASE_URL set: cd $prefix && ./silvermere setup && ./silvermere serve"
  else
    say "No systemd here; start it with: cd $prefix && ./silvermere serve"
  fi
elif ask "Install it as a systemd service, started now and at boot?"; then
  [ -z "$hosted" ] || say "It asks for the direct (unpooled) database URL, which does not show as you paste it; then it builds the world and asks for the sysop there."
  # shellcheck disable=SC2086 # url_flag is one word or empty
  ./silvermere service install $sys $url_flag <&3
else
  say "Later: cd $prefix && ./silvermere service install $sys $url_flag"
  [ -n "$hosted" ] || say "Or run it by hand: cd $prefix && ./silvermere serve"
fi
