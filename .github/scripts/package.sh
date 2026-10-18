#!/usr/bin/env bash
# Lays out one release archive's directory and checks that it holds exactly the files a user
# gets. release.yml packages every target with it; ci.yml's Windows job packages its build the
# same way, so the Windows zip's contents are checked on every Rust PR without cutting a
# release (T-109).
# usage: package.sh KIND BIN DIR   (KIND: zig, darwin or msvc; DIR is created)
set -euo pipefail
kind=$1 bin=$2 dir=$3
mkdir -p "$dir"
cp "$bin" "$dir/"
cp silvermere.example.toml "$dir/silvermere.toml"
case "$kind" in
  # Windows: the exe, the commented config and the plain-text guide
  # README-WINDOWS.txt with CRLF line endings, for Notepad before Windows 10 1809
  msvc) sed -e 's/\r$//' -e 's/$/\r/' packaging/windows/README-WINDOWS.txt > "$dir/README-WINDOWS.txt"
        want="README-WINDOWS.txt silvermere.exe silvermere.toml" ;;
  *)    cp README.md "$dir/"
        want="README.md silvermere silvermere.toml" ;;
esac
have=$(cd "$dir" && ls -1 | sort | xargs)
if [ "$have" != "$want" ]; then
  echo "::error::$dir holds '$have'; expected '$want'"
  exit 1
fi
echo "$dir: $have"
