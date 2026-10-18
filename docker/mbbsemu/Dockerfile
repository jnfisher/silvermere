# syntax=docker/dockerfile:1
# Dev-only image (never published): the original WCCMMUD.DLL under MBBSEmu with staged data,
# for the differential harness (tools/differential). The build context is a directory holding
#   mbbsemu.zip   the Linux release HANDOFF.md names (mbbsemu-linux-x64-*.zip)
#   run/          `test/stage.sh` output for the original: run/orig/{appsettings.json,
#                 modules.json,WCCMMUD/} (with its activation codes)
# Build and run (the workflow does exactly this):
#   docker build -f docker/mbbsemu/Dockerfile -t silvermere-mbbsemu CONTEXT
#   docker run --rm -p 2323:2323 silvermere-mbbsemu
# Opt-in, the m4 differential group only: `docker run -e CODEC_OF_TWINKS=78 ...` writes
# WCCMMUD.INI with the developer code the sysop TWEAK/SUMMON/EXPRESS_TRAIN/MONSTER commands
# need; without it the instance keeps stock settings.
FROM debian:bookworm-slim
# MBBSEmu's console UI (Terminal.Gui) loads libncursesw.so.6 at start and exits without it;
# ncurses-base/ncurses-term carry the terminfo entries for TERM and tmux's own terminal.
RUN apt-get update \
 && apt-get install -y --no-install-recommends unzip tmux util-linux ca-certificates libicu72 libssl3 \
      libncursesw6 ncurses-base ncurses-term \
 && rm -rf /var/lib/apt/lists/*
ENV DOTNET_SYSTEM_GLOBALIZATION_INVARIANT=1 TERM=xterm-256color
COPY mbbsemu.zip /tmp/mbbsemu.zip
RUN mkdir -p /mbbsemu/bin && unzip -q /tmp/mbbsemu.zip -d /mbbsemu/bin && rm /tmp/mbbsemu.zip \
 && chmod +x /mbbsemu/bin/MBBSEmu
COPY run/orig /mbbsemu/run
COPY <<'EOF' /mbbsemu/entrypoint.sh
#!/bin/sh
# MBBSEmu has only a full-screen console UI and needs a terminal: run it under tmux, as
# test/run.sh does, and stay in the foreground until it exits.
set -eu
# a `docker restart` keeps /tmp: an exit marker from the run before would end this one at once
rm -f /tmp/mbbsemu.exited
cd /mbbsemu/run
sed -i 's|"Path": "[^"]*"|"Path": "/mbbsemu/run/WCCMMUD/"|' modules.json
# listen on every interface so the published port reaches it from outside the container
grep -q '"Telnet.IP"' appsettings.json || sed -i 's|"Telnet.Port"|"Telnet.IP": "0.0.0.0",\n  "Telnet.Port"|' appsettings.json
if [ -n "${CODEC_OF_TWINKS:-}" ]; then
  for f in WCCMMUD.INI wccmmud.ini WCCMMUD/WCCMMUD.INI WCCMMUD/wccmmud.ini; do
    printf 'CODEC_OF_TWINKS=%s\r\n' "$CODEC_OF_TWINKS" > "$f"
  done
fi
RESET=""; [ -f mbbsemu.db ] || RESET="-DBRESET sysop"
tmux new-session -d -s orig -x 160 -y 50 \
  "/mbbsemu/bin/MBBSEmu -S appsettings.json -C modules.json $RESET 2>console.err; touch /tmp/mbbsemu.exited"
while [ ! -f /tmp/mbbsemu.exited ]; do sleep 2; done
cat console.err >&2
exit 1
EOF
RUN chmod +x /mbbsemu/entrypoint.sh
EXPOSE 2323
ENTRYPOINT ["/mbbsemu/entrypoint.sh"]
