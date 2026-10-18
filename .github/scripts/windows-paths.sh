#!/usr/bin/env bash
# Prints rust=true when the change BASE..HEAD touches what ci.yml's Windows job builds, tests
# or packages (T-109): Rust sources, Cargo files, the crates and their fixtures, migrations,
# the stock archive, the packaging and this workflow. Hosted Windows minutes are billed, so
# every other change skips the job.
# usage: windows-paths.sh BASE HEAD
set -euo pipefail
base=$1 head=$2
if ! files=$(git diff --name-only "$base" "$head" 2>/dev/null); then
  echo "no usable base ($base); building" >&2
  echo "rust=true"
  exit 0
fi
re='(\.rs$|(^|/)Cargo\.(toml|lock)$|^rust-toolchain\.toml$|^(bin|crates|migrations|stock|tests|data|packaging)/'
re+='|^patches/|^silvermere\.example\.toml$|^\.gitattributes$|^\.github/workflows/(ci|release)\.yml$'
re+='|^\.github/scripts/(package|windows-paths|windows-cross-prep|windows-imports|install-tools)\.sh$)'
if grep -Eq "$re" <<<"$files"; then
  echo "rust=true"
else
  echo "rust=false"
fi
