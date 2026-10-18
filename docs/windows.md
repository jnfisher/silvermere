# Silvermere on Windows

Silvermere runs on Windows as a single program, `silvermere.exe`. You need no BBS software, MBBSEmu,
DLLs or Btrieve files: the first start downloads the stock MajorMUD 1.11p game data once and checks
it ([install.md](install.md#game-data)). This page covers playing on
your own PC, hosting for friends, and running as a Windows service. The release zip carries a
shorter plain-text copy of the first part as `README-WINDOWS.txt`
([packaging/windows/README-WINDOWS.txt](../packaging/windows/README-WINDOWS.txt)).

- [Quick start](#quick-start)
- [Connecting](#connecting)
- [Where things are kept](#where-things-are-kept)
- [Stopping, backing up, updating](#stopping-backing-up-updating)
- [Hosting for friends](#hosting-for-friends)
- [Running as a Windows service](#running-as-a-windows-service)
- [Troubleshooting](#troubleshooting)
- [How Windows is built and tested](#how-windows-is-built-and-tested)
- [Test checklist for a Windows VM](#test-checklist-for-a-windows-vm)

## Quick start

1. Download `silvermere-<version>-x86_64-pc-windows-msvc.zip` from the release page. Until the
   first public release, use the CI build ([How Windows is built and tested](#how-windows-is-built-and-tested)).
2. Unzip it to a folder you can write to, such as `C:\Silvermere` or your Documents folder. Don't use
   `C:\Program Files`: Silvermere keeps its world in a `data` folder beside `silvermere.exe`, and
   Program Files is read-only for normal programs.
3. Double-click `silvermere.exe`.

On the first start, Silvermere:

1. downloads the stock game data into `data\cache` (6.7 MB, once; its sha256 is checked) and builds
   the world into `data\silvermere.db` (a minute or two; it prints a table of what it read);
2. asks for a **sysop name** and **password** (twice). This is the account you run the realm with.
   The password doesn't show as you type it. Leave the name empty to skip this step; you can
   make a sysop later with `silvermere account create NAME --sysop`;
3. starts serving and prints:

```
Silvermere is running. Connect MegaMud or a telnet client to localhost port 2323. Close this window or press Ctrl-C to stop.
```

Later starts skip steps 1 and 2. If the window was closed while step 1 ran, the next start
says the last import did not finish and offers to build the world again: press Enter.

Two Windows prompts may appear on the first run:

- **"Windows protected your PC"** (SmartScreen). The program isn't signed with a code-signing
  certificate, so Windows calls the publisher unknown. Click **More info**, then **Run anyway**.
  Windows remembers this for this copy of the file. To avoid the prompt, right-click the downloaded
  zip, choose **Properties**, tick **Unblock** and click **OK** before unzipping.
- **"Windows Defender Firewall has blocked some features of this app."** Silvermere listens on port
  2323 on every network interface. If only you will play, on this PC, click **Cancel**:
  `localhost` still works. If friends will connect, tick **Private networks** and click
  **Allow access**. See [Hosting for friends](#hosting-for-friends).

The settings are in `silvermere.toml` next to the exe. Every line starts commented out at its
default; remove the `#` and change the value to override it. To listen on this PC only (no
firewall prompt), set `telnet_bind = "127.0.0.1:2323"` under `[server]`.
[config.md](config.md) documents every key.

## Connecting

**MegaMud.** Add a BBS entry with address `localhost` (or the server's address), port `2323` and
terminal type ANSI. The login goes:

1. `Username:` then `Password:`. Type `NEW` at `Username:` to make a player account (open signup
   is on by default), or log in as your sysop.
2. `Make your selection (X to exit):`. `M` enters MajorMUD.
3. The `[MAJORMUD]:` menu. `E` enters the realm.

**Telnet clients.** Windows' own `telnet` is off by default. Use one of these:

- [SyncTERM](https://syncterm.bbsdev.net/): add a directory entry, connection type Telnet, address
  `localhost`, port `2323`. It speaks ANSI and CP437 out of the box.
- [PuTTY](https://www.chiark.greenend.org.uk/~sgtatham/putty/): connection type **Other: Telnet**,
  host `localhost`, port `2323`. Under **Window > Translation**, set the remote character set to
  **CP437** so the line-drawing characters show.
- Windows' telnet client: **Turn Windows features on or off > Telnet Client**, then
  `telnet localhost 2323` in a Command Prompt.

**Telnet is plaintext.** Everything you type, passwords included, crosses the network
unencrypted. On your own PC or home network that's fine. Over the internet, anyone on the path can
read it. Tell players to use a password they use nowhere else.

## Where things are kept

The data folder is `data` in the folder Silvermere runs from. That is the exe's folder when you
double-click it. In a Command Prompt, it's the current folder unless you pass `--data-dir`.

| Path                  | What                                                     |
| --------------------- | -------------------------------------------------------- |
| `silvermere.toml`     | settings (every key commented, at its default)           |
| `data\silvermere.db`  | the world, the accounts and the characters (SQLite)      |
| `data\assets\`        | the room description files                               |
| `data\logs\`          | the server log, one file a day (`silvermere.log.<date>`) |
| `data\backups\`       | backups made by `silvermere backup`                      |

With PostgreSQL instead of SQLite, `pgpass.conf` is looked up under `%APPDATA%\postgresql\`;
set `PGPASSFILE` otherwise.

The console window shows only the start and stop lines. Everything else goes to `data\logs\`.

## Stopping, backing up, updating

**Stop** by closing the window, or by pressing Ctrl-C (or Ctrl-Break) in it. Silvermere tells the
players, saves every character and exits. Windows gives a closing window about five seconds,
which is plenty. Don't end the process from Task Manager: that skips the save, and characters
lose what they did since the last periodic save.

**Close the window before you sign out or shut down Windows.** Signing out with the server still
running may skip the save. The server hears the sign-out and starts saving, but Windows may end it
before the save is done.

Windows' limits: after a window closes, Windows waits about 5 seconds before it ends the program.
At a sign-out or shutdown it waits about 20 seconds, less if you click **Sign out anyway** or
**Shut down anyway**. Saving normally takes well under a second. The log line
`characters saved: N committed` means the save reached the database, and
`shutdown complete: N characters saved` means the server stopped cleanly. From the start of a
shutdown, each log line is written to the file as it is logged, and each phase is logged as it
starts (`shutdown phase 1/4` to `4/4`), so a log cut short shows how far the shutdown got.

**Back up** while the server runs. Open a Command Prompt in the Silvermere folder (type `cmd` in
Explorer's address bar and press Enter), then run:

```bat
silvermere backup --keep 14
```

That writes `data\backups\silvermere-YYYYMMDD-HHMMSS.db` (UTC) and deletes the oldest beyond 14.
`--to D:\Backups\silvermere.db` writes elsewhere. To run it nightly, add a Task Scheduler task with
**Program** set to the full path of `silvermere.exe`, **Arguments** set to `backup --keep 14`, and
**Start in** set to the Silvermere folder.

**Restore** with the server stopped:

```bat
silvermere restore data\backups\silvermere-20261017-040000.db
```

With the server stopped, a copy of the whole `data` folder is also a complete backup.

**Update** by stopping the server, taking a backup, and replacing `silvermere.exe` with the new
one. Keep `data` and your `silvermere.toml`. The new version migrates the database on its first
start.

## Hosting for friends

1. **Allow Silvermere through the firewall.** Answer the first-run prompt with **Allow access**.
   Or, from a Command Prompt run as administrator:

   ```bat
   netsh advfirewall firewall add rule name="Silvermere telnet" dir=in action=allow protocol=TCP localport=2323
   ```

   Open only 2323. The admin API listens on `127.0.0.1:8080`; leave it there.
   Don't open 8080 ([admin-api.md](admin-api.md)).
2. **On your home network**, friends connect to this PC's IPv4 address (`ipconfig` shows it, for
   example `192.168.1.20`), port 2323.
3. **Over the internet**, forward TCP port 2323 on your router to that address, and give friends
   your public address. A dynamic DNS name helps if your address changes. Remember that
   [telnet is plaintext](#connecting).
4. Keep the PC from sleeping while people play (**Settings > System > Power**).

## Running as a Windows service

A service runs Silvermere in the background, starts it with Windows and needs nobody logged in.
Silvermere has no service mode of its own. [NSSM](https://nssm.cc/) wraps it:

1. Start Silvermere once by double-clicking, to build the world and make the sysop. Then stop it.
2. Download NSSM, then from a Command Prompt run as administrator (adjust `C:\Silvermere`):

   ```bat
   nssm install Silvermere C:\Silvermere\silvermere.exe serve
   nssm set Silvermere AppDirectory C:\Silvermere
   nssm set Silvermere AppStdout C:\Silvermere\data\logs\service.log
   nssm set Silvermere AppStderr C:\Silvermere\data\logs\service.log
   nssm set Silvermere AppStopMethodConsole 20000
   nssm set Silvermere AppExit Default Restart
   nssm start Silvermere
   ```

   - `serve`, not a bare `silvermere.exe`: a service has no console to answer the first-start
     prompts.
   - `AppDirectory` makes `data` and `silvermere.toml` resolve to the Silvermere folder.
   - `AppStopMethodConsole 20000` gives Silvermere 20 seconds after NSSM's Ctrl-C to save every
     character. NSSM's default of 1.5 seconds is too short on a busy realm.
3. `nssm stop Silvermere` stops it cleanly. `nssm remove Silvermere confirm` removes the service.
   Use `services.msc` to set the start type or the account it runs as. The account needs write
   access to the Silvermere folder.

While the service runs, the `backup`, `account` and `sysop` commands work from a Command Prompt in
the same folder. A double-click refuses to start a second server on the same data, so stop the
service first.

## Troubleshooting

- **The window shows an error and "Press Enter to close this window."** Read the error. The common
  ones:
  - `another silvermere is already serving ...`: one is already running (another window, or the
    service). Stop it first.
  - an error saying the address is already in use: another program has port 2323 (or 8080).
    Change `telnet_bind` or `admin_bind` in `silvermere.toml`.
  - `Access is denied` while creating `data`: the folder is read-only. Move Silvermere out of
    Program Files.
- **The server seems frozen, and the window title starts with "Select".** You clicked in the
  console, and Windows pauses a program that writes to a console while text is selected. Press Esc.
- **Line-drawing characters show as accented letters.** Set your client's character set to CP437,
  or set `compat.charset = "utf8"` in `silvermere.toml` for a UTF-8 terminal.
- **Anything else.** Look in `data\logs\` for the day's log file.

## How Windows is built and tested

- **Every pull request that touches the build** cross-compiles `silvermere.exe` on the
  Linux runner with [cargo-xwin](https://github.com/rust-cross/cargo-xwin). This is ci.yml's
  `windows-cross` job (`.github/scripts/windows-cross-prep.sh`). The job also lays out the release
  zip's folder and uploads it as an artifact, and runs clippy for the Windows target over every
  crate and test. It uses the runner's `clang` when there is one. Otherwise it fetches clang
  19.1.7 from the official LLVM release once (checked against a pinned SHA-256) and keeps only
  clang and its headers, about 230 MB, in `~/.cache/silvermere-tools/`. It downloads the Windows
  SDK into `~/.cache/cargo-xwin` (about 1 GB) on its first run.
- **The import check.** The same job checks the DLLs `silvermere.exe` imports against a list
  (`.github/scripts/windows-imports.sh`). Windows sends the sign-out and shutdown events only to
  a console program that has loaded neither `user32.dll` nor `gdi32.dll`, and `shell32.dll` (among
  others) loads `user32.dll`. With either loaded, the server stops at sign-out without saving.
  sqlx's Postgres driver imported `shell32.dll` through etcetera's known-folder call, so the
  workspace patches etcetera with `patches/etcetera`, which reads `%APPDATA%` only (T-111). A new
  DLL fails the check until someone confirms it loads neither.
- **Nothing runs the tests on Windows automatically yet.** This repository is private, and
  GitHub-hosted runners don't start here because of the Actions spending limit (#34). So
  [the checklist below](#test-checklist-for-a-windows-vm) runs on John's Windows VM, by hand, until
  the public repository has hosted CI. ci.yml's hosted `windows` job builds and tests on
  `windows-latest`. It runs in the public repository (docs/open-source-plan.md, decision 3), or
  here when the repository variable `WINDOWS_HOSTED_CI` is `true`.
- **Windows releases need hosted runners too.** release.yml builds the Windows zip on
  `windows-latest`, so Windows releases come from the public repository. Until then, the CI
  artifact is the Windows build.

## Test checklist for a Windows VM

This checklist tests a build from CI on a clean Windows machine (T-109). Work through it in order,
and copy the console text before you close a window. **Never paste a password.** The sysop prompt
doesn't echo it; if one shows anywhere, blank it out.

1. **Download.** On the pull request, open **Checks**, then the **CI** run (its
   `windows (cross-compile, package, clippy)` job), then the run's summary page. Under
   **Artifacts**, download `silvermere-ci-<sha>-x86_64-pc-windows-msvc`. You need to be signed in to
   GitHub. This is a test build (less optimised, so a little slower than a release), cross-compiled
   on Linux.
   *Expect:* a zip holding one folder with `silvermere.exe`, `silvermere.toml` and
   `README-WINDOWS.txt`.
2. **Unzip** the folder to `C:\Silvermere-test`.
3. **Double-click `silvermere.exe`.**
   *Expect:* maybe SmartScreen (click **More info > Run anyway**), then
   `First start: building the world in C:\Silvermere-test\data ...`, the import's count table, and
   `The world is built.` *Note:* which prompts appeared, and roughly how long the build took.
4. **Answer the sysop prompt.** Choose a name, then type the password twice.
   *Expect:* `Created the sysop account <name>.`, then the firewall prompt (click
   **Allow access**), then
   `Silvermere is running. Connect MegaMud or a telnet client to localhost port 2323. ...`
5. **Copy the console text** (select it with the mouse and press Enter, or Ctrl-C in Windows
   Terminal). Keep it for step 13. *Expect:* no password in it.
6. **Connect MegaMud** to `localhost`, port `2323`, ANSI. Type `NEW` at `Username:` and make a
   player account. Press `M`, then `E`.
   *Expect:* the character creation questions, as on a 1.11p BBS.
7. **Create a character and walk a few rooms.** Write down the last room's name and your
   experience and gold.
8. **Close the console window** with its X while MegaMud is still connected.
   *Expect:* MegaMud is told the server is shutting down and disconnects, and the window closes
   within a few seconds.
9. **Double-click `silvermere.exe` again.**
   *Expect:* no world build, no sysop prompt, straight to `Silvermere is running.`
10. **Log in again** with MegaMud. *Expect:* the same character, in the room you wrote down, with
    the same experience and gold.
11. **Try the failure path.** With the server still running, double-click `silvermere.exe` a second
    time. *Expect:* the new window says `another silvermere is already serving ...` and
    `Press Enter to close this window.`, and stays open until you press Enter. Then press Ctrl-C in
    the first window. *Expect:* it saves and closes.
12. **Optional: sign out with a character connected.** Skip this step if you like; the save at
    sign-out is not promised (close the window first, as above). Double-click `silvermere.exe`, log
    in with MegaMud and walk to a different room; write down its name. Leave the server and
    MegaMud running, and sign out of Windows (**Start > your name > Sign out**). Sign back in,
    double-click `silvermere.exe`, and log in again.
    *What the log may show:* the newest file in `C:\Silvermere-test\data\logs\` has
    `stop requested signal="sign-out"` and `shutting down`, then as far as the server got before
    Windows ended it: `shutdown phase 1/4: saving characters`,
    `shutdown phase 2/4: N saves queued in X ms`, `characters saved: N committed in X ms`,
    `shutdown phase 3/4: draining sessions`, `shutdown phase 4/4: stopping the engine and the
    database`, `shutdown complete`. Note whether the character is in the room you wrote down,
    and paste the log's lines from `stop requested` on.
13. **Paste back on the pull request:**
    - the console text from steps 3 to 4 and from step 11;
    - the Windows version (run `winver`);
    - which prompts appeared (SmartScreen, firewall);
    - whether the character came back as it was in step 10 (and step 12, if you did it);
    - the last 50 lines of the newest file in `C:\Silvermere-test\data\logs\` if anything went
      wrong;
    - anything odd: a garbled screen, a hang, a slow step, a confusing message.
