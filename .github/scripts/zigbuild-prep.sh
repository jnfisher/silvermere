#!/usr/bin/env bash
# Readies the basement Linux runner for release.yml's musl builds (T-117): zig and
# cargo-zigbuild at pinned versions, installed once each into ~/.cache/silvermere-tools (zig-<version>
# and bin/), the way windows-cross-prep.sh and install-tools.sh install theirs, plus the Rust
# standard library for each target given. The zig release is checked against its SHA-256. No sudo.
# Run after runner-prep.sh; adds zig and cargo-zigbuild to PATH for the later steps.
#   usage: zigbuild-prep.sh TARGET...
set -euo pipefail
ZIG_VERSION=0.13.0
ZIG_SHA256=d45312e61ebcc48032b77bc4cf7fd6915c11fa16e4aad116b66c9468211230ea
ZIG_URL="https://ziglang.org/download/$ZIG_VERSION/zig-linux-x86_64-$ZIG_VERSION.tar.xz"
CARGO_ZIGBUILD_VERSION=0.23.4
tools="${XDG_CACHE_HOME:-$HOME/.cache}/silvermere-tools"

# The runners on this VM share ~/.cache: one install at a time. Released when this script exits.
mkdir -p "$tools"
exec 9>"$tools/.zig.lock"
flock -w 1800 9 || { echo "::error::$tools/.zig.lock held for 30 min"; exit 1; }

dir="$tools/zig-$ZIG_VERSION"
if [ ! -x "$dir/zig" ]; then
  echo "fetching zig $ZIG_VERSION into $dir (once)"
  tmp="$dir.partial"
  rm -rf "$tmp" "$tmp.tar.xz" && mkdir -p "$tmp"
  curl -sSfL --retry 3 -o "$tmp.tar.xz" "$ZIG_URL"
  sum=$(sha256sum "$tmp.tar.xz" | cut -d' ' -f1)
  if [ "$sum" != "$ZIG_SHA256" ]; then
    rm -rf "$tmp" "$tmp.tar.xz"
    echo "::error::$ZIG_URL: SHA-256 $sum, expected $ZIG_SHA256"
    exit 1
  fi
  tar -xJ -C "$tmp" --strip-components=1 -f "$tmp.tar.xz"
  rm -f "$tmp.tar.xz"
  rm -rf "$dir" && mv "$tmp" "$dir"
fi

# "cargo-zigbuild 0.23.4" carries the version; built in a throwaway dir, as install-tools.sh does
if ! "$tools/bin/cargo-zigbuild" --version 2>/dev/null | grep -qF " $CARGO_ZIGBUILD_VERSION"; then
  CARGO_TARGET_DIR="$tools/build-zigbuild" \
    cargo install --locked --root "$tools" cargo-zigbuild --version "$CARGO_ZIGBUILD_VERSION"
  rm -rf "$tools/build-zigbuild"
fi

{ echo "$dir"; echo "$tools/bin"; } >> "$GITHUB_PATH"
export PATH="$dir:$tools/bin:$PATH"
for t in "$@"; do rustup target add "$t"; done
zig version
cargo-zigbuild --version  # the binary: `cargo zigbuild` takes no --version
