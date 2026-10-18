#!/bin/sh
# Tests install.sh against a fake release in a temporary directory (T-116): no network, no
# Rust. The fake `silvermere` records its arguments. The stock distribution is served from a
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
check "runs setup, then service install --user" \
  test "$(cat "$HOME/calls")" = "silvermere 1.1.0 setup
silvermere 1.1.0 service install --user"

# 2. a pinned version over it keeps the config; declining the service
rm "$HOME/calls"
echo '# mine' > "$p/silvermere.toml"
SILVERMERE_TTY=$t/noservice sh "$install" --version v1.0.0 > "$t/out2" 2>&1 || cat "$t/out2"
check "installs --version" grep -q "Installed silvermere 1.0.0" "$t/out2"
check "keeps silvermere.toml" grep -q mine "$p/silvermere.toml"
check "keeps the checked stock data" grep -q "Stock data already there and checked" "$t/out2"
check "skips the service when declined" test "$(cat "$HOME/calls")" = "silvermere 1.0.0 setup"

# 3. --prefix, with no terminal: installs and prints the rest
rm -f "$HOME/calls"
SILVERMERE_TTY=$t/missing sh "$install" --prefix "$t/elsewhere" > "$t/out3" 2>&1 || cat "$t/out3"
check "--prefix" test -x "$t/elsewhere/silvermere"
check "no terminal: runs nothing" test ! -e "$HOME/calls"
check "no terminal: prints the commands" grep -q "silvermere setup && ./silvermere service install --user" "$t/out3"

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
