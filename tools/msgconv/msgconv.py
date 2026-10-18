#!/usr/bin/env python3
"""msgconv: convert the MajorMUD .MSG files into silvermere's checked-in data.

A development tool, not shipped; standard library only. Run it with `python3 -I`:

    msgconv.py --selftest
    msgconv.py --msg WCCMMUD.MSG --catalogue crates/silvermere-text/messages/wccmmud.toml \
               --options docs/config-defaults.md
    msgconv.py --msg WCCTEXT.MSG --catalogue crates/silvermere-text/messages/wcctext.toml \
               --assets crates/silvermere-text/messages/assets
    msgconv.py --msg WCCMMHLP.MSG --help-topics crates/silvermere-text/messages/help_topics.toml

The .MSG format (one entry per `KEY {value} TYPE ...`; `~}` is a literal `}` and `~~` a
literal `~`; the prose between entries describes the next one) is the MajorBBS message
file. `tools/msgconv/README.md` says how the outputs map onto the catalogue
(crates/silvermere-text/src/catalogue.rs) and docs/config-defaults.md.

Line ends: the files carry CR LF; the CRs are dropped and every LF becomes the `\\r`
line end the catalogue uses (the renderer turns `\\r` into CR LF). This is settled by the
T-005 capture `entry-existing`, which shows the LF kept before a space or an ESC; the
narrower rule of the MBBSEmu compiler (LF only before alphanumerics, `%`, `(`, `"`) does
not reproduce it. docs/text.md records the rule.
"""

import argparse
import hashlib
import os
import re
import sys

ENC = "cp437"  # the files are DOS text; high bytes are CP437 box drawing


# --------------------------------------------------------------------------- parsing


class Entry:
    """One `KEY {value} ...` entry of a .MSG file."""

    def __init__(self, key, line, value, spec, prose):
        self.key = key
        self.line = line  # 1-based line of the key
        self.value = value  # unescaped, CR-free
        self.prose = prose  # the sysop-facing description before the entry
        toks = spec.split()
        self.kind = toks[0] if toks else ""  # T N S B C E, or "" (LEVEL/FILE markers)
        self.lo = self.hi = self.length = None
        self.choices = []
        self.label = ""  # the question text of N/B/E entries
        self.default = value
        self.note = ""  # the description after the type letters
        if self.kind == "N":
            self.lo, self.hi = int(toks[1]), int(toks[2])
            head, _, last = value.rpartition(" ")
            self.label, self.default = head, last
        elif self.kind == "B":
            head, _, last = value.rpartition(" ")
            self.label, self.default = head, last
        elif self.kind == "E":
            head, _, last = value.rpartition(" ")
            self.label, self.default = head, last
            self.choices = toks[1:]
        elif self.kind == "S":
            self.length = int(toks[1])
            self.note = " ".join(toks[2:])
        elif self.kind == "T":
            self.note = " ".join(toks[1:])


def unescape(raw):
    """`~}` -> `}`, `~~` -> `~`; any other `~` is literal."""
    out = []
    i = 0
    while i < len(raw):
        if raw[i] == "~" and i + 1 < len(raw) and raw[i + 1] in "}~":
            out.append(raw[i + 1])
            i += 2
        else:
            out.append(raw[i])
            i += 1
    return "".join(out)


def escape(value):
    """The inverse of `unescape` for text placed inside `{...}`."""
    return value.replace("~", "~~").replace("}", "~}")


KEY_RE = re.compile(r"^([A-Z][A-Z0-9_]*)[ \t]*\{", re.M)


def parse_msg(data):
    """Parses a .MSG file (bytes) into a list of `Entry`."""
    text = data.decode(ENC).replace("\r", "")
    entries = []
    pos = 0
    while True:
        m = KEY_RE.search(text, pos)
        if not m:
            break
        i = m.end()
        buf = []
        while i < len(text):
            c = text[i]
            if c == "~" and i + 1 < len(text) and text[i + 1] in "}~":
                buf.append(text[i + 1])
                i += 2
            elif c == "}":
                break
            else:
                buf.append(c)
                i += 1
        else:
            raise ValueError(f"{m.group(1)}: unterminated value")
        eol = text.find("\n", i)
        eol = len(text) if eol < 0 else eol
        spec = text[i + 1 : eol]
        prose = " ".join(text[pos : m.start()].split())
        line = text.count("\n", 0, m.start()) + 1
        entries.append(Entry(m.group(1), line, "".join(buf), spec, prose))
        pos = eol
    return entries


# ----------------------------------------------------------------------- conversion


def to_template(value, args=()):
    """Turns an unescaped .MSG value into a catalogue template.

    LF -> `\\r`; `{`/`}` -> `{{`/`}}`; `%s`/`%c` -> `{arg}` taken in order from `args`;
    `%%` -> `%`. Any other printf conversion is an error.
    """
    args = list(args)
    out = []
    i = 0
    while i < len(value):
        c = value[i]
        if c == "\n":
            out.append("\r")
        elif c == "{":
            out.append("{{")
        elif c == "}":
            out.append("}}")
        elif c == "%":
            nxt = value[i + 1] if i + 1 < len(value) else ""
            if nxt == "%":
                out.append("%")
            elif nxt in ("s", "c") and args:
                out.append("{" + args.pop(0) + "}")
            else:
                raise ValueError(f"unhandled printf directive near {value[i:i+4]!r}")
            i += 1
        else:
            out.append(c)
        i += 1
    return "".join(out)


def toml_str(s):
    """A TOML basic string, ASCII only."""
    out = ['"']
    for c in s:
        o = ord(c)
        if c == "\\":
            out.append("\\\\")
        elif c == '"':
            out.append('\\"')
        elif c == "\r":
            out.append("\\r")
        elif c == "\n":
            out.append("\\n")
        elif c == "\t":
            out.append("\\t")
        elif o < 0x20 or o == 0x7F:
            out.append("\\u%04X" % o)
        elif o > 0x7E:
            out.append("\\u%04X" % o if o <= 0xFFFF else "\\U%08X" % o)
        else:
            out.append(c)
    out.append('"')
    return "".join(out)


# What each entry becomes in wccmmud.toml / wcctext.toml: key -> (pattern, args placeholders
# in printf order, example, note)
WCCMMUD_TEXTS = {
    "USRLOGIN": (
        "login_greeting",
        ["version"],
        {"version": "1.11p-WG"},
        "The logon greeting. The one argument is the bare version (`1.11p-WG`).",
    ),
    "USRLOGOF": (
        None,
        [],
        {},
        "Disabled in the original (the file's own comment: it 'has been disabled and will be "
        "removed in a future version'); kept for completeness, unused.",
    ),
    "USRFOLED": (
        "enter_banner",
        [],
        {},
        "Clear screen, the `MajorMUD - ROL` banner and the 77-character `=-` rule.",
    ),
    "USREXIT": (
        None,
        [],
        {},
        "Shown when the player leaves the game.",
    ),
    "LEAVEMNU": (
        None,
        [],
        {},
        "Sub-board hook: the original sends these characters to disconnect from a sub-board. "
        "A single space and a line end; unused in silvermere.",
    ),
    "INACHUP": (
        "inactive_hangup",
        [],
        {},
        "Three spaces before `Bye!` are in the original.",
    ),
}

WCCTEXT_TEXTS = {
    "GOODTEXT": (
        None,
        [],
        {},
        "The Lawful explanation. The game prints it before GOODPMPT.",
    ),
    "GOODPMPT": (
        None,
        [],
        {},
        "Two spaces before `[Yes/No]` are in the original; a space follows it.",
    ),
    "CHSRACE": (
        None,
        [],
        {},
        None,
    ),
    "CHSCLASS": (
        None,
        [],
        {},
        None,
    ),
    "MENUTEXT": (
        "mud_title",
        ["version"],
        {"version": "1.11p-WG (Dec 30 2005 14:20:05)"},
        "The main menu. The one argument is the full version string; `{Realm Of Legends}` is literal text "
        "(the file writes it `{Realm Of Legends~}`).",
    ),
    "MENUPMPT": (
        "mud_menu_prompt",
        [],
        {},
        "The `[MAJORMUD]: ` prompt; no line end follows.",
    ),
}

# The two full-screen-display templates of character creation: kept as
# assets for T-063 (character creation), not catalogue entries.
FSD_TEMPLATES = ("EDITCHA1", "EDTCHAN2")

HELP_ARGS = ["talk_char", "yell_char"]


def sources_header(msg_name, script_note):
    return (
        f"# Generated from {msg_name} by tools/msgconv/msgconv.py{script_note}; do not edit by hand.\n"
        "# Format: crates/silvermere-text/src/catalogue.rs. Line ends are `\\r`; `{{` and `}}` are\n"
        "# literal braces; ESC is \\u001B.\n"
    )


def write_catalogue(entries, texts, msg_name, drop=()):
    by_key = {e.key: e for e in entries}
    out = [sources_header(msg_name, " --catalogue"), ""]
    count = 0
    for key, (pattern, args, example, note) in texts.items():
        e = by_key[key]
        if e.kind != "T":
            raise ValueError(f"{key} is not a text block")
        out.append(f"[messages.{key.lower()}]")
        out.append(f"text = {toml_str(to_template(e.value, args))}")
        if pattern:
            out.append(f"pattern = {toml_str(pattern)}")
        if example:
            items = ", ".join(f"{k} = {toml_str(v)}" for k, v in example.items())
            out.append(f"example = {{ {items} }}")
        if note:
            out.append(f"note = {toml_str(note)}")
        out.append("")
        count += 1
    return "\n".join(out), count


def write_assets(entries, outdir):
    os.makedirs(outdir, exist_ok=True)
    written = []
    for e in entries:
        if e.key in FSD_TEMPLATES:
            lines = [
                f"# Generated from WCCTEXT.MSG:{e.key} by tools/msgconv/msgconv.py; do not edit by hand.",
                "# The full-screen-display template for character creation (T-063): `$$$$` and",
                "# `????` are the field placeholders of the full-screen display.",
                "# Text is the CP437 file text decoded to Unicode; lines end with `\\r`.",
                "",
                f"text = {toml_str(e.value.replace(chr(10), chr(13)))}",
                "",
            ]
            path = os.path.join(outdir, f"{e.key.lower()}.toml")
            with open(path, "w", newline="\n") as f:
                f.write("\n".join(lines))
            written.append(path)
    return written


def write_help_topics(entries):
    """The help file is pairs of messages: `HL1Tn` (the topic words) and the message that
    follows it (`HL1Bn`), by position, which is how the game pairs them. Names may
    disagree: HL1T60 is followed by HL1B61."""
    heads = [e for e in entries if re.fullmatch(r"HL1[TB]\w+", e.key)]
    if not heads or heads[0].key != "HL1B0":
        raise ValueError("HL1B0 (the overview) must come first")
    out = [
        "# Generated from WCCMMHLP.MSG by tools/msgconv/msgconv.py --help-topics; do not edit by hand.",
        "# `overview` is HL1B0; each [[topic]] is an HL1Tn message and the message after it in file",
        "# order. `words` is the topic string the HELP command matches; `text` is the body, a catalogue template whose `{talk_char}` and",
        "# `{yell_char}` are the say and yell characters (the file's two `%c`). `body_key` is the",
        "# message the text came from. The last row, `Help ?`, is the end marker.",
        "",
        f"overview = {toml_str(to_template(heads[0].value))}",
        "",
    ]
    rest = heads[1:]
    if len(rest) % 2:
        raise ValueError("topic and body messages do not pair up")
    count = 0
    for t, b in zip(rest[0::2], rest[1::2]):
        if not t.key.startswith("HL1T") or not b.key.startswith("HL1B"):
            raise ValueError(f"{t.key}/{b.key} is not a topic/body pair")
        if b.value.count("%c") > len(HELP_ARGS):
            raise ValueError(f"{b.key} has too many %c")
        out.append("[[topic]]")
        out.append(f"key = {toml_str(t.key)}")
        out.append(f"body_key = {toml_str(b.key)}")
        out.append(f"words = {toml_str(t.value)}")
        out.append(f"text = {toml_str(to_template(b.value, HELP_ARGS))}")
        out.append("")
        count += 1
    return "\n".join(out), count


# ------------------------------------------------------------- option documentation

# `.MSG` options. name: (disposition, [game] key, module default, GameConfig default
# (T-017; as TOML), note)
# disposition: keep | role | drop | not-read
K, R, D, N = "keep", "role", "drop", "not-read"
MSG_OPTIONS = {
    "ACTIVATE": (D, None, "(none)", None, "Activation code. No activation in silvermere."),
    "GAMCRD": (D, None, "0", None, "Credits per minute. No credit billing."),
    "REALCRD": (D, None, "60", None, "Credits per minute for everyone. No credit billing."),
    "FREEMIN": (D, None, "0", None, "Free minutes for exempt users. No credit billing."),
    "SYSOPKEY": (R, None, "(none)", None, "Role name `sysop` (T-013). Shipped key `wccsysop`."),
    "GAMOPKEY": (R, None, "(none)", None, "Role name `gameop` (T-013). Shipped key `wccgameop`."),
    "LOANKEY": (D, None, "(none)", None, "Key to take bank loans. Loans were guaranteed with BBS credits; no credits here."),
    "PLAYKEY": (D, None, "(none)", None, "Key to play at all. Gating is by account, not by BBS key."),
    "NOPLAYKY": (D, None, "(none)", None, "Key that bans a user. An account flag does this (T-013)."),
    "SAVEKEY": (D, None, "(none)", None, "Key to save a character. Every character saves."),
    "NODMOKEY": (D, None, "(none)", None, "Key to pass DEMOLVL. No demo mode."),
    "NOKICKEY": (D, None, "(none)", None, "Key against low-credit exits. No credits."),
    "GOSSKEY": (R, None, "(none)", None, "Role name `gossip-ban` (a key that removes gossip; T-013 maps it to a muted-channel flag)."),
    "AUCTKEY": (R, None, "(none)", None, "Role name `auction-ban` (a key that removes auction; T-013 maps it to a muted-channel flag)."),
    "NOPAYKEY": (D, None, "(none)", None, "Key against credit charges. No credits."),
    "SECOTH": (K, "sec_oth", "60", "160", "Seconds between player saves. The game rounds it down to whole minutes, minimum 60."),
    "SECBUFF": (D, None, "60", None, "Seconds between buffer saves. There are no buffers."),
    "BUFTOSAV": (D, None, "5", None, "Buffers saved per pass. There are no buffers."),
    "ROOMBUF": (D, None, "2000", None, "In-memory buffer size (rooms). The database replaces buffers."),
    "ITEMBUF": (D, None, "450", None, "In-memory buffer size (items)."),
    "MONSBUF": (K, "buffers.monsbuf", "300", "300", "The active-monster cache holds MONSBUF + 1 slots, and the background passes walk it (T-086), so it decides which monsters move, regenerate and refill their energy. At start-up the original lowers it in steps of 10, while it is above 25, until the cache fits in the free memory: under MBBSEmu it ends at 20 (21 slots), and the differential sets that value (T-040)."),
    "KMONSBUF": (D, None, "200", None, "In-memory buffer size (known monsters)."),
    "SPELBUF": (D, None, "200", None, "In-memory buffer size (spells)."),
    "RACEBUF": (D, None, "12", None, "In-memory buffer size (races)."),
    "CLASSBUF": (D, None, "16", None, "In-memory buffer size (classes)."),
    "BANKBUF": (D, None, "25", None, "In-memory buffer size (bankbooks)."),
    "MESSBUF": (D, None, "300", None, "In-memory buffer size (messages)."),
    "SHOPBUF": (D, None, "49", None, "In-memory buffer size (shops)."),
    "GANGBUF": (D, None, "35", None, "In-memory buffer size (gangs)."),
    "TALKCHAR": (K, "talk_char", "'.'", "\".\"", "Prefix that makes a line speech when the profile is SLOW talking."),
    "YELLCHAR": (K, "yell_char", "'\"'", "'\"'", "Prefix that makes a line a yell."),
    "USRCURNM": (K, "usr_cur_name", "(none)", "\"runic coins\"", "Name of the top currency. 1 to 28 characters."),
    "DEATHHP": (K, "death_hp", "-10", "-200", "Hit points at which the player dies. John decided `-200` ships (2026-10-14); the module default is `-10`. The game also tests for a value above 2000, which can never be true after the range clamp."),
    "XTRALVL": (K, "xtra_lvl", "1", "2", "Levels of experience a player may earn beyond the current one before training."),
    "PUNHUP": (K, "pun_hup", "HIGH", "\"none\"", "Punishment for hanging up: high, medium, low, none. Any other word counts as HIGH in the original; silvermere rejects it."),
    "INVGLO1": (K, "inv_glo", "(none)", "[\";x\", \";-\", \";re\"]", "First of ten disallowed global commands. `inv_glo` is a list of at most ten strings; an empty one is not stored."),
    "INVGLO2": (K, "inv_glo", "(none)", "[\";x\", \";-\", \";re\"]", "As INVGLO1."),
    "INVGLO3": (K, "inv_glo", "(none)", "[\";x\", \";-\", \";re\"]", "As INVGLO1."),
    "INVGLO4": (K, "inv_glo", "(none)", "[\";x\", \";-\", \";re\"]", "As INVGLO1."),
    "INVGLO5": (K, "inv_glo", "(none)", "[\";x\", \";-\", \";re\"]", "As INVGLO1."),
    "INVGLO6": (K, "inv_glo", "(none)", "[\";x\", \";-\", \";re\"]", "As INVGLO1."),
    "INVGLO7": (K, "inv_glo", "(none)", "[\";x\", \";-\", \";re\"]", "As INVGLO1."),
    "INVGLO8": (K, "inv_glo", "(none)", "[\";x\", \";-\", \";re\"]", "As INVGLO1."),
    "INVGLO9": (K, "inv_glo", "(none)", "[\";x\", \";-\", \";re\"]", "As INVGLO1."),
    "INVGLO10": (K, "inv_glo", "(none)", "[\";x\", \";-\", \";re\"]", "As INVGLO1."),
    "MAXTOP": (K, "max_top", "10", "10", "Longest TOP n / TOP n GANGS list."),
    "EVILKEEP": (K, "evil_keep", "100", "0", "Percentage of evil points kept by the next character."),
    "MAILUSER": (D, None, "(none)", None, "Mailink recipient of internal errors. No mail system; errors go to the log."),
    "PROFCHEK": (K, "profanity_check", "0", "false", "DECIDED: survives as `profanity_check`, default off (`false`). Filters profanity in `say`/`yell`/`gossip`-class lines. It is a cheap, optional sysop feature; the word list comes with the card that implements those commands. "),
    "INITRATE": (D, None, "100", None, "Rate of buffer initialisation. No buffers."),
    "POLLRATE": (D, None, "200", None, "Relative polling counter. The async runtime has no polling."),
    "DEMOLVL": (D, None, "99", None, "Highest level without NODMOKEY. No demo mode."),
    "EXPINTEN": (K, "exp_in_topten", "1", "true", "DECIDED: survives, default on. Shows experience in the TOPTEN list. One flag, no cost. "),
    "LVLDIFF": (K, "lvl_diff", "10", "9", "Largest level gap for PvP; -1 disables PvP. Tournament play forces 100."),
    "MINHUPHP": (K, "min_hup_hp", "25", "0", "Least percentage of maximum HP lost on a hang-up."),
    "MAXHUPHP": (K, "max_hup_hp", "50", "0", "Most percentage of maximum HP lost on a hang-up."),
    "MAXHUPIT": (K, "max_hup_items", "100", "0", "Most items lost on a hang-up."),
    "INACTIVE": (K, "inactive_secs", "3600", "3600", "Seconds without a command before the player is hung up on (INACHUP). John decided `3600` ships (2026-10-14); the shipped file has `900`."),
    "DATADIR": (D, None, "(empty)", None, "Location of the databases. `data_dir` and `database.url` replace it."),
    "KEEPEXP": (K, "keep_exp", "1", "false", "Keep some experience after the final death."),
    "LIGHTEVY": (K, "light_evy", "1", "true", "Whether SYSOP LIGHTNING may hit any player."),
    "GENDERGO": (K, "gender_gold", "-1", "0", "Gold charged to change gender; -1 forbids it, 0 is free."),
    "GANGEXP": (K, "gang_exp", "0", "0", "Gang experience (x1000) required to buy a gang house."),
    "DELUSER": (D, None, "1", None, "DECIDED: dropped. It decides whether deleting a BBS account deletes its MajorMUD character. In silvermere accounts own their characters, so deleting an account deletes them (the original's `YES`). The `NO` mode lets a new account that reuses the name inherit the old character, which is a BBS-ism with no use here."),
    "MAXEPDAY": (K, "max_ep_day", "12", "12", "Most evil points forgiven per day."),
    "EPCYCLE": (K, "ep_cycle", "60", "60", "Minutes per evil-point forgiveness cycle."),
    "EPAMOUNT": (K, "ep_amount", "1", "1", "Evil points forgiven per cycle."),
    "ENABLAWF": (K, "enable_lawful", "1", "true", "Whether players may choose Lawful. Tournament play forces it off."),
    "COINSWP": (N, None, "", None, "Has no effect on the game. Not carried."),
    "GCOINSW": (N, None, "", None, "Has no effect on the game. Not carried."),
}

# The 34 WCCMMUD.INI options:
# name: (disposition, [game] key, type, range, default, GameConfig default, note)
SYS_KEY_NOTE = "A BBS key a sysop or game-op also needs to run the command. silvermere maps it to a permission of the sysop/gameop role (T-013); no `[game]` key."
INI_OPTIONS = {
    "TRACK_LIMITED": (K, "track_limited", "int", "0, 1 (any number read)", "0", "false", "Log every move of a limited item."),
    "LOGFILE": (D, None, "string", "file name", "WCCMMUD.LOG", None, "Activity log file. silvermere logs through `tracing` (`[logging]`)."),
    "EXP_IN_TOPGANG": (K, "exp_in_top_gang", "int", "0, 1", "0", "false", "Show each gang's experience in TOP GANGS."),
    "LOG_SYSOP_COMMANDS": (K, "log_sysop_commands", "int", "0, 1", "1", "true", "Log the sysop commands used in the game."),
    "TOURNAMENT_PLAY": (K, "tournament_play", "word", "`on` or anything else", "off", "false", "Only `on` turns it on. Taken on the first read only: changing it needs a restart. On, it forces `enable_lawful` off and `lvl_diff` to 100 and prints the tournament line on the menu. The original also needs the 'A Call to Arms' add-on."),
    "UPDATE_SPEED": (D, None, "int", "0..5", "0", None, "Speed of the automatic database updates. There is no update database; T-019 replaces it."),
    "SYS_MAP_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: SYS MAP."),
    "SYS_REPORT_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: SYS REPORT."),
    "SYS_GOTO_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: SYS GOTO."),
    "SYS_LIGHTNING_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: SYS LIGHTNING."),
    "SYS_STATUS_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: SYS STATUS."),
    "SYS_RESET_ROOMS": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: SYS RESET."),
    "SYS_ADDLIFE_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: GOD user ADD LIFE."),
    "SYS_RETRAIN_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: GOD user RETRAIN."),
    "SYS_ALIGNMENT_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Commands: GOD user GOOD/SAINT/NEUTRAL/LAWFUL/UNLAWFUL."),
    "SYS_EDITEVIL_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: GOD user ADD EVIL."),
    "SYS_JAIL_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: GOD user JAIL."),
    "SYS_SPEAK_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: GOD user SPEAK."),
    "SYS_LISTUSER_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Commands: LIST USERS/EVIL/GANG."),
    "SYS_LISTLIMIT_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: LIST LIMITED_ITEMS."),
    "SYS_USERINFO_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Commands: GOD user INVENTORY/PROFILE/SPELLS/POWERS/ABILITIES/INFO/STATUS."),
    "SYS_CHANGE_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Commands: GOD user CHANGE NAME/SEX."),
    "SYS_TOPTEN_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Commands: GOD user ENABLE/DISABLE TOPTEN."),
    "SYS_DISBAND_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: SYS DISBAND gang (a sysop command, not given to a game-op by default)."),
    "SYS_BUFFERS_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: SYS BUFFERS (no buffers in silvermere, so the command goes with them)."),
    "SYS_CLEARSUICIDE_KEY": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Command: GOD user CLEAR SUICIDE."),
    "MAXIMUM_BANK_BALANCE": (K, "maximum_bank_balance", "long", "0 = no limit; copper", "0", "0", "Any excess is removed from the account without warning (WCCMMUD.RLN)."),
    "ENABLE_CLASS_CHANGE": (K, "enable_class_change", "int", "0, 1", "0", "false", "Let characters from before 1.1u change class as well as race."),
    "BLANK_LASTNAME_ALLOWED": (K, "blank_lastname_allowed", "int", "0, 1", "0", "false", "Allow characters with no last name."),
    "RACE_CHANGE_DURATION": (K, "race_change_duration", "int", "1..720 minutes", "300", "300", "Minutes of play in which a race change stays open. The original clamps an out-of-range value to the nearest limit; silvermere rejects it at startup, like every other range (see below)."),
    "CODEC_OF_TWINKS": (K, "codec_of_twinks", "int", "any", "0", "0", "Undocumented. Developer code: when it equals 78 the sysop tools DEBUG, MONSTER, SUMMON, EXPRESS_TRAIN and TWEAK work. Kept as an integer, default 0. "),
    "SYS_LOG_DEATHS": (K, "sys_log_deaths", "int", "0, 1", "0", "false", "Log player deaths, rerolls and suicides. "),
    "KID_GLOVES": (K, "kid_gloves", "int", "0, 1", "0", "false", "Hand back the items or coins when an NPC purchase is paid wrongly (added in 1.11p)."),
    "SYS_DISABLE_GANGS": (R, None, "string", "up to 15 characters", "(empty)", None, SYS_KEY_NOTE + " Commands: SYS DISABLE/ENABLE gang."),
}

DISP_TEXT = {K: "keep", R: "role", D: "drop", N: "not read"}


def cell(s):
    return s.replace("|", "\\|")


def shipped(e):
    """The dist file's value of an option, as text for the doc."""
    return e.default


def options_tables(entries):
    by_key = {e.key: e for e in entries}
    missing = [k for k in MSG_OPTIONS if k not in by_key]
    if missing:
        raise ValueError(f"WCCMMUD.MSG lacks {missing}")
    keep, role, drop = [], [], []
    for name, (disp, toml, mdef, cfg, note) in MSG_OPTIONS.items():
        e = by_key[name]
        if e.kind not in ("N", "S", "B", "C", "E"):
            raise ValueError(f"{name}: unexpected kind {e.kind!r}")
        if e.kind == "N":
            rng = f"{e.lo}..{e.hi}"
            ty = "number"
        elif e.kind == "B":
            rng, ty = "YES, NO", "yes/no"
        elif e.kind == "E":
            rng, ty = ", ".join(e.choices), "token"
        elif e.kind == "C":
            rng, ty = "one character", "character"
        else:
            rng, ty = f"up to {e.length} characters", "string"
        ship = e.default if e.default != "" else "(empty)"
        if e.kind == "C":
            ship = repr(e.default)
        row = (name, toml, ty, rng, mdef, ship, cfg, note, disp, e.line)
        {K: keep, R: role, D: drop, N: drop}[disp].append(row)
    return keep, role, drop


def md_table(header, rows):
    out = ["| " + " | ".join(header) + " |", "|" + "|".join("---" for _ in header) + "|"]
    for r in rows:
        out.append("| " + " | ".join(cell(str(c)) for c in r) + " |")
    return "\n".join(out)


def code(s):
    return f"`{s}`" if s else ""


def render_options_doc(msg_entries, template):
    keep, role, drop = options_tables(msg_entries)
    # surviving .MSG options
    krows = []
    for name, toml, ty, rng, mdef, ship, cfg, note, disp, line in keep:
        krows.append(
            (
                code(name),
                code(f"game.{toml}"),
                ty,
                rng,
                code(mdef),
                code(ship),
                code(cfg) if cfg is not None else "(not in GameConfig yet)",
                f"WCCMMUD.MSG:{line}",
                note,
            )
        )
    kh = ["Option", "TOML key", "Type", "Range", "Module default", "Shipped value", "GameConfig default", "Defined at", "Notes"]
    rrows = []
    for name, toml, ty, rng, mdef, ship, cfg, note, disp, line in role:
        rrows.append((code(name), ty, rng, code(ship), f"WCCMMUD.MSG:{line}", note))
    rh = ["Option", "Type", "Range", "Shipped value", "Defined at", "Role mapping"]
    drows = []
    for name, toml, ty, rng, mdef, ship, cfg, note, disp, line in drop:
        drows.append((code(name), DISP_TEXT[disp], f"WCCMMUD.MSG:{line}", note))
    dh = ["Option", "Status", "Defined at", "Why"]
    # INI
    irows = []
    for name, (disp, toml, ty, rng, mdef, cfg, note) in INI_OPTIONS.items():
        key = f"game.{toml}" if toml else "(none)"
        irows.append(
            (
                code(name),
                DISP_TEXT[disp],
                code(key) if toml else key,
                ty,
                rng,
                code(mdef),
                code(cfg) if cfg is not None else ("(not in GameConfig yet)" if toml else ""),
                note,
            )
        )
    ih = ["Option", "Status", "TOML key", "Type", "Range", "Default", "GameConfig default", "Notes"]
    text = template
    text = text.replace("@@TABLE_KEEP@@", md_table(kh, krows))
    text = text.replace("@@TABLE_ROLES@@", md_table(rh, rrows))
    text = text.replace("@@TABLE_DROPPED@@", md_table(dh, drows))
    text = text.replace("@@TABLE_INI@@", md_table(ih, irows))
    text = text.replace("@@COUNT_MSG@@", str(len(MSG_OPTIONS) + 1))
    text = text.replace("@@COUNT_INI@@", str(len(INI_OPTIONS)))
    return text


# ------------------------------------------------------------------------- self test


def selftest():
    # ~} and ~~ round trip
    assert unescape("a~}b~~c~x") == "a}b~c~x"
    for s in ["plain", "a}b", "a~b", "~}", "}~", "{x~}~~", "~~~}"]:
        assert unescape(escape(s)) == s, s
    data = "K1 {a~}b~~c} T desc\r\nK2 {x\r\n y} T\r\n".encode(ENC)
    got = parse_msg(data)
    assert [e.key for e in got] == ["K1", "K2"]
    assert got[0].value == "a}b~c" and got[1].value == "x\n y", (got[0].value, got[1].value)
    # key, value and re-escaped text parse back to the same value
    rebuilt = "".join(f"{e.key} {{{escape(e.value)}}} T\n" for e in got).encode(ENC)
    assert [e.value for e in parse_msg(rebuilt)] == [e.value for e in got]
    # templates
    assert to_template("v%s {w}\nx", ["version"]) == "v{version} {{w}}\rx"
    assert to_template("100%%") == "100%"
    for bad in ("%d", "%s"):
        try:
            to_template(bad)
        except ValueError:
            pass
        else:
            raise AssertionError(bad)
    # TOML strings parse back
    try:
        import tomllib
    except ImportError:  # pragma: no cover
        tomllib = None
    sample = 'esc\x1b[0m "q" \\ \r\n\t \u00b3 end'
    if tomllib:
        assert tomllib.loads("x = " + toml_str(sample))["x"] == sample
    # typed entries
    t = parse_msg(b"DEATHHP {Hit points at which the user dies? -200} N -20000 0\r\nFLAG {Q? NO} B\r\n")
    assert (t[0].default, t[0].lo, t[0].hi) == ("-200", -20000, 0)
    assert t[1].default == "NO"
    print("msgconv selftest ok")


# ----------------------------------------------------------------------------- main


def write(path, text):
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with open(path, "w", newline="\n") as f:
        f.write(text if text.endswith("\n") else text + "\n")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--msg", help="the .MSG file to read")
    ap.add_argument("--catalogue", help="write the catalogue TOML (WCCMMUD.MSG or WCCTEXT.MSG)")
    ap.add_argument("--help-topics", help="write help_topics.toml (WCCMMHLP.MSG)")
    ap.add_argument("--options", help="write docs/config-defaults.md (WCCMMUD.MSG)")
    ap.add_argument("--assets", help="directory for the FSD templates (WCCTEXT.MSG)")
    args = ap.parse_args(argv)
    if args.selftest:
        selftest()
        return 0
    if not args.msg:
        ap.error("--msg is required (or --selftest)")
    with open(args.msg, "rb") as f:
        entries = parse_msg(f.read())
    name = os.path.basename(args.msg).upper()
    here = os.path.dirname(os.path.abspath(__file__))
    if args.catalogue:
        texts = {"WCCMMUD.MSG": WCCMMUD_TEXTS, "WCCTEXT.MSG": WCCTEXT_TEXTS}.get(name)
        if texts is None:
            ap.error("--catalogue needs WCCMMUD.MSG or WCCTEXT.MSG")
        text, count = write_catalogue(entries, texts, name)
        write(args.catalogue, text)
        print(f"{args.catalogue}: {count} entries")
    if args.assets:
        for p in write_assets(entries, args.assets):
            print(p)
    if args.help_topics:
        text, count = write_help_topics(entries)
        write(args.help_topics, text)
        print(f"{args.help_topics}: {count} topics")
    if args.options:
        with open(os.path.join(here, "config-defaults.md.in"), encoding="utf-8") as f:
            template = f.read()
        write(args.options, render_options_doc(entries, template))
        print(f"{args.options}: written")
    return 0


if __name__ == "__main__":
    sys.exit(main())
