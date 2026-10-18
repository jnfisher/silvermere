SILVERMERE FOR WINDOWS
======================

Silvermere is a MajorMUD 1.11p server in one program. You do not need a BBS or
MBBSEmu. The first start downloads the stock 1.11p game data once (6.7 MB, from
download.mbbsemu.com), checks it and keeps it in data\cache.


1. UNZIP

   Unzip this folder somewhere you can write to, for example C:\Silvermere or
   your Documents folder. Do not use C:\Program Files: Silvermere keeps its
   world next to silvermere.exe, and Program Files is read-only.


2. DOUBLE-CLICK silvermere.exe

   "Windows protected your PC" (SmartScreen): this program is not signed by a
   known publisher. Click "More info", then "Run anyway".

   The first time, Silvermere downloads the game data and builds the world
   (a minute or two), then asks
   for a sysop name and password. This is the account you run the realm
   with. The password doesn't show as you type it.

   If you close the window while the world is being built, the next start
   says so and offers to build it again: press Enter.

   "Windows Defender Firewall has blocked some features": click "Allow
   access" if friends on your network or the internet will connect. Playing
   on this PC alone works either way.

   When you see this line, the server is up:

     Silvermere is running. Connect MegaMud or a telnet client to localhost
     port 2323. Close this window or press Ctrl-C to stop.

   Leave the window open while people play. If you click inside it and the
   title says "Select", press Esc.


3. CONNECT

   MegaMud: add a BBS entry with address localhost, port 2323, terminal type
   ANSI. At "Username:" type NEW to make a player account, or log in as your
   sysop. At "Make your selection" press M, then E to enter the realm.

   A telnet client (SyncTERM or PuTTY): connect to localhost, port 2323,
   with ANSI colour and the CP437 character set.

   Friends connect to your PC's address instead of localhost. Telnet is not
   encrypted: use a password you use nowhere else.


4. WHERE THINGS ARE KEPT

   silvermere.toml    settings (every line is commented; see the top)
   data\silvermere.db the world, the accounts and the characters
   data\assets\       the room description files
   data\logs\         the server log, one file a day
   data\backups\      backups made with "silvermere backup"


5. BACK UP

   While the server runs, open a Command Prompt in this folder (type cmd in
   Explorer's address bar and press Enter) and run:

     silvermere backup --keep 14

   That writes data\backups\silvermere-<date>-<time>.db and keeps the 14
   newest. To put one back, stop the server first, then:

     silvermere restore data\backups\silvermere-<date>-<time>.db

   With the server stopped you can also copy the whole data folder.


6. STOP

   Close the window or press Ctrl-C. Silvermere saves every character before
   it exits. Do not end it from Task Manager: that skips the save.

   Close the window before you sign out or shut down Windows.

   To start again, double-click silvermere.exe. It goes straight to serving.


7. MORE

   Updating, running as a Windows service, opening the firewall for friends,
   and fixing problems: docs/windows.md in the Silvermere repository.
   If something fails, the window stays open with the error: read it, then
   press Enter to close.
