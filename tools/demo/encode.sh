#!/bin/bash
# Turns the recorded scenes (DIR/*.webm from record.js) into MP4 and GIF beside them.
#   tools/demo/encode.sh DIR [MP4 WIDTH] [GIF WIDTH] [GIF FPS]
# MP4: H.264, starts at once in a browser, a fraction of the GIF's size. GIF: two passes, a
# palette made for each scene, so text stays sharp.
set -euo pipefail
DIR=${1:?the directory of the .webm scenes}
WIDTH=${2:-1100}
GIF_WIDTH=${3:-960}
FPS=${4:-8}
command -v ffmpeg >/dev/null || { echo "ffmpeg is needed" >&2; exit 1; }
for webm in "$DIR"/*.webm; do
  base=${webm%.webm}
  ffmpeg -loglevel error -y -i "$webm" -vf "scale=${WIDTH}:-2:flags=lanczos" \
    -c:v libx264 -pix_fmt yuv420p -crf 24 -preset slow -movflags +faststart -an "$base.mp4"
  ffmpeg -loglevel error -y -i "$webm" \
    -vf "fps=${FPS},scale=${GIF_WIDTH}:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
    "$base.gif"
  printf '%-14s mp4 %5s KB   gif %5s KB\n' "$(basename "$base")" \
    "$(( $(stat -c %s "$base.mp4") / 1024 ))" "$(( $(stat -c %s "$base.gif") / 1024 ))"
done
