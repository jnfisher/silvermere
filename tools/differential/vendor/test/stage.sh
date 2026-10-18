#!/bin/bash
# Copied from the project's MBBSEmu test harness, then edited here (stages orig only); see vendor/SOURCE.md.
# Stage one MBBSEmu instance of MajorMUD 1.11p from the distribution data:
#   test/run/orig   the original WCCMMUD.DLL from the distribution  (telnet 2323)
# Wipes and recreates test/run/orig.  Usage: test/stage.sh [DIST_DIR] [BTURNO]
# Opt-in (the m4 differential group only): STAGE_CODEC_OF_TWINKS=78 writes WCCMMUD.INI with
# CODEC_OF_TWINKS=78, the developer code SYSOP TWEAK/SUMMON/EXPRESS_TRAIN/MONSTER need.
# Unset, the instance keeps the stock settings.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# distribution: a folder given as $1, else the stock zip from the cache (downloaded and checked
# once by tools/differential/fetch-stock.sh; unpacked once into test/run/dist, git-ignored)
DIST="${1:-}"
if [ -z "$DIST" ]; then
  ZIP="$("$ROOT/../fetch-stock.sh")"
  DIST="$ROOT/test/run/dist/WCCMMUD_1.11p_DOS_MBBSEmu_v2"
  if [ ! -f "$DIST/WCCMMUD.DLL" ]; then
    mkdir -p "$DIST" && unzip -qo "$ZIP" -d "$DIST"
  fi
fi
BTURNO="${2:-12345678}"
[ -f "$DIST/WCCMMUD.DLL" ] || { echo "no distribution at $DIST"; exit 1; }

# activation code and WCCADDON.SYS for this BTURNO
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
# from keygen.py where the private workspace provides it; otherwise the precomputed output
# for this BTURNO (test/keys/) is used
if [ -d "$ROOT/WCCMMUD-1.11p-source/SRC" ]; then
  python3 "$ROOT/test/keygen.py" "$BTURNO" --addon-sys "$TMP/WCCADDON.SYS" | tee "$TMP/keygen.txt"
elif [ -f "$ROOT/test/keys/$BTURNO.keygen.txt" ]; then
  cp "$ROOT/test/keys/$BTURNO.keygen.txt" "$TMP/keygen.txt"
  cp "$ROOT/test/keys/$BTURNO.WCCADDON.SYS" "$TMP/WCCADDON.SYS"
  cat "$TMP/keygen.txt"
else
  echo "no key generator and no precomputed keys for BTURNO $BTURNO (test/keys/)"; exit 1
fi
CODE="$(sed -n "s/^ACTIVATE  '\(.*\)'.*/\1/p" "$TMP/keygen.txt")"

stage() {  # stage <instance> <dll> <port>
  local RUN="$ROOT/test/run/$1" MOD="$ROOT/test/run/$1/WCCMMUD"
  rm -rf "$RUN"; mkdir -p "$MOD"
  # module files: DLL, MDF, message files, data templates, update file, map/ANSI/IP files
  cp "$2" "$MOD/WCCMMUD.DLL"
  cp "$DIST"/WCCMMUD.MDF "$DIST"/WCCMMHLP.MSG "$DIST"/WCCTEXT.MSG "$MOD/"
  cp "$DIST"/WCC*.VIR "$DIST"/WCCUPDAT.DAT "$DIST"/WCCMAP01.ANS "$DIST"/WCCDMA.SYS "$MOD/"
  # the MDF's "BBS UP: wccmmutl -needed -fast" runs the DOS utility at every start
  # (applies WCCUPDAT.DAT when needed); MBBSEmu runs it, so it and its screens must be here
  cp "$DIST"/WCCMMUTL.EXE "$DIST"/*.BIN "$MOD/"
  # a Worldgroup install creates each .DAT from its virgin .VIR; the DOS utility opens the .DATs
  for v in "$MOD"/WCC*.VIR; do cp "$v" "${v%.VIR}.DAT"; done
  # ganghouse/emblem/banner/statue/stone descriptions, as WCCMISC.BAT installs them
  for z in WCCHSE WCCEMB WCCBAN WCCSTA WCCSTO; do unzip -qo "$DIST/$z.ZIP" -d "$MOD"; done
  # WCCMMUD.MSG with the generated activation code in place of DEMO
  CODE="$CODE" perl -pe 's/^ACTIVATE \{DEMO\}/ACTIVATE {$ENV{CODE}}/' "$DIST/WCCMMUD.MSG" > "$MOD/WCCMMUD.MSG"
  grep -q "^ACTIVATE {$CODE}" "$MOD/WCCMMUD.MSG" || { echo "ACTIVATE not set"; exit 1; }
  cp "$TMP/WCCADDON.SYS" "$MOD/"
  # the developer code, opt-in (see the top); wccmmud.ini is opened from the BBS folder, so
  # the file goes next to the emulator config and next to the module, in both cases
  if [ -n "${STAGE_CODEC_OF_TWINKS:-}" ]; then
    for f in "$RUN/WCCMMUD.INI" "$RUN/wccmmud.ini" "$MOD/WCCMMUD.INI" "$MOD/wccmmud.ini"; do
      printf 'CODEC_OF_TWINKS=%s\r\n' "$STAGE_CODEC_OF_TWINKS" > "$f"
    done
  fi
  # emulator config
  sed -e "s/@PORT@/$3/" -e "s/@BTURNO@/$BTURNO/" -e "s/@TITLE@/MajorMUD $1/" \
      "$ROOT/test/config/appsettings.json" > "$RUN/appsettings.json"
  sed -e "s|@PATH@|$MOD/|" "$ROOT/test/config/modules.json" > "$RUN/modules.json"
  echo "staged $1: $(shasum -a 256 "$MOD/WCCMMUD.DLL" | cut -c1-16)…  telnet localhost:$3"
}
stage orig  "$DIST/WCCMMUD.DLL" 2323
