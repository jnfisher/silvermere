#!/bin/sh
# Runs shellcheck on the given files (T-116). GitHub-hosted ubuntu ships it; the basement
# runners do not, and their Python refuses `pip install` (PEP 668). So when it is missing,
# fetch the pinned static release once into the runner's cache, checksum-verified.
# usage: shellcheck.sh FILE...
set -eu
v=0.10.0
if command -v shellcheck > /dev/null; then
  exec shellcheck "$@"
fi
case $(uname -m) in
  x86_64) arch=x86_64; sum=6c881ab0698e4e6ea235245f22832860544f17ba386442fe7e9d629f8cbedf87 ;;
  aarch64) arch=aarch64; sum=324a7e89de8fa2aed0d0c28f3dab59cf84c6d74264022c00c22af665ed1a09bb ;;
  *) echo "shellcheck.sh: no pinned shellcheck for $(uname -m)" >&2; exit 1 ;;
esac
dir=${XDG_CACHE_HOME:-$HOME/.cache}/silvermere-ci/shellcheck-v$v
if [ ! -x "$dir/shellcheck" ]; then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  tarball=shellcheck-v$v.linux.$arch.tar.xz
  curl -fsSL --retry 3 -o "$tmp/$tarball" \
    "https://github.com/koalaman/shellcheck/releases/download/v$v/$tarball"
  echo "$sum  $tmp/$tarball" | sha256sum -c - > /dev/null
  tar -C "$tmp" -xJf "$tmp/$tarball"
  mkdir -p "$dir"
  mv "$tmp/shellcheck-v$v/shellcheck" "$dir/shellcheck.new"
  mv -f "$dir/shellcheck.new" "$dir/shellcheck"
  rm -rf "$tmp"
  trap - EXIT
fi
"$dir/shellcheck" --version | sed -n 2p
exec "$dir/shellcheck" "$@"
