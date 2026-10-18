#!/usr/bin/env bash
# Readies the Linux runner for ci.yml's Windows cross-compile (T-109): cargo-xwin
# builds x86_64-pc-windows-msvc with clang as the C compiler (ring and SQLite have C code), the
# toolchain's rust-lld as the linker, and llvm-ar (the llvm-tools component) as the librarian. It
# downloads the MSVC CRT and Windows SDK headers and libraries once, into ~/.cache/cargo-xwin
# (about 1 GB, kept between jobs).
#
# No sudo. When the runner has no clang, this fetches the official LLVM release below once
# (1.6 GB, checked against its SHA-256) and keeps only clang and its headers (about 230 MB) in
# ~/.cache/silvermere-tools/llvm-<version>. Installing the system package instead
# (sudo apt-get install -y clang) makes the download unnecessary. Run after runner-prep.sh and
# install-tools.sh; adds clang to PATH for the later steps.
set -euo pipefail
target=x86_64-pc-windows-msvc
LLVM_VERSION=19.1.7
LLVM_SHA256=4a5ec53951a584ed36f80240f6fbf8fdd46b4cf6c7ee87cc2d5018dc37caf679
LLVM_URL="https://github.com/llvm/llvm-project/releases/download/llvmorg-$LLVM_VERSION/LLVM-$LLVM_VERSION-Linux-X64.tar.xz"
tools="${XDG_CACHE_HOME:-$HOME/.cache}/silvermere-tools"

# The runners on this VM share ~/.cache: one fetch of clang at a time (ci.yml's first cargo-xwin
# build takes the same lock for the Windows SDK download). Released when this script exits.
mkdir -p "$tools"
exec 9>"$tools/.xwin.lock"
flock -w 1800 9 || { echo "::error::$tools/.xwin.lock held for 30 min"; exit 1; }

if ! command -v clang >/dev/null; then
  major=${LLVM_VERSION%%.*}
  dir="$tools/llvm-$LLVM_VERSION"
  if [ ! -x "$dir/bin/clang" ]; then
    echo "no clang on the runner: fetching clang $LLVM_VERSION into $dir (once)"
    tmp="$dir.partial"
    rm -rf "$tmp" && mkdir -p "$tmp"
    # one pass: the archive is hashed as it is unpacked, and never stored whole
    curl -sSfL --retry 3 "$LLVM_URL" \
      | tee >(sha256sum | cut -d' ' -f1 > "$tmp.sha256") \
      | tar -xJ -C "$tmp" --strip-components=1 --wildcards \
          "*/bin/clang-$major" "*/lib/clang/$major/include/*"
    # the hash is written by a process substitution, which the pipeline does not wait for
    for _ in $(seq 60); do [ -s "$tmp.sha256" ] && break; sleep 1; done
    sum=$(cat "$tmp.sha256" 2>/dev/null || true)
    rm -f "$tmp.sha256"
    if [ "$sum" != "$LLVM_SHA256" ]; then
      rm -rf "$tmp"
      echo "::error::$LLVM_URL: SHA-256 $sum, expected $LLVM_SHA256"
      exit 1
    fi
    ln -s "clang-$major" "$tmp/bin/clang"
    rm -rf "$dir" && mv "$tmp" "$dir"
  fi
  echo "$dir/bin" >> "$GITHUB_PATH"
  export PATH="$dir/bin:$PATH"
fi
clang --version | head -n1
# the Rust standard library for Windows, and llvm-ar (as llvm-lib) when the system has no LLVM tools
if command -v rustup >/dev/null; then
  rustup target add "$target"
  if ! command -v llvm-lib >/dev/null && ! command -v llvm-ar >/dev/null; then
    rustup component add llvm-tools
  fi
elif [ ! -d "$(rustc --print target-libdir --target "$target")" ]; then
  echo "::error::the runner's Rust has no $target standard library and no rustup to add it"
  exit 1
fi
cargo xwin --version
