#!/usr/bin/env bash
# Checks that silvermere.exe imports only the DLLs below (T-111). Windows sends the sign-out and
# shutdown events to a console process only while it has loaded neither user32.dll nor gdi32.dll,
# and a DLL that pulls either in (shell32, ole32, comctl32 and more) silences them: the server
# then stops at sign-out without saving. None of the DLLs below loads them. A new import fails
# here: check what loads it before adding it to the list, and keep user32, gdi32 and shell32 out.
# usage: windows-imports.sh EXE   (ci.yml on the cross-built debug exe, release.yml on the shipped one)
set -euo pipefail
exe=$1
allowed='advapi32 bcrypt bcryptprimitives kernel32 ntdll userenv vcruntime140 ws2_32'

readobj=$(command -v llvm-readobj || true)
if [ -z "$readobj" ]; then
  # the toolchain's llvm-tools component (windows-cross-prep.sh adds it when the runner has no LLVM)
  rustup component add llvm-tools >/dev/null
  readobj=$(find "$(rustc --print sysroot)/lib/rustlib" -name 'llvm-readobj*' -type f | head -n1)
fi
[ -x "$readobj" ] || { echo "::error::no llvm-readobj"; exit 1; }

# --coff-imports prints a block per DLL, `Import {` or `DelayImport {` (delay-loaded), each with a
# `Name: X.dll` line; the sed reads the Name lines of both kinds, so a delay-loaded user32 fails too.
dlls=$("$readobj" --coff-imports "$exe" \
  | sed -nE 's/^ *Name: *(.*)\.dll$/\1/Ip' | tr 'A-Z' 'a-z' | sort -u)
[ -n "$dlls" ] || { echo "::error::$exe: no imports read"; exit 1; }
bad=0
for dll in $dlls; do
  case "$dll" in
    api-ms-win-*) continue ;; # API sets of the C runtime and the core: no windowing
  esac
  case " $allowed " in
    *" $dll "*) ;;
    *) echo "::error::$exe imports $dll.dll, which is not on windows-imports.sh's list"; bad=1 ;;
  esac
done
echo "imports:" $dlls
exit $bad
