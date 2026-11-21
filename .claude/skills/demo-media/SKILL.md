---
name: demo-media
description: Make or refresh the web console's screenshots, GIFs and MP4 scenes, or run the 64-character stress test, with tools/demo/run.sh. Use when asked for demo media, promotional GIFs, console screenshots, a stress or load test with simulated players, or a refreshed docs image.
---

# Demo media and the stress run

Everything is done by `tools/demo/run.sh`; this skill adds the judgement around it. Do not
rewrite the steps by hand.

## Run it

```bash
tools/demo/run.sh media demo-run          # screenshots, GIF and MP4 of the console
tools/demo/run.sh stress demo-run 5       # 64 characters for 5 minutes: demo-run/report.json
tools/demo/run.sh all demo-run 5          # both
```

- It builds the release binary and the `populate` example, imports the stock world (once),
  makes 64 characters (`COUNT`, `SEED`), creates the sysop `Demo` with a random password
  (`demo-run/sysop-password`), serves on ports 2623/8380/8388, and plays the characters.
- A second run reuses `demo-run/` (world, characters). Delete it for a clean start.
- In a cloud session the browser and ffmpeg are there; if `require("playwright")` fails,
  set `NODE_PATH=$(npm root -g)`. On GitHub: Actions > "Demo and stress run" > Run workflow,
  then download the artifact.

## Check the result before handing it over

1. Look at every screenshot (`demo-run/media/*.png`) and at a few frames of each video
   (`ffmpeg -ss 8 -i X.mp4 -frames:v 1 frame.png`). Retake a scene when:
   - the player table is empty or nearly empty (the bots had not logged in yet);
   - a click missed (the panel did not open, the drawer shows no command);
   - a page error shows, or the restart scene never says "The server is back".
2. Retake one scene: `node tools/demo/record.js --web http://127.0.0.1:8388 --user Demo
   --password "$(cat demo-run/sysop-password)" --out demo-run/media --scenes moderate`, with
   the server and bots from `run.sh` still running (start them again with `run.sh` if not), then
   `tools/demo/encode.sh demo-run/media`.
3. Sizes: MP4s are a few hundred KB; GIFs about 2-3 MB. For a smaller GIF:
   `tools/demo/encode.sh demo-run/media 1100 800 6`.
4. Stress: report `commands_per_second`, `latency_ms`, deaths, disconnects and the server's
   `cpu_pct` / `rss_mb` from `report.json`. Disconnects or errors above zero need a look in
   `demo-run/serve.log` and `bots.log` before the numbers are quoted.

## What goes in the repository

Only small, finished images, into `docs/images/` and referenced from the docs (the
screenshots). Everything under `docs/` is built into the server program, so nothing large goes
there. The one GIF in git is the README's, `.github/media/console-dark.gif` (the `dark` scene
at 880 px, about 1.5 MB); replace it only when asked. The rest stay out of git: hand them to
the user, or point to the workflow's artifact.

## The scenes

`tools/demo/record.js` defines them: `overview` (the live player list), `moderate` (mute,
jail, release from the player panel), `command-line` (typed commands), `restart` (the page
loses the server and reconnects, still signed in) and `dark`. Add a scene there; keep each
under about 20 seconds.
