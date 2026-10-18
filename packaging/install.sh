#!/bin/sh
# Installs a silvermere release on Linux (T-116):
#   curl -fsSL https://raw.githubusercontent.com/jnfisher/silvermere/main/packaging/install.sh | sh
# Downloads the release archive for this machine, checks its SHA-256 against the release's
# SHA256SUMS, unpacks the binary to ~/silvermere (or --prefix), downloads the stock MajorMUD
# 1.11p distribution into data/cache and checks its SHA-256, runs the first start (the stock
# world, then the sysop prompt) and offers `silvermere service install`. No Rust toolchain
# needed. A re-run upgrades the binary and keeps data/ and silvermere.toml.
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

say "This will:"
say "  - download $archive from $base"
say "  - check its SHA-256 against the release's SHA256SUMS"
say "  - install the silvermere binary into $prefix (data/ and silvermere.toml there are kept)"
say "  - download the stock MajorMUD 1.11p distribution (6.7 MB) from $stock_url into"
say "    $prefix/data/cache and check its SHA-256 (silvermere ships no game data)"
say "  - ask whether the server uses SQLite (in $prefix/data) or a hosted PostgreSQL"
say "  - run the first start in that database: build the world if there is none, ask for a sysop if there is none"
say "  - offer to install it as a systemd service (silvermere service install)"
ask "Go ahead?" || die "nothing done"

fetch "$base/$archive" "$tmp/$archive" || die "could not download $base/$archive"
have=$(sha256 "$tmp/$archive")
[ "$have" = "$want" ] || die "SHA-256 mismatch for $archive: expected $want, got $have; nothing installed"
say "SHA-256 checked: $have"

tar -C "$tmp" -xzf "$tmp/$archive"
src=$tmp/${archive%.tar.gz}
[ -x "$src/silvermere" ] || die "$archive holds no silvermere binary"

mkdir -p "$prefix"
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
say "Installed silvermere $version in $prefix."

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
