"""Stage and start both servers from the same dist/ zip (`--stage`).

original: `test/stage.sh` (vendored: unzip, activation codes, appsettings) and
          `test/run.sh orig` (MBBSEmu in tmux), telnet 2323.
new:      `silvermere import --from <the same unzipped dist>` then `silvermere serve
          --telnet-bind 127.0.0.1:2325`, data dir `vendor/test/run/new`.

Everything is a subprocess; a missing piece is reported (exit 2), never a traceback.
"""
import os
import socket
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
ORIG_PORT = 2323
NEW_PORT = 2325
_procs = []


def fail(msg):
    print(f"stage: {msg}", file=sys.stderr)
    sys.exit(2)


def explo():
    """The vendored tree, or $SILVERMERE_RESEARCH_DIR (another copy of the harness tree)."""
    root = os.environ.get("SILVERMERE_RESEARCH_DIR") or os.path.join(HERE, "vendor")
    if not os.path.isdir(root):
        fail(f"{root} not found")
    return root


def silvermere_bin():
    path = os.environ.get("SILVERMERE_BIN") or os.path.join(ROOT, "target", "debug", "silvermere")
    if not os.path.isfile(path):
        fail(f"{path} not found: cargo build -p silvermere, or set SILVERMERE_BIN")
    return path


def wait_for(port, banner, timeout):
    """True once `port` accepts and sends something containing `banner`."""
    end = time.time() + timeout
    while time.time() < end:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=2) as s:
                s.settimeout(3)
                data = b""
                while banner not in data and time.time() < end:
                    chunk = s.recv(4096)
                    if not chunk:
                        break
                    data += chunk
                if banner in data:
                    return True
        except OSError:
            pass
        time.sleep(1)
    return False


def stage_orig(args):
    root = explo()
    if not os.path.isfile(os.path.join(root, "tools", "mbbsemu", "MBBSEmu")):
        fail(f"{root}/tools/mbbsemu/MBBSEmu not found: download the release HANDOFF.md names "
             "(see tools/differential/README.md), or run the original yourself and pass --orig")
    dist = getattr(args, "dist", None)
    cmd = [os.path.join(root, "test", "stage.sh")] + ([dist] if dist else [])
    for step in (cmd, [os.path.join(root, "test", "run.sh"), "orig"]):
        try:
            subprocess.run(step, check=True, cwd=root, stdout=subprocess.DEVNULL,
                           stderr=subprocess.PIPE, text=True)
        except (subprocess.CalledProcessError, OSError) as e:
            detail = (getattr(e, "stderr", None) or str(e)).strip()[-300:]
            fail(f"{' '.join(os.path.basename(c) for c in step[:2])} failed: {detail}")
    if not wait_for(ORIG_PORT, b"Username:", 120):
        fail(f"MBBSEmu did not answer with 'Username:' on {ORIG_PORT} in 120 s "
             f"(`{root}/test/run.sh orig screen` shows its console)")
    print(f"stage: original up on {ORIG_PORT}")


def stage_new(args):
    root = explo()
    binary = silvermere_bin()
    dist = getattr(args, "dist", None) or os.path.join(
        root, "test", "run", "dist", "WCCMMUD_1.11p_DOS_MBBSEmu_v2")
    data = os.path.join(root, "test", "run", "new")
    os.makedirs(data, exist_ok=True)
    imp = subprocess.run([binary, "--data-dir", data, "import", "--from", dist],
                         capture_output=True, text=True)
    if imp.returncode != 0:
        # silvermere import is a stub until T-008 (PR #16) merges: say so and carry on, the new
        # side then fails at its first prompt and the report says why.
        print(f"stage: `silvermere import` exited {imp.returncode}: "
              f"{(imp.stderr or imp.stdout).strip()[:200]}", file=sys.stderr)
    log = open(os.path.join(data, "serve.log"), "ab")
    _procs.append(subprocess.Popen(
        [binary, "--data-dir", data, "serve", "--telnet-bind", f"127.0.0.1:{NEW_PORT}",
         "--seed", "1"], stdout=log, stderr=log))
    if not wait_for(NEW_PORT, b"", 30):
        print(f"stage: silvermere did not accept on {NEW_PORT} (see {data}/serve.log)",
              file=sys.stderr)


def stage_both(args):
    stage_orig(args)
    stage_new(args)


def stop_both():
    for p in _procs:
        p.terminate()
    for p in _procs:
        try:
            p.wait(10)
        except subprocess.TimeoutExpired:
            p.kill()
    _procs.clear()
    root = os.environ.get("SILVERMERE_RESEARCH_DIR") or os.path.join(HERE, "vendor")
    if os.path.isfile(os.path.join(root, "test", "run.sh")):
        subprocess.run([os.path.join(root, "test", "run.sh"), "orig", "stop"], cwd=root,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
