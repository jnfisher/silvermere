# Text rendering contract (`silvermere-text`)

What a telnet client receives for a game message, and why. The crate reproduces the bytes
the original module and its Worldgroup host sent together: the module built each message and
handed it to the host (`btuxmt`); the host then resolved if-ANSI strings, normalised line ends,
word-wrapped at the `btutsw` width and escaped `0xFF`. In
silvermere the module's part is `render::prf` and the host's part `render::finish`; `render`
does both, and the transport (T-012) does the `0xFF` doubling.

## The pipeline

`Msg` (a catalogue entry with its arguments) goes through, in this order:

1. **Template.** The catalogue text with `{name}` arguments substituted (numbers and
   strings formatted with `Display`), `{@ANSI_*}` strings inserted as resolved bytes, and
   `{@COL_*}` colour rows replaced by `CLRTBL[row][palette]` for the player's palette.
   Text is encoded to CP437 here; ANSI bytes are ASCII and pass through.
2. **If-ANSI resolution.** `ESC [ [ <ansi> | <plain> ]` becomes `<ansi>`. Sessions are
   always ANSI, so the plain half (space/backspace pairs) is never sent. Catalogue text is
   already resolved; the step is for raw `.MSG` text and tests.
3. **Line ends.** `\r` as the original wrote it becomes `\r\n`; a lone `\n` too; `\r\n`
   stays; `\n\r` becomes one `\r\n` (the host's `FormatNewLineCarriageReturn`).
4. **Word wrap**, only for messages whose catalogue entry says `wrap = true` (or when the
   caller passes a width to `render_with`): see below.

The result is `Rendered { bytes, wraps }`. For a message the original printed as the
line-clear prefix, the text and `\r`, the bytes are exactly `1B 5B 37 39 44 1B 5B 4B` + CP437
text + `0D 0A`.

## The prefix

Nearly every message starts with `ESC[79D ESC[K` (`ANSI_CLRLINE`): cursor
79 columns left, erase the line. It wipes the prompt, which is printed without a newline.
The prefix is part of the catalogue text where the original printed it (`{@ANSI_CLRLINE}`),
not added by the renderer: messages such as `There is no exit in that direction!`
never had it. The prompt's `^A` terminator is not sent.

## ANSI strings

The original's 20 shared ANSI strings and the few more the catalogue needs are `pub const`
byte strings in `ansi.rs`, already resolved: `ANSI_LTRED` is `ESC[1;31m`. `NOCOLOR` is empty.
`ansi::by_name` looks one up by name; `tests/ansi_macros.rs` proves each raw literal resolves
to its constant and checks every string and the `CLRTBL` rows against the committed fixture
`tests/fixtures/ansi_facts.json`.

## Colour rows and palettes

`colour::CLRTBL` is 15 rows by 4 palettes. A row is one kind of text, `ColRow` (the
catalogue's `{@COL_*}` names); a column is the player's palette, `SET PALETTE 0-3`, as
`Palette`. `WCCMMUD.DOC`
describes the palettes as 0 medium brightness, 1 dim, 2 the brightest and 3 a combination of
0 and 1; the table is the truth. A new character starts on palette 0 and an out-of-range
stored value is repaired to 0 at entry.

Messages to a room or the game send `CLRTBL[row][palette]` only for rows up to `MAXCLR`
(14); `TELL_NOCOLOR` (0xFF) and anything above 14 send nothing (`colour::tell_colour`). The
original's table has 40 rows; rows 15-39 are empty and never read. Hit points are coloured by
`get_hp_colour`: above half the maximum `PromptVal`, above a quarter `HpHurt`, otherwise
`HpLow`, with integer division (hp above maxhp/2, above maxhp/4).

## Word wrap

The host wrapped at the width set by `btutsw`. The module set 79 (`WRAPCOLS`) around
specific displays (room and list displays, and realm entry) and 0 after them, so wrapping
is per display, never global. The catalogue marks the messages that wrap; the
default is no wrapping.

`wrap::wrap` ports MBBSEmu's `LineBreaker`: a line holds at most `width` printable
characters; when one more arrives, the line breaks at the last space (the space becomes
`\r\n`), or before the character when the line has no space. ANSI sequences do not count;
cursor moves `H`, `f`, `C`, `D`, `G` adjust the column; `\r` starts a new line; `\n` and DEL
are ignored; a backspace takes one column back. A line therefore never fills all 80 columns
(`MBBSEmu.Tests SessionBase_Tests.cs:82`). `wrap::Wrapper` keeps the line state across
pushes for callers that assemble a line from several messages.

## CP437

Internal text is UTF-8; the wire is CP437, one byte per column. `cp437::CP437_TO_UNICODE`
is MBBSEmu's `CP437Converter.cs` table (the standard mapping for 0x80-0xFF; 0x00-0x7F pass
through) with that converter's UTF-8-mode exception kept: DC1-DC4 (0x11-0x14) render as
`◄ ↕ ‼ ¶`, because the full-screen editor draws with those bytes. `encode` maps every
character of the table back to its byte and anything else to `?`; `decode` is the UTF-8
session mode of the transport.

## Other pieces

- `component::pick` picks one alternative of `"a|b|c"` with
  `Which::Index(n)` (1 up) or `Which::Random` (`genrdn(0,cnt)`, uniform `0..cnt`), at most
  `MAXALTS` (5) alternatives, the result cut at `COMPSIZ-1` (39) characters, `None` text
  giving `"N/A"`. An index past the last alternative gives an empty string and a warning.
- `strip::strip_ansi` is the original's `stpans()` for logs and tests, with the pattern
  registry's regex: `ESC [ [0-9;?]* letter`, `ESC ( x` / `ESC ) x`, and any other `ESC` with
  the byte after it.
- Not carried: text variables (`\x01NAME\x01`), pause markers 0x13/0x14, `btuoes`,
  `stp4cs`, split-screen, `senddpk`, the `^A` prompt terminator.
- The `game_full` entry keeps only `We're sorry, but the game is currently full.`; the
  original's second line is not in the catalogue.

## The catalogue

`messages/catalogue.toml` holds one entry per message the original printed:

```toml
[messages.exp_gain]
text = "{@ANSI_CLRLINE}You gain {exp} experience.\r"   # the bytes the original printed
wrap = false                                           # true: 79-column wrap (btutsw)
note = "…"                                             # what it is for (the generated doc comment)
pattern = "exp_gain"                                   # name in tests/fixtures/patterns.json
args = { exp = "i64" }                                 # placeholder types (default String)
example = { exp = "13" }                               # values for the snapshot test
```

`build.rs` generates the `Msg` enum from it (`Msg::ExpGain { exp: 13 }`), so a key typo is a
compile error. Format codes of the original's texts (`%d`, `%ld`, `%s`, widths) are translated when
an entry is written, never interpreted at render time. `tests/patterns_snapshot.rs` renders
every entry with its example arguments, strips ANSI and checks it against the pattern
registry's regex and literal example, and pins the bytes in an `insta` snapshot; entries
later cards add are covered as soon as they name a `pattern`.

The help, `WCCMMUD.MSG` and `WCCTEXT.MSG` texts are read from the assets directory at run time.
In them `%s`/`%c` take their argument and `%%` is a percent sign; any other `%` is kept as
written (`185% ` prints `185% `), and the load logs one warning per file with the count.

## Prompt and statline

`prompt::render_prompt` builds the prompt. It always starts with `ESC[79D ESC[K` and
`CLRTBL[Prompt][palette]`, never ends with a newline, and does not send the original's `^A`
terminator. What follows depends on the player record's `statline` member (`StatlineMode`):

| mode | prints |
|---|---|
| `Off` 0 | `>` |
| `On` 1, `Full` 2, `maxmana` 0 | `[HP=48]:` |
| the same, with mana and a class record | `[HP=32/MA=22]:`, `KAI` when the class magery is `Magery::Kai` |
| the same, with mana and no class record | nothing after the colour prefixes (a warning is logged) |
| 3 and up (`Custom`, `FullCustom`) | the player's template through `statline::substitute` |

The resting or meditating mark (` (Resting) `, ` (Meditating) `, resting first) sits in
two different places:

- without mana, **inside** the brackets: `[HP=29 (Resting) ]:`;
- with mana, **after** `]:`: `[HP=7/MA=19]: (Resting) `.

The bytes of the mana form are `[HP=` + `get_hp_colour` + hp + `CLRTBL[2]` + `/MA=` +
`CLRTBL[3]` + mana + `CLRTBL[2]` + `]:` + mark. A sysop in debug mode gets
`{Energy=1234 Snk JstEnt Hdn }:` after the standard line only.

### Custom templates

`statline::substitute` expands a template; the social actions (T-046) use it too, with
`%t`. `SET STATLINE` stores at most 60 characters (`STATLINELEN`); `render_prompt` cuts
again before expanding. Expansion stops once 196 characters are written (`SUBSTMAX`), but
the piece being copied finishes.

| code | gives |
|---|---|
| `%h` | hit points; after `%d`, in the hit point colour followed by the prompt colour |
| `%H` | maximum hit points |
| `%m` / `%M` | mana (or kai) / maximum |
| `%x` | experience, the 32-bit total |
| `%X` | the experience needed for the next level (race exp + class exp as the modifier) minus exp, floored at 0; a bare `X` without the records |
| `%c` | wealth in copper farthings |
| `%i` | first name |
| `%t` | the caller's title, or `nothing` |
| `%p` / `%P` | `his`/`her`/`its` / `him`/`her`/`it` |
| `%r` | ` (Resting) `, ` (Meditating) ` or nothing |
| `%w` | `On`/`Off`, the warn-before-evil setting |
| `%n` | `\r` (sent as `\r\n`) |
| `%d` | the prompt colour; colours later `%h` |
| `%B` `%N` `%U` `%L` `%R` | `ESC[1m` `ESC[0m` `ESC[4m` `ESC[5m` `ESC[7m` |
| `%f0`-`%f7` | `ESC[30m`-`ESC[37m`; clears the `%h` colouring |
| `%b0`-`%b7` | `ESC[40m`-`ESC[47m`, but `%b1` is `ESC[42m` (sic); clears the `%h` colouring |
| `%%`, `%f%`, `%b%` | two percent signs |
| anything else | the pending `%`/`%f`/`%b` and the character: `%z`, `%f9` |
| a code letter in the wrong state | the character alone: `%5` gives `5`, `%fh` gives `h` |

## The menu and the entry screen

`menu::render_menu` prints `MENUTEXT` with the version (`MUDVER`, from config), then the
`[CHANGERACE]` line with `[N game minutes left]` or `[Last chance]` while the race change is
open, the `[SYSOP]` line for sysops, the tournament notice and `MENUPMPT`, `[MAJORMUD]: `
with no newline. The add-on menu hooks are not carried. `render_entry_banner` is `USRFOLED`
and `render_login_greeting` is `USRLOGIN` with `SHORTVER`. Their texts are in
`messages/prompt.toml` with the prompt's pieces; the bytes are those of the T-005 capture
`entry-existing`, which `tests/menu.rs` compares against.

## The game's long text, from the package

The long, prose-like text is not in the repository: the server reads it from the downloaded
package when it starts (T-138; `silvermere-text`'s `package` module). The import copies the
module's three message files into `<data_dir>/assets/` and writes the long screens it takes
out of `WCCMMUD.DLL` into `DLLTEXT.TXT` there ([import.md](import.md#assets)); a data directory
without them gets them from the cached package at start-up. `PackageText::load` parses them and
`package::install` makes them the process's text:

| From | What | Used by |
|---|---|---|
| `WCCMMUD.MSG` | the six text blocks `usrlogin`, `usrlogof`, `usrfoled`, `usrexit`, `leavemnu`, `inachup` (`USRLOGOF` and `LEAVEMNU` are unused) | the exit and inactivity texts |
| `WCCTEXT.MSG` | the six text blocks `goodtext`, `goodpmpt`, `chsrace`, `chsclass`, `menutext`, `menupmpt`, and the full-screen forms `EDITCHA1` and `EDTCHAN2` | `goodtext`; the full-screen character editor (T-063) |
| `WCCMMHLP.MSG` | the overview (`HL1B0`) and 131 topics | HELP |
| `DLLTEXT.TXT` | the long catalogue screens of `package::MOVED_KEYS` | the main menu's About, Information and Counts screens (ANSI and plain), the SYSOP help and menu, the line-mode chart, and a few long notices |

The moved catalogue entries keep their keys and arguments in `catalogue.toml`, so `Msg` still
has them; their catalogue text is the notice `package::NOT_INSTALLED`, which is what they print
while no text is installed. Once it is, the renderer takes their template from the package.
HELP without the text prints the same notice (the built-in topics still work), and character
creation uses the line-mode editor. The short messages stay in `catalogue.toml` and
`prompt.toml`.

The release guard (`tools/release/moved-text-guard.py`) checks that the published tree holds
none of this text, and `crates/silvermere-text/tests/package_text.rs` that what the package
gives is byte for byte the text the repository held before (hashes of every help topic, text
block, form and rendered screen). Tests compare against the text loaded from the stock package,
and snapshots replace it with `[package text]` (`PackageText::redact`).

The numeric options of `WCCMMUD.MSG` are not text; they are in `docs/config-defaults.md`.
`prompt.toml` still carries its own copies of the four short texts the menu and entry screen
print, and `tests/msg_catalogue.rs` checks that they equal the text blocks the package gives.

**Escapes and line ends.** In the `.MSG` text `~}` is a literal `}` and `~~` a literal `~`
(so `MENUTEXT` writes the world name as `{Realm Of Legends~}`). The reader drops every CR
and turns **every LF into the catalogue's `\r` line end**; the renderer's `finish` then makes
`\r` into CR LF. This differs from the MBBSEmu compiler (`MsgFile.cs`), which keeps an LF only
before an alphanumeric, `%`, `(` or `"`: the T-005 capture `entry-existing` shows the line end
kept before a space and before ESC (` M A J O R  M U D` after the clear screen; the blank lines
of `USRLOGIN`), and only "keep every LF" reproduces its bytes. `tests/msg_catalogue.rs` renders
`usrlogin`, `usrfoled`, `menutext` and `menupmpt` and compares them with the capture bytes.
A literal `{` or `}` in a text becomes `{{` or `}}`.

**Arguments.** `USRLOGIN` and `MENUTEXT` each take one `%s`, written `{version}`: the short
version (`1.11p-WG`) for the greeting, the full one (`1.11p-WG (Dec 30 2005 14:20:05)`) for the menu. The
server supplies the original's strings by default (`compat.version_string`). In the help
topics the texts' two `%c` are `{talk_char}` and `{yell_char}`, the say and yell characters.

**Help topics.** The file is pairs of messages: a topic string (`HL1Tn`, the words) and the
message after it, which is what the original shows for the topic. The names do not always
agree (`HL1T60` is followed by `HL1B61`); the loader pairs by position and keeps both names
(`key`, `body_key`). `package::help_topics` reads the file and `HelpTopics::validated` checks
it. There are 131 topics;
the "263" of the card is the number of messages in `WCCMMHLP.MSG` (`HL1B0`, 131 topic strings,
131 texts). The last topic, `Help ?` (`HL1T9999`), is the end marker (`HELP_END_MARK`) and
stays as the last row. Matching, which T-027 implements: the player's topic is compared with
every topic string in file order by a match that accepts the beginning (any case) of any word
of the string; the first match shows its text; reaching the end marker without a match is "no
such topic". `HELP TOPICS` lists the first word of each string, wrapped at 79 columns, ending
with `Help ?.`. `HELP` alone prints the overview. The built-in topics after the walk
(experience table and the rest) are code, not data.
