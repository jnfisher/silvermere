#!/bin/sh
# Prints the path of the stock MajorMUD 1.11p distribution (the MBBSEmu package), downloading it
# first when the cache has no copy that matches its sha256 (T-120). The URL, size and sha256 are
# read from crates/silvermere-import/src/stock.rs, where they are pinned; the cache is the one the
# tests use: $SILVERMERE_STOCK_CACHE, else ${XDG_CACHE_HOME:-~/.cache}/silvermere/stock.
# A download goes to a temporary file and is renamed into place only once it matches.
# Usage: tools/differential/fetch-stock.sh   (needs curl or wget, and sha256sum or shasum)
set -eu
here=$(cd "$(dirname "$0")" && pwd)
pins=$here/../../crates/silvermere-import/src/stock.rs
pin() { sed -n "s/^pub const $1: &str =[[:space:]]*\"\\(.*\\)\";/\\1/p" "$pins"; }
name=$(pin FILE_NAME)
sha=$(pin SHA256)
url=${SILVERMERE_STOCK_URL:-$(sed -n '/^pub const URL: &str =/,/;/s/^[[:space:]]*"\(.*\)";/\1/p' "$pins")}
if [ -z "$name" ] || [ -z "$sha" ] || [ -z "$url" ]; then
  echo "fetch-stock.sh: cannot read the pins in $pins" >&2
  exit 1
fi

cache=${SILVERMERE_STOCK_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/silvermere/stock}
mkdir -p "$cache"
if command -v sha256sum > /dev/null; then sum() { sha256sum "$1" | cut -d' ' -f1; }
else sum() { shasum -a 256 "$1" | cut -d' ' -f1; }; fi

zip=$cache/$name
if [ -f "$zip" ] && [ "$(sum "$zip")" = "$sha" ]; then
  echo "$zip"
  exit 0
fi
rm -f "$zip"
tmp=$cache/.$name.$$.part
trap 'rm -f "$tmp"' EXIT
echo "fetch-stock.sh: downloading $url" >&2
if command -v curl > /dev/null; then curl -fsSL "$url" -o "$tmp"
else wget -q "$url" -O "$tmp"; fi || {
  echo "fetch-stock.sh: downloading $url failed; fetch it by hand and save it as $zip (sha256 $sha)" >&2
  exit 1
}
got=$(sum "$tmp")
[ "$got" = "$sha" ] || {
  echo "fetch-stock.sh: $url gave sha256 $got, expected $sha; nothing kept" >&2
  exit 1
}
mv -f "$tmp" "$zip"
echo "$zip"
