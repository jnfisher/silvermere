# Importing the legacy world

`silvermere import` reads the MajorMUD 1.11p Btrieve files (`WCC*.VIR`, the virgin set the
distribution ships, or with `--live` a running realm's `WCC*.DAT`) and writes them into the
normalised schema (`docs/schema.md`), then unpacks the runtime text files into
`<data_dir>/assets/` (see "Assets"). It is the only code that knows the Btrieve format and the
game's record layouts (`crates/silvermere-import`).

```
silvermere import [ZIP|DIR | --from DIR|ZIP | --download] [--live] [--map-files WCCMP001.VIR,NEWMP001.VIR] [--dry-run]
                  [--json OUT] [--content-only | --reset-state] [--with-users]
                  [--allow-duplicates] [--legacy-tz ZONE] [--no-patches | --patches PATH]
                  [--overwrite-assets] [--database-url URL]
```

## Sources

- **Nothing** (`silvermere import`): the stock 1.11p distribution, the MBBSEmu package
  `WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip` (6,673,244 bytes, sha256
  `815f7b35cdbc6c4de8deb0b686031e34905da147f360655858058a1f642d3583`, pinned in
  `crates/silvermere-import/src/stock.rs`). silvermere ships no game data: the first import
  downloads it from
  <https://download.mbbsemu.com/modules/WCCMMUD/WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip> into
  `<data_dir>/cache/` and checks its size and sha256. The download goes to a temporary file in
  that directory and is renamed into place only once it matches, so a partial or wrong file is
  never used or kept; the error names the URL and how to fetch the file by hand. Later imports use
  the cached copy, checked again each time; one that no longer matches is downloaded again.
  `--download` fetches it again even when the cached copy is good. `SILVERMERE_STOCK_URL` names a
  mirror (still checked); `HTTPS_PROXY`/`NO_PROXY` and `SSL_CERT_FILE` (extra root certificates,
  PEM) are honoured. The zip is unpacked into `<data_dir>/.import-<pid>/` (about 150 MB),
  imported and removed, a dry run too. A copy left by an interrupted import (SIGINT or SIGKILL,
  which skip the clean-up) is removed by the next import that unpacks one: any `.import-<pid>`
  whose process is gone (or, without `/proc`, older than a day). This is a complete stock world:
  content, initial state and assets.
- **`FILE.zip`** or **`--from FILE.zip`**: a zip of the files, unpacked the same way; works
  offline. `silvermere import WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip` imports a package fetched by hand
  (not checked: a zip you give is yours).
- **`--from DIR`**: a directory holding the files (names in any case); works offline.
- **`--from DIR --live`**: a running realm (see "Live import").

## Flags

| Flag | Meaning |
|---|---|
| `ZIP\|DIR` | A zip (or directory) with the `.VIR` files, as `--from`. |
| `--from DIR\|ZIP` | Directory (or zip) with the `.VIR` files (names matched case-insensitively). Default: the stock distribution, downloaded once into `<data_dir>/cache/` and checked. |
| `--download` | Download the stock distribution again, replacing the cached copy, and import it. |
| `--live` | Read a running realm's `.DAT` set and its players (see "Live import"). Needs `--from`. |
| `--map-files A,B` | Room files, in order. Default `WCCMP001.VIR,NEWMP001.VIR`; with `--live`, `WCCMP001` alone. |
| `--dry-run` | Read, decode and count; no database is opened. |
| `--json OUT` | Also write every decoded record to `OUT/<TABLE>.jsonl` (see "JSON export"). |
| `--content-only` | Write content tables only; state is neither created nor changed. |
| `--reset-state` | Rewrite state from the records (excludes `--content-only`). |
| `--with-users` | Import `WCCUSERS` records as characters (an account per userid, no password; see `account passwd --missing`). |
| `--allow-duplicates` | Keep the first of records with the same key instead of failing. |
| `--legacy-tz ZONE` | The zone the legacy server ran in (IANA name, default `UTC`), for the records' DOS dates. |
| `--no-patches` | Skip the `WCCUPDAT.DAT` patches (see "Patches"). |
| `--patches PATH` | The patch file to apply instead of `DIR/WCCUPDAT.DAT`. A missing file is a warning, not an error. |
| `--overwrite-assets` | Replace the files already in `<data_dir>/assets/` (by default they are kept). |
| `--database-url URL` | `sqlite://...` or `postgres://...`; also `SILVERMERE_DATABASE_URL`. Default `sqlite://data/silvermere.db`. Migrations run first. |

Each file gets a count line, then the timings (a dry run says "to write" for "written"):

```
rooms [WCCMP001.VIR]: 26720 records read, 26720 written, 0 skipped
rooms [NEWMP001.VIR]: 0 records read, 0 written, 0 skipped
...
characters [WCCUSERS.VIR]: 0 records read, 0 written, 0 skipped
patches [WCCUPDAT.DAT] UPD_RACE    13 applied (0 rewritten), 0 inserted
...
patches [WCCUPDAT.DAT] UPD_ROOM    26720 applied (6948 rewritten), 0 inserted
...
patches [WCCUPDAT.DAT]: 38754 records, seqnum 1..38754, 21 gang shops left alone
assets [WCCHSE.ZIP]: 47 files, 47 written, 0 kept
...
assets [text files]: 5 files, 5 written, 0 kept
```

That is the stock import. Each description archive gets its own line (`WCCHSE.ZIP` 47,
`WCCEMB.ZIP` 10, `WCCBAN.ZIP` 10, `WCCSTA.ZIP` 14, `WCCSTO.ZIP` 7), and `[text files]` counts the
loose ones (the map, the release notes and the sysop menu texts).

A record whose key repeats one already read is skipped, and so is a bank book (with
`--with-users`) whose userid has no account and is not a character of the import; any such
skip is an error (found before anything is written, exit 1) unless `--allow-duplicates`. Records skipped by request (state tables
with `--content-only`, users without `--with-users`) are counted with the reason and are not
errors. The `patches` lines (the `PatchReport`) appear when the patch file was applied.

## Files and tables

| File | Record bytes | Key | Tables |
|---|---|---|---|
| `WCCMP001.VIR`, `NEWMP001.VIR` | 1528 | `(map, room)` from the record | `rooms`, `room_exits`, `room_placed_items`; state `room_state`, `room_floor_items`, `room_monsters` |
| `WCCITEMS.VIR` | 1061 | `number` | `items` (+ `active`, state), `item_abilities`, `item_classes`, `item_races`, `item_negates` |
| `WCCKNMSR.VIR` | 732 | `number` | `monsters`, `monster_abilities`, `monster_attacks`, `monster_items`, `monster_spells`; state `monster_state` |
| `WCCACMSR.VIR` | 386 | `number` | state `active_monsters` and children |
| `WCCSPELS.VIR` | 253 | `number` | `spells`, `spell_abilities` |
| `WCCCLASS.VIR` | 153 | `number` | `classes`, `class_abilities` |
| `WCCRACE.VIR` | 126 | `number` | `races`, `race_abilities` |
| `WCCSHOPS.VIR` | 500 | `number` | `shops`, `shop_items`; state `shop_stock` |
| `WCCMSG.VIR` | 253 | `number` | `messages` |
| `WCCTEXT.VIR` | 2022 (variable length) | `(part, number)` | `text_blocks` (`link` kept) |
| `WCCACTS.VIR` | 1010 | `name` (any case) | `actions`, `action_texts` |
| `WCCGANGS.VIR` | 253 | `key` (any case) | state `gangs` |
| `WCCBANKS.VIR` | 72 | `(userid, bank)` | state `bank_books` (only with `--with-users`; needs the account) |
| `WCCITOWN.VIR` | 65 | `id` | state `item_ownership` |
| `WCCUSERS.VIR` | 1998 | `userid` | state `characters` and children (only with `--with-users`) |

Stock counts: rooms 26,720; items 1,950; known monsters 1,101; spells 1,379; shops 178;
messages 3,867; text blocks 3,467; actions 64; classes 15; races 13; the other files are
empty. `NEWMP001.VIR` is a v6 file of 1544-byte records with none in it: a file with no
records is accepted whatever its record length (nothing is decoded); one with records must
match the layout.

`NEWMP001.VIR` is neither patched nor a "new map 1": the game never opens it. The DLL opens
the rooms only from `WCCMP001.DAT` (the fifteen files it opens, plus `WCCVACN` and
`WCCUPDAT.DAT`, are the `Btrieve files: 17` of `WCCMMUD.MDF`); the update looks rooms up in
that file and inserts new ones into it.
`WCCMMUD.RLN`'s install list names `WCCMP001.vir` as the map database and does not list
`NEWMP001`, and `WCCMISC.BAT` (the install step) does not touch it. The importer still reads
it when listed in `--map-files` (the default lists it; it is empty) and a repeated
`(map, room)` is a duplicate; `--map-files WCCMP001.VIR` imports what the game reads.

Not read: `WCCVACN` (reported with `--live`, see "Live import"), `WCCADDON.SYS`, `WCCDMA.SYS`, `WCCREG.TXT`, `WCCKNOWS.SYS`.
`WCCUPDAT.DAT` is applied to the decoded content (see "Patches").

## Assets

The engine reads a few text files by name from `<data_dir>/assets/` (loaded once when the
server starts, `display::Assets::load_dir`): the `FILE DESCRIPTION` room files (the gang houses'
`WCCnnnnn.HSE` and the emblem, banner, statue and stone files, `.EMB` `.BAN` `.STA` `.STO`), the
room map files (`WCCMAP01.ANS`, the MAP command), the release notes `WCCMMUD.NOT` (main menu R)
and the sysop menu's `WCCMMUD.RLN`, `WCCMMSYS.REL` and `WCCMMSYS.NOT` ([sysop.md](sysop.md)).
The import writes them there (`assets::unpack_assets`):

1. the members of the archives the install step `WCCMISC.BAT` unpacks with `pkunzip -o`:
   `WCCHSE.ZIP`, `WCCEMB.ZIP`, `WCCBAN.ZIP`, `WCCSTA.ZIP`, `WCCSTO.ZIP`;
2. then every file of the source with the extension `.ANS`, `.HSE`, `.BAN`, `.EMB`, `.STA`,
   `.STO`, `.NOT`, `.REL` or `.RLN`. A running BBS's own copy (a sysop may have edited a gang
   house's description) wins over the archive's.

Names match in any case, as on DOS. A file already in the assets directory is kept (counted
"kept") unless `--overwrite-assets`, which writes over it under its existing name. A dry run
counts and writes nothing. A damaged archive fails the import, naming it. The archives are
read by the crate's own zip reader (`zip.rs`: stored and deflated members, CRC-32 checked).

## Live import

`--live` imports a running realm: its content as it stands and all of its state, players
included. Stop the BBS first (or copy the files from a clean shutdown).

- **Which file.** The DLL opens only the `.DAT` files (through the host's `opnbtv`); nothing
  in the module copies a `.VIR`. That a missing `.DAT` is created from its `.VIR` is the
  MBBS/Worldgroup host's behaviour (`opnbtv` is the host library's) [I].
  `WCCMMUD.RLN` lists the `.VIR` files as the empty databases, gives "Copy
  WCCUSERS.VIR to WCCUSERS.DAT" and the like as the reset steps, and warns
  that only `wccacmsr.vir` may safely be copied over its `.DAT` (with an offline recovery
  after): "Copying any other file can have bad side-effects". So each table is
  read from its `.DAT` and, only where the `.DAT` is missing, from its `.VIR`; a `.VIR` is
  never read in place of a `.DAT` that is there. The count line names the file read. Without `--live` only the `.VIR` files are read, even
  where a `.DAT` is beside them.
- **Rooms** come from `WCCMP001` alone (the DLL never opens `NEWMP001`, see above);
  `--map-files` overrides it.
- **Players.** `--live` implies `--with-users`: every `WCCUSERS` record becomes a character
  under an account of its userid, created with no password: nobody can log in to it
  until it has one. The import's closing output counts the accounts with no password;
  `silvermere account passwd --missing` gives each a random one and prints
  `name<TAB>password` lines to hand on to the players (`--out FILE` writes them to a new
  file only its owner can read; `--dry-run` lists the accounts), and
  `silvermere account passwd NAME` sets one by hand. The gangs, bank books, item ownership, active monsters, room
  state (floor items, coins, owners, ...), shop stock, monster state and `items.active` come
  from the realm's records through the same state path as the stock import: created where
  missing by default, rewritten with `--reset-state`, untouched with `--content-only`.
- **Patches.** `WCCUPDAT.DAT` is applied as for the stock set: the original applies it at
  every start, so a realm's next start would have applied it too.
- **Per record.** A record read and not written that is not a duplicate key or a skip by
  request gets its own line after the counts:

  ```
  not carried [WCCBANKS.DAT] record 2 (userid Ghost, bank 1): its owner Ghost has no account and no character
  not carried [WCCVACN.DAT] record 1: silvermere has no vacation table (the feature is a stub)
  not carried [WCCUSERS.DAT] record 4 (userid Mira): the account already has a character, kept (--reset-state rewrites it)
  ```

  A bank book whose owner has neither an account nor a `WCCUSERS` record (a player since
  deleted) is skipped and reported, not an error (John, 2026-10-17); in an import without
  `--live` such a book is still an error unless `--allow-duplicates`. `WCCVACN.DAT`, the vacation file the DLL opens only when it exists,
  is read and each record reported: vacations are a stub offered only on the developers' own
  systems.
- **Btrieve features of a live file.** The reader handles v5 and v6 files with records (the
  v6 ones through the page allocation table and the active FCR copy, tested on Worldgroup's
  own `GALTELA.DAT` and `WGSMENU2.DAT`, `tests/btrieve_fixtures.rs`). These are refused, naming
  the file, before anything is written:

  | Found | Error |
  |---|---|
  | `NAME.PRE` beside the file (Btrieve 5 pre-image: changes not committed) | `...: NAME.PRE beside it holds uncommitted changes (the BBS is running or was not shut down cleanly); stop the BBS cleanly, or open the file once with Btrieve to roll it back, then import again` |
  | `NAME.^01` beside the file (an extended file's continuation) | `...: it continues in NAME.^01; extended (multi-file) and truncated Btrieve files are not supported` |
  | a v5 file holding fewer pages than its header counts | `...: the header counts N pages of L bytes, the file holds M; extended ...` |
  | the v5 "inconsistent" mark (FCR +0x22 = 0xFFFF) | `...: marked inconsistent, needs Btrieve recovery before import` |
  | compressed records | `...: compressed records are not supported` |
  | not a Btrieve file | `...: not a Btrieve 5.x/6.x file: ...` |
  | records of another length than the struct | `...: record length is N, expected M` |

  Known limit: a file given an owner name with BUTIL is not detected (see "Owner names" below).

  Owner names: the DLL opens every file with `opnbtv`, which passes no owner
  (the host's owner variant `opnbtvl` is never called), and Btrieve
  refuses to open an owned file without its owner name, so a file the realm runs on carries
  none. The reader does not look for one: where the FCR marks an owner (and encryption) is in
  neither reader this one follows (wbtrv32's `BtrieveDatabase.cc`, MBBSEmu's `BtrieveFile.cs`),
  so there is no confirmed flag to check. The page-count check is v5 only: a v6 header counts
  its pages differently (the stock v6 `NEWMP001.VIR` counts 3 of its 6).

`tests/live_dat.rs` builds such a realm (one player in a gang with a bank book and a limited
item, three rooms with floor state, an orphaned bank book, a vacation record) and checks the
round trip and each refusal. Its `.DAT` files are written at test time by
`tests/support/btrieve_v5.rs` from the stock `.VIR` files' own pages (FCR, keys, ACS) plus data
pages holding the records; the index pages are not rebuilt (the reader never reads them).

## Decoding

Every record is decoded into a flat struct that mirrors the game's record layout member for
member (`RoomRecord`, `ItemRecord`, ...; `decode/*.rs`), `unk_*` spans and cache bytes included. The
member offsets are not written in the decoders: each is looked up by name in the
`silvermere-types` layout (`records::layout`, the T-002 contract) and its size checked against
its Rust type. Member types: characters and bytes u8, signed bytes i8, 16-bit integers i16
or u16, 32-bit integers i32 or u32; little-endian, packed.

- **Text** (`name[N]`): the bytes up to the first NUL, CP437. The flat struct keeps them
  as the game sees them; the schema and the JSON export trim them as T-006's export does
  (white space at both ends).
- **Text blocks.** `WCCTEXT` is variable-length: a record is the 22-byte fixed part and up to
  2000 text bytes, zero-filled to the 2022-byte buffer the game reads it into. The text is stored with
  0x20 added to each byte. The game shifts each stored byte down until it meets a stored NUL,
  then forces `text[1999]` to NUL, and uses the result as a NUL-terminated string, so the text
  is the stored bytes up to the first stored 0x00 or 0x20 (a decoded NUL), at most 1999, each minus 0x20, as
  CP437. In the stock file every block's text ends at a stored 0x00.
- **Coins.** Room and player coins are five named fields `runic..copper`. The `cash[5]`
  arrays of items, known monsters and active monsters are runic first: `cash[0]` is dropped
  or found as runic. The schema stores copper-first `Coins` (five columns). Known record:
  orc rogue (#6, level 1) has raw `cash` `[0, 0, 0, 10, 20]`, 20 copper and 10 silver.

- **Dates.** The records hold DOS packed dates and times, the legacy server's local wall
  clock. The flat records keep the raw values (so `--json` matches T-006); the schema
  holds Unix seconds UTC with 0 for never (PR #14), converted with `dos_to_unix` in the
  `--legacy-tz` zone: monster `killdate`+`killtime` become `killed_at`, gang `created`
  and player `lastdate`/`deathdate` are dates at local midnight, and active-monster
  `created` (a time of day only) is read on the import day in that zone.
  An invalid nonzero date fails the import (a dry run too) with an error naming the
  record and member. The stock files hold no dates (all zero).

## Content and state

The original kept live state inside content records. The import splits them:

| Record | State | Where |
|---|---|---|
| room | `monsters[15]`, floor and hidden items with uses and counts, coins, `owner`, `RF_SPECALIVE`, `lastkill`, `foundmask`, `areacount`, `searchers`, `nummonsters` | `room_state`, `room_floor_items`, `room_monsters` |
| shop | `qty[20]` | `shop_stock` |
| monster | `curalive`, `killdate`/`killtime` (as `killed_at`) | `monster_state` |
| item | `active` (copies in play) | `items.active` |
| gangs, bank books, item ownership, active monsters, users | the whole record | their tables |

The `.VIR` is the virgin state, so a fresh import creates the state rows from the records
(content equals initial state). Modes:

- default: content upserted by key; state created where a row has none, existing state kept
  (so a re-import never resets play);
- `--content-only`: content upserted; no state row created or changed;
- `--reset-state`: content upserted; every state row rewritten from the record.

Known limitation (from T-003's `Shops::put`): shops decide "already has state" from the
`shops` row, not from `shop_stock` (which holds no row when every quantity is zero). So after
a `--content-only` import into an empty database, a later default import does not create
`shop_stock`; run that one with `--reset-state`, or import with the default mode first.
Rooms and monsters check `room_state`/`monster_state` and do not have this gap.

Re-importing content is idempotent: `--content-only` twice leaves every content table
identical (`tests/invariants.rs` checks a checksum of every row of the 25 content tables).

The schema stores the used slots of the record arrays (child tables with a `slot` column,
T-003), so a few values on empty slots are not kept: a room's empty floor slot holds `uses`
-1 or, in some virgin hidden slots, -2; a monster's empty
item slot has a drop chance. The game reads neither while the slot's item number is 0.
Monster spell slots are the exception: the spell pick reads every slot's
chance and the picked slot's level whether or not it names a spell, so `monster_spells` keeps a slot whose spell, chance or level is set (4 stock slots have a
level with no spell).

Writers: one transaction per table through the repositories' `put_many`
(`silvermere-db`), rooms in multi-row statements of 500.

## Patches (`WCCUPDAT.DAT`)

The original applies `WCCUPDAT.DAT` at every start: it opens the file, and without it nobody
may play; then it applies one record per poll until the end. The importer runs the same pass once, after decoding and before the content/state split
(`patch::apply_patches`, `crates/silvermere-import/src/patch/`), so the content tables hold the
patched records and the state tables are created from the patched records' initial values (a
patched room's floor is its initial floor). The import has no run-time state yet, so every
field the merge keeps keeps its virgin value. A `--content-only` re-import with patches is the
same pass on a running world's content:
`--no-patches` followed by `--content-only` gives the same content tables as one patched
import, and the pass is idempotent (`tests/post_update_equivalence.rs`).

**Records.** Each record holds `seqnum` (32-bit, key 0), `type` (`UPD_RACE` 1 ..
`UPD_ACTION` 10) and `data[2022]`, the image of one record of that kind. The stock file is a
Btrieve v5 file of 2015-byte records (the game reads them into a 2027-byte buffer), so 2010 bytes of `data` come from the file and
the rest of the buffer is zero; every image but a text block's fits (a room is 1528 bytes).
Each image is decoded with the `.VIR` decoder of its kind; the bytes after the record are
ignored, as the game ignores them.

**Order.** Ascending `seqnum`, every record, whatever the physical order: the game reads with
GETGT on key 0 from the last `seqnum` applied, starting at 0, so
a record with `seqnum` 0 or below is never reached (counted, not applied). A type byte outside
1..10 is an error naming the `seqnum` (the original ignores it; the stock file has none).

**Lookup**: by `number` for races, classes, shops, items, spells, monsters and messages; by
`(mapnum, roomnum)` for rooms; by `(number, part)` for text blocks; by the verb in any case
for actions. A found record is merged by its kind's rule below; an existing gang shop (`type`
11) is skipped and logged. A key not found is inserted; an inserted text block keeps its text
up to the stored NUL (the record is stored as 23 bytes plus the text length). Inserted
records count under the file `WCCUPDAT.DAT` in the count lines.

**Merge rules**, per kind (`patch/merge_*.rs`, each tested in `tests/patch_merge.rs`):

| Kind | Compared (any difference triggers the copy) | Copied | Kept |
|---|---|---|---|
| race | the ten ability slots; `name`, `unk_0x05C` (as a NUL-terminated string), the six min and six max stats, `hpbonus`, `maxenergy`, `restored`, `exp`, `basecp` | the whole record | nothing (no copy when only an uncompared member differs) |
| class | the ten ability slots; `name`, `unk_0x05E`, `basehp`, `hplevel`, `exp`, `attmult`, `maxenergy`, `restored`, `magery`, `magelvl`, `weapon`, `armour`, `combat`, `titles` | the whole record | as races |
| room | each exit's destination and type, then its parameters without the run-time one (`exitpar2s` of spell, key, trap, user-count, spell-trap exits; `exitpar3s` of timed exits; `exitpar1s` of hidden exits, doors and gates); `placed`; any hidden item in the update; flags; `name`, `desc[7]`, `mapfile`, `type`, `shopnum`, `minlevel`, `maxlevel`, `genmonster`, `light`, `ganghouse`, `maxmonsters`, `unk_0x555`, `mongroup`, `deathroom`, `huproom`, `cmdtext`, `unk_0x5AF`, `roomspell`, `picktrap`, `regendelay`, `areamax`, `ctlroom`, `protmsg`, `specmon` | exit destinations; a changed exit type takes all four parameters; the same type: a key exit gets `exitpar2s` 2 (locked), a door or gate `exitpar1s` 2, a spell or user-count exit keeps `exitpar2s`, any other takes all four; `placed`; the design members above; flags `RF_PROTECTED`, `RF_PATROL`, `RF_OWNED` follow the update both ways, `RF_GANGHOUSE` is only cleared | `monsters[]`, `nummonsters`, floor `items`/`itemuses`/`itemcounts`, floor and hidden coins, `owner`, `lastkill`, `foundmask`, `areacount`, `searchers`, `hidcounts`, the other flag bits (`RF_SPECALIVE`), `unk_0x008`, `unk_0x5F0`, `unk_0x5F5` |
| room hidden items | | each hidden item of the update that cannot be picked up (`item.gettable` 0, the item table as patched so far) goes into the first free hidden slot with its uses unless the room has it; while looking, slots whose item no longer exists or is gettable with `limit` 0 are emptied (uses -1); the search stops at the slot holding the item | gettable hidden items of the update are not added |
| shop | each member on its own | `name`, `desc[3]`; `minlevel`, `maxlevel`, `markup`, `ownable`, `expandable`, `trainclass` together; a new `type` takes the whole stock list with quantities; per slot a different item brings its `qty`, while `maxqty`, `regentime`, `regennum`, `regenpct` are copied alone | `qty` of an unchanged slot; `number`, `dirty`, `unk_0x1dc`; a gang shop (in the update or the game) entirely |
| item | `abilities`/`abilvals`, `classes`, `races`, `negates`; `name`, `desc[6]`, `hiddesc[3]` and every other definition member | all compared members | `number`, `active` (copies in play), `unk_0x008`, `unk_0x3bf`, `dirty` |
| spell | every byte | the whole record | nothing |
| monster | abilities, items, attacks, spells; `name`, `desc[4]` and every other design member | all compared members | `number`, `curalive`, `killdate`, `killtime` (the schema's `killed_at`), `unk_0x004`, `dirty` |
| message | `msgs[3]` | `msgs[3]` | `number`, `modified`, `unk_0x0FB` |
| action | none | the whole record | an action whose `targets` has `AT_KEEP` (0x40) entirely |
| text block | the bytes the record carried (the original: all 2022; see below) | those bytes (the original: the whole record) | the bytes the record did not carry |

The merge "rewritten" count in the report is what the original marks for writing back (`modified`, `dirty`): a room with any
hidden item in the update is always rewritten, whether or not a value differs.

**What the stock stream changes.** The stock `WCCUPDAT.DAT` holds 38,754 records, one image
of every record of the stock `.VIR` set (13 races, 15 classes, 26,720 rooms, 178 shops, 1,950
items, 1,379 spells, 1,101 monsters, 3,867 messages, 3,467 text blocks, 64 actions) and nothing
new, and each image equals its virgin record byte for byte but for the text blocks' last 12
bytes. So the pass changes (`tests/patched_fields.json`, patched against unpatched):

- `exitpar1s` of 4 rooms: doors relocked to 2 in rooms the merge rewrites (the start-up preload
  relocks them too);
- 21 virgin gang shops are left alone; nothing is inserted.

**Text blocks: a deliberate deviation [I].** 110 stock text blocks have text running past
`text[1988]`, which the 2010 bytes a 2015-byte record carries do not hold. Taken
literally, the original's text update compares all 2022 bytes, finds these blocks different
and writes the update record over them, shortening their text to 1988 characters. How many
bytes that write stores on the variable-length `WCCTEXT` is not known, and T-006's export
holds the 110 blocks unshortened. The importer therefore compares and copies only the bytes the record carried and
keeps the rest of the stored block (`merge_text`), so the stock stream changes no text block
and the patched import equals the export. Decided as option (b) of silvermere#25 (no answer
by its deadline, so its stated default applied); it is inferred, not confirmed.

**Not carried**: the pacing (`UPDATE_SPEED`), the sysop menu update and its screens
(updating, reindexing, cross-referencing), the progress spinner, the cross-check against the
host's user accounts, and the open patch file. The post-update rebuilds are not persisted at
import: the monster quick references (rebuilt again 60 s after the update) are derived when
the engine loads (T-035), and the limited-item reindex (the `WCCITOWN` scans) is recomputed
from the final
content's `item.limit` and placed copies (T-030/T-041).

## JSON export

`--json OUT` writes `OUT/<TABLE>.jsonl` (one per file: `WCCMP001.jsonl`, `WCCITEMS.jsonl`, ...),
one object per record in reader order, in the shape of T-006's export (the export MBBSEmu's
database gave for the stock world), so the two compare line by line.

- `_id` is the reader's 1-based ordinal: records in logical-page order and slot order within
  a page (T-007), which is MBBSEmu's `data_t.id` and T-006's `_id`. `_raw_len` is the
  record length (2022 for text blocks).
- Every other key is the record's member name. Keys are sorted (T-006 writes them in
  declaration order; compare by key). Field-name mapping:

| This export | T-006 | Note |
|---|---|---|
| every member, by member name | same name | integers by member type; texts trimmed; arrays in slot order |
| `unk_*` (all of them) | only those T-009 needs | byte spans as base64, integer members as integers |
| monster `cash` | `cash` | raw order, runic first |
| monster `cash_copper_first` | `cash_copper_first` | `cash` reversed |
| text block `text_raw` | `text_raw` | the 2000 stored bytes, base64, still shifted |
| text block `text` | (none) | the decoded text (rule above) |
| player `gang`, `suicpw` | byte arrays | texts in the record layout, so strings here |

## Equivalence with T-006

`tests/equivalence.rs` decodes each table and compares every record with the export row of the
same `_id` on every common field. The tree holds no game records, so it compares against the
export's digests, `crates/silvermere-import/tests/fixtures/world/DIGEST.json` (per table, the
records compared and a sha256 per compared field; `tests/support/world_digest.rs`), and a wrong
value names its table and field. `MMUD_WORLD_JSON` names an export directory to compare against
record by record, and `UPDATE_WORLD_DIGEST=1` rewrites the digests from it. On the stock set every content field of every table is equal. The export
was taken from an instance that had started, so its run-time fields differ from the virgin
`.VIR` and are reported, not compared: `items.active`, monster `curalive`/`killdate`/
`killtime`, the room members listed above, the `RF_SPECALIVE` bit of room `flags`, and
`exitpar1s` of door and gate exits (the start-up preload relocks them to 2). `WCCACMSR` (795 rows) and `WCCITOWN` (37 rows) exist only in the export:
they are run-time state and empty in the virgin set.

`tests/post_update_equivalence.rs` makes the same comparison on the patched content (the
stock `.VIR` set plus `WCCUPDAT.DAT`, unpacked from the committed `WCCUPDAT.DAT.gz`), with no
allowance for patched fields: every content field of every table, rooms included, is equal
(the text blocks through the deviation in "Patches"). It also checks the `PatchReport` (38,754 records, seqnum 1..38,754),
`tests/patched_fields.json` (what the stream changes; `UPDATE_PATCHED_FIELDS=1` rewrites it)
and the idempotence of the pass.
