#!/usr/bin/env python3
# Copied from the project's MBBSEmu test harness; edit here (see vendor/SOURCE.md). Stdlib only.
"""
Screen-pattern registry for MajorMUD 1.11p (stock Realm of Legends, played on MBBSEmu).

One table of named patterns (event name -> compiled regex, capture groups, category, example),
a fast line classifier, a configurable prompt (statline) parser, and loaders that turn the
game's own WCCMSG records (monster arrival/death/attack texts, weapon hit/miss verbs, spell
cast texts, item and room messages) into patterns instead of hard-coding them.

    import patterns as pt
    reg = pt.Registry.load()                 # static table + game data (needs mmud_data)
    for ev in reg.classify(text):            # text: bridge.plain() output, or raw bytes
        print(ev.name, ev.fields)

    pp = pt.PromptParser("custom", "[%h/%H %m/%M]%r:")   # SET STATLINE CUSTOM ...
    reg = pt.Registry.load(prompt=pp)

Self-test (runs the registry over logs/*/transcript.txt next to this script):

    python3 patterns.py [--top N] [--raw] [--logs DIR ...] [--no-data]

Design
------
* Lines, not blocks.  Text is split into lines first.  On the raw stream every message starts
  with ANSI_CLRLINE (ESC[79D ESC[K), which is a line break for us; on plain text (where that
  sequence is already gone) prompts glued to the front of a line ("[HP=48]:Slum Street") are
  peeled off.  A few displays wrap at 79 columns and are joined back (exits, "You notice ...
  here.", "Also here: ...", "You are carrying ...", keys).
* Rare-word index.  Every pattern has a key: the rarest literal word it must contain (worked
  out from the regex or WCCMSG template).  A line is only tried against patterns whose key is
  one of its words, so a line costs a handful of dict lookups and a few anchored regexes.
  Patterns without a usable key go to a small "always" bucket.
* Priorities.  Specific static patterns (50) beat data templates (60) beat generic fallbacks
  (90).  Within a priority, the longer (more literal) pattern wins.
* Context.  Room titles come from the room data (exact name match); lines after a title that
  match nothing are the long description.  Tables (shop list, spell list, party, who, "is
  equipped with") turn on row patterns until a line doesn't fit.

Pure stdlib; game data through mmud_data.py (optional: Registry.load(data=False)).
"""
import collections
import glob
import os
import re
import struct
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))

# --------------------------------------------------------------------------------------------
# Text preparation
# --------------------------------------------------------------------------------------------
CLRLINE = "\x1b[79D\x1b[K"            # ANSI_CLRLINE (starts nearly every message)
ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b[()][A-Z0-9]|\x1b.")
IAC_RE = re.compile(rb"\xff[\xfb-\xfe].|\xff\xfa.*?\xff\xf0|\xff[\xf0-\xfa]", re.S)


def raw_to_lines(data):
    """Raw session bytes (CP437 + ANSI, as in logs/*/raw.bin) -> plain lines.  CLRLINE, CR and
    LF all end a line; other ANSI is dropped; backspaces are applied; the prompt's ^A end
    marker (stripped by MBBSEmu in practice) also ends a line."""
    if isinstance(data, bytes):
        data = IAC_RE.sub(b"", data).decode("cp437", "replace")
    data = data.replace(CLRLINE, "\n").replace("\x01", "\n")
    data = ANSI_RE.sub("", data)
    prev = None
    while prev != data:
        prev, data = data, re.sub("[^\n\x08]\x08", "", data)
    data = data.replace("\x08", "").replace("\x07", "")
    data = data.replace("\r\n", "\n").replace("\r", "\n")
    return data.split("\n")


def plain_to_lines(text):
    """bridge.plain() output (or a transcript) -> lines."""
    return text.replace("\r\n", "\n").replace("\r", "\n").replace("\x07", "").split("\n")


# --------------------------------------------------------------------------------------------
# Prompt / statline
# --------------------------------------------------------------------------------------------
# The prompt.  Statline modes (SET STATLINE):
#   0 OFF ">", 1 ON and 2 FULL "[HP=..]:" / "[HP=../MA=..]:" / "[HP=../KAI=..]:",
#   3 CUSTOM and 4 FULL CUSTOM: the player's template with its %-codes filled in.
# FULL (and FULL CUSTOM) also make an empty line (Enter) redisplay the room;
# ON and CUSTOM only reprint the prompt.  New characters start FULL.
# Note where the resting mark goes: inside the brackets without mana, after "]:" with mana.
STD_PROMPT = (r"\[HP=(?P<hp>-?\d+)(?:/(?P<mtype>MA|KAI)=(?P<mana>-?\d+))?"
              r"(?P<state1> \((?:Resting|Meditating)\) )?\]:"
              r"(?P<state2> \((?:Resting|Meditating)\) )?"
              r"(?:\{Energy=(?P<energy>-?\d+) (?P<dbg>[^}]*)\}:)?")   # sysop DEBUG

# 'auto' mode without a known template: any bracketed "[...hp=...]:" line start (custom
# statlines seen in forum captures: "[HP=395/395,MA=392/433]:", "[hp=1/2,MA=8/42,CASH=180750,WARNS=Off]:")
GENERIC_PROMPT = re.compile(r"\[(?P<body>(?=[^\]\n]*(?:hp|HP|Hp)=)[^\]\n]{1,80})\]:"
                            r"(?P<state> \((?:Resting|Meditating)\) )?")
KV_RE = re.compile(r"([A-Za-z]+)=(-?\w+)(?:/(\w+))?")

# Custom statline %-codes
STATLINE_CODES = {
    "h": ("hp", r"-?\d+"), "H": ("maxhp", r"-?\d+"), "m": ("mana", r"-?\d+"),
    "M": ("maxmana", r"-?\d+"), "x": ("exp", r"\d+"), "X": ("exp_left", r"\d+"),
    "c": ("wealth", r"\d+"), "i": ("name", r"[A-Za-z]+"), "w": ("evilwarn", r"On|Off"),
    "p": (None, r"his|her|its"), "P": (None, r"him|her|it"), "t": (None, "nothing"),
    "r": ("state", r"(?: \((?:Resting|Meditating)\) )?"),
}
STATLINE_LEN = 60   # SET STATLINE CUSTOM text is cut to 60 characters


def compile_statline(tmpl):
    """A custom statline template -> list of regex sources, one per output line (%n splits).
    As the game fills it in: %% prints "%%" (two percent signs), unknown codes print
    as typed, %f/%b + digit and %d %B %N %U %L %R are colour codes (nothing in plain text)."""
    tmpl = tmpl[:STATLINE_LEN]
    segs, out, used = [], [], set()
    i = 0
    while i < len(tmpl):
        c = tmpl[i]
        if c != "%" or i + 1 >= len(tmpl):
            out.append(re.escape(c)); i += 1; continue
        k = tmpl[i + 1]
        if k in "fb" and i + 2 < len(tmpl) and tmpl[i + 2] in "01234567":
            i += 3; continue
        if k in "fb":                      # "%fh" gives "h": the letter after is copied
            i += 2; continue
        if k in "dBNULR":
            i += 2; continue
        if k == "%":
            out.append("%%"); i += 2; continue
        if k == "n":
            segs.append("".join(out)); out = []; i += 2; continue
        if k in STATLINE_CODES:
            name, rx = STATLINE_CODES[k]
            if name and name not in used:
                used.add(name)
                out.append(f"(?P<{name}>{rx})")
            else:
                out.append(f"(?:{rx})")
            i += 2; continue
        if k.isdigit():                    # "%5" gives "5"
            out.append(re.escape(k)); i += 2; continue
        out.append(re.escape("%" + k)); i += 2
    segs.append("".join(out))
    return segs


class PromptParser:
    """Recognises the prompt at the start of a line.  mode: 'std' (ON/FULL, the default),
    'off' ('>'), or 'custom' with the template given to SET STATLINE [FULL] CUSTOM.
    'auto' accepts the standard form and also a custom template once set_custom() is called
    (e.g. after reading 'Statusline:' from the SET screen)."""

    def __init__(self, mode="std", template=None):
        self.mode = mode
        self.std = re.compile(STD_PROMPT)
        self.custom = None
        self._pending = None   # fields of the earlier segments of a multi-line custom prompt
        if template:
            self.set_custom(template)

    def set_custom(self, template):
        self.template = template
        self.custom = [re.compile(s) for s in compile_statline(template)]
        if self.mode == "std":
            self.mode = "custom"

    def peel(self, line):
        """(fields or None, rest of the line).  fields: hp, mana, mtype, state ('Resting',
        'Meditating' or None) and whatever the custom template carries.  A multi-line custom
        prompt yields fields only on its last line ({'partial': True} before)."""
        if self.mode in ("std", "auto"):
            m = self.std.match(line)
            if m:
                d = {k: v for k, v in m.groupdict().items() if v is not None}
                st = (d.pop("state1", "") or d.pop("state2", "")).strip(" ()")
                d.pop("state2", None)
                d["state"] = st or None
                for k in ("hp", "mana", "energy"):
                    if k in d:
                        d[k] = int(d[k])
                d["form"] = "std"
                return d, line[m.end():]
        if self.mode in ("custom", "auto") and self.custom:
            segs = self.custom
            k = 0 if self._pending is None else self._pending[0]
            m = segs[k].match(line)
            if m and (m.end() > 0 or segs[k].pattern == ""):
                d = dict(self._pending[1]) if self._pending else {}
                d.update({a: b for a, b in m.groupdict().items() if b is not None})
                if k + 1 < len(segs):
                    self._pending = (k + 1, d)
                    return {"partial": True}, line[m.end():]
                self._pending = None
                for f in ("hp", "maxhp", "mana", "maxmana", "exp", "exp_left", "wealth"):
                    if f in d:
                        d[f] = int(d[f])
                if "state" in d:
                    d["state"] = d["state"].strip(" ()") or None
                d["form"] = "custom"
                return d, line[m.end():]
            self._pending = None
        if self.mode == "auto" and not self.custom:
            m = GENERIC_PROMPT.match(line)
            if m:
                d = {"form": "generic", "state": (m.group("state") or "").strip(" ()") or None}
                for k, v, mx in KV_RE.findall(m.group("body")):
                    k = {"ma": "mana", "kai": "mana", "cash": "wealth"}.get(k.lower(), k.lower())
                    if v.lstrip("-").isdigit():
                        d[k] = int(v)
                    if mx.isdigit():
                        d["max" + k] = int(mx)
                return d, line[m.end():]
        if self.mode == "off" and line.startswith(">"):
            return {"form": "off"}, line[1:]
        return None, line


# --------------------------------------------------------------------------------------------
# The static table: (category, event, regex, example, seen)
#   seen: C = confirmed against the game, F = forum capture only, I = inferred
#   Regexes are matched against the whole line (stripped of trailing blanks).
#   {HITV} {HITV3} {MISSV} are filled with weapon verbs from the item data (WCCMSG).
# --------------------------------------------------------------------------------------------
N = r"-?\d+"
DIR = r"north|south|east|west|northeast|northwest|southeast|southwest|up|down"
COIN = r"runic coins?|platinum pieces?|gold crowns?|silver nobles?|copper farthings?"
COINS = rf"(?:(?:\d+ (?:{COIN})(?:, )?)+|nothing)"
WHERE = rf"(?:to the (?P<dir>{DIR})|above you|below you|above|below)"
STAT = r"\*?\s*(-?\d+)"   # a '*' before a value marks a stat changed by a spell [F]

STATIC = [
    # ---- system / BBS -----------------------------------------------------------------------
    ("system", "bbs_banner", r"Enter Username or enter \"NEW\" to create a new Account", 'Enter Username or enter "NEW" to create a new Account', "C"),
    ("system", "bbs_username", r"Username:", "Username:", "C"),
    ("system", "bbs_password", r"Password:", "Password:", "C"),
    ("system", "bbs_masked", r"\*+", "*********", "C"),
    ("system", "login_greeting", r"Fantasy awaits you in MajorMUD v(?P<ver>\S+)!", "Fantasy awaits you in MajorMUD v1.11p-WG!", "C"),
    ("system", "bbs_menu_intro", r"Please select one of the following:", "Please select one of the following:", "C"),
    ("system", "bbs_menu_item", r"(?P<key>\w) \.\.\. (?P<item>.+)", "M ... MajorMUD", "C"),
    ("system", "bbs_menu_title", r"Main Menu", "Main Menu", "C"),
    ("system", "bbs_menu_prompt", r"Make your selection \(X to exit\):", "Make your selection (X to exit):", "C"),
    ("system", "bbs_more", r".*\((?:N)\)onstop, \((?:Q)\)uit, or \((?:C)\)ontinue\?.*", "(N)onstop, (Q)uit, or (C)ontinue?", "I"),
    ("system", "mud_title", r"M A J O R  M U D v(?P<ver>\S+) \((?P<built>.+)\)", "M A J O R  M U D v1.11p-WG (Dec 30 2005 14:20:05)", "C"),
    ("system", "mud_world", r"\{(?P<world>[^}]+)\}", "{Realm Of Legends}", "C"),
    ("system", "mud_ansi_note", r"\*ANSI RECOMMENDED\*", "*ANSI RECOMMENDED*", "C"),
    ("system", "mud_menu_item", r"\[(?P<key>[A-Z]+)\] \. (?P<item>.+?)(?: \[(?P<left>\d+) game minutes left\]| \[Last chance\])?", "[E] . Enter the Realm", "C"),
    ("system", "mud_menu_prompt", r"\[MAJORMUD\]:", "[MAJORMUD]:", "C"),
    ("system", "tournament_notice", r"\*\*\* MajorMUD Tournament Play enabled! \*\*\*", "*** MajorMUD Tournament Play enabled! ***", "C"),
    ("system", "enter_hangup_note", r"Last time you were on, you disconnected while playing\.", "Last time you were on, you disconnected while playing.", "C"),
    ("system", "enter_hangup_punished", r"The gods have punished you appropriately\.", "The gods have punished you appropriately.", "C"),
    ("system", "enter_banner", r"MajorMUD - ROL", "MajorMUD - ROL", "C"),
    ("system", "enter_rule", r"(?:=-)+=?", "=-=-=-=-=-...", "C"),
    ("system", "suicide_pw_notice", r"(?:To prevent accidental suicide or reroll, these commands|have been password protected\.  You have not yet entered|your suicide password, so please do so soon using the|SET SUICIDE command\.)", "SET SUICIDE command.", "C"),
    ("system", "bbs_logoff_confirm", r"Are you sure \(Y/N, or R to re-logon\)\?", "Are you sure (Y/N, or R to re-logon)?", "C"),
    ("system", "name_validating", r"(?:Please wait - Validating your name|Validating your name, please wait)\.", "Please wait - Validating your name.", "C"),
    ("system", "game_full", r"We're sorry, but the game is currently full\.", "We're sorry, but the game is currently full.", "C"),
    ("system", "out_of_credits", r"You have run out of credits for Major Mud\.", "You have run out of credits for Major Mud.", "C"),
    ("system", "player_entered", r"(?P<who>.+) just entered the Realm\.", "Bob just entered the Realm.", "C"),
    ("system", "player_left_realm", r"(?P<who>.+) just left the Realm\.", "Bob just left the Realm.", "C"),
    ("system", "player_disconnected", r"(?P<who>.+) just disconnected!!!", "Bob just disconnected!!!", "C"),
    ("system", "player_hungup", r"(?P<who>.+) just hung up!!!", "Bob just hung up!!!", "F"),
    ("system", "bbs_logoff_broadcast", r">> (?P<who>\S+) has just logged off, (?P<n>\d+) users on-line\.", ">> Witch1 has just logged off, 28 users on-line.", "F"),
    ("system", "bbs_logoff_self", r"\[(?P<who>[^\]]+)\] logs OFF - Bye bye now!", "[Bob] logs OFF - Bye bye now!", "F"),
    ("system", "exit_saved", r"Your character has been saved\. If you have any comments or suggestions, please", "Your character has been saved. ...", "C"),
    ("system", "exit_saved2", r"leave them in E-mail to Sysop\. Thanks\.", "leave them in E-mail to Sysop. Thanks.", "C"),
    ("system", "inactive_hangup", r"You have been inactive for too long\.\s+Bye!", "You have been inactive for too long.   Bye!", "C"),
    ("system", "exit_meditate", r"You will exit after a period of silent meditation\.\.?", "You will exit after a period of silent meditation.", "C"),
    ("system", "exit_others", r"(?P<who>.+) sits down and meditates\.", "Bob sits down and meditates.", "C"),
    ("system", "exit_interrupted", r"\.*Your meditation has been interrupted - you may not exit now!", "Your meditation has been interrupted - you may not exit now!", "C"),
    ("system", "exit_wounded", r"You may not exit MajorMud while you are mortally wounded!", "You may not exit MajorMud while you are mortally wounded!", "C"),
    ("system", "exit_waiting", r"You may not perform any commands while waiting to exit!", "You may not perform any commands while waiting to exit!", "C"),
    ("system", "too_fast", r"You are typing too quickly - command ignored", "You are typing too quickly - command ignored", "C"),
    ("system", "slow_down", r"Why don't you slow down for a few seconds\?", "Why don't you slow down for a few seconds?", "C"),
    ("system", "command_interrupted", r"Command interrupted\.", "Command interrupted.", "C"),
    ("system", "quiet_set", r"Quiet mode set", "Quiet mode set", "C"),
    ("system", "verbose_set", r"Verbose mode set", "Verbose mode set", "C"),
    ("system", "version", r"MajorMUD v (?P<ver>\S+)", "MajorMUD v 1.11p", "C"),
    ("system", "ambiguous", r"Please be more specific\.  You could have meant any of these:", "Please be more specific.  You could have meant any of these:", "C"),
    ("system", "ambiguous_item", r"-- (?P<name>.+)", "-- short sword", "C"),
    ("system", "settings_statline", r"Statusline:\s+(?P<mode>.+)", "Statusline:         Full", "C"),
    ("system", "settings_line", r"(?P<key>Life for this CHAR|Display Mode|Broadcast Channel|Talking speed|Follow Mode|Attack Interrupts|Receive Items|Warn on Evil|Allow Telepaths|Block Entrance Msg|Allow Gossip|Allow Auction|View gang members|Keep some exp|Messages|Paging|Talking Responses|Style|Colour Palette):?\s+(?P<val>.+)", "Display Mode:       Verbose", "C"),
    ("system", "statline_options", r"Valid statline options: ON, OFF, FULL, CUSTOM xxx, FULL CUSTOM xxx", "Valid statline options: ...", "C"),
    ("system", "statline_syntax", r"Syntax: STATLINE \{ON/OFF/FULL\}", "Syntax: STATLINE {ON/OFF/FULL}", "C"),
    ("system", "set_confirm", r"You will (?:now|no longer) .+\.", "You will now see game broadcasts such as user entrances and exits.", "C"),
    ("system", "set_options", r"Valid (?:\w+ )?(?:options|palettes)(?: are)?:? .+", "Valid follow options: BLIND, NORMAL", "C"),
    ("system", "syntax", r"Syntax: (?P<syntax>.+)", "Syntax: AID {user name}", "C"),
    ("system", "help_topics", r"The following help topics are currently available:", "The following help topics are currently available:", "C"),
    ("system", "help_none", r"Sorry, no help is available for that topic!", "Sorry, no help is available for that topic!", "C"),
    ("system", "chargen_cp", r"You have (?P<cp>\d+) CP to distribute\.", "You have 12 CP to distribute.", "C"),
    ("system", "chargen_limit", r"(?P<stat>Strength|Intellect|Willpower|Agility|Health|Charm) may not be (?P<dir>lower|higher) than (?P<n>\d+)\.(?:\s+.*)?", "Health may not be lower than 40.", "C"),
    ("system", "chargen_toomuch", r"You may not assign that much to (?P<stat>\w+)", "You may not assign that much to Strength", "C"),
    ("system", "chargen_name", r"(?:You must enter a last name!|You have entered an invalid name!|Your given name can only be 10 characters long!|Only ALPHA characters are allowed in a name!|Your family name can only be 18 characters long!)", "You must enter a last name!", "C"),
    ("system", "chargen_values", r"(?:\d+|none|black|white|silver|red|brown|blonde|green|blue|grey|SAVE|EXIT|short|long|[\s_\\/|.])+", "40   50   none   black", "C"),
    ("system", "chargen_box", r".*[\u2500-\u257f\u2580-\u259f].*", "│ » Strength ($$$$ to $$$$) $$$$ « │", "C"),
    ("system", "chargen_list_row", r"\[(?P<n>\d+)\]\s+(?P<name>.+)", "[1]  Human", "C"),
    ("system", "chargen_choose", r"(?:Please choose (?:a|your) (?P<what>race|class)(?: from the following list:| \[ \? for help \] :)|You must choose a (?:valid )?(?:race|class)\. \[ \? for help \])", "Please choose your race [ ? for help ] :", "C"),
    ("system", "chargen_lawful", r"Do you want to be Lawful\?\s+\[Yes/No\]", "Do you want to be Lawful?  [Yes/No]", "C"),
    ("system", "bbs_signup", r"Please (?:enter|re-enter|select) .+|NOTE: Passwords are stored .+|The MajorBBS and Worldgroup have a maximum password size.+", "Please enter a unique Username (Max. 29 Characters):", "C"),
    ("system", "who_header", r"Current Adventurers", "Current Adventurers", "C"),
    ("system", "who_rule", r"=+", "===================", "C"),
    ("system", "who_count", r"There are (?P<n>\d+) users in the game\.", "There are 3 users in the game.", "C"),
    ("system", "who_none", r"There are no users in the game at the moment\.", "There are no users in the game at the moment.", "C"),
    ("system", "topten_header", r"(?:Top )?Heroes of (?:T|t)he Realm|Top Gangs of the Realm", "Top Heroes of the Realm", "C"),

    # ---- room display (title, long description and items/who/exits) -------------------------
    ("room", "room_exits", r"Obvious exits: (?P<exits>.+)", "Obvious exits: closed door north, up", "C"),
    ("room", "room_items", r"You notice (?P<items>.+) here\.", "You notice 9 copper farthings, torch here.", "C"),
    ("room", "room_also_here", r"Also here:\s+(?P<who>.+)\.", "Also here: Thornwyn, giant rat.", "C"),
    ("room", "room_light", r"The room is (?P<light>barely visible|dimly lit)", "The room is dimly lit", "C"),
    ("room", "room_dark", r"The room is (?P<light>pitch black|very dark) - you can't see anything", "The room is very dark - you can't see anything", "C"),
    ("room", "blind", r"You are blind[.!]", "You are blind.", "C"),
    ("room", "arena_warning", r"(?:NOTE: This room is \*not\* a death-free zone right now!|Treat this room as you would any other combat zone\.)", "NOTE: This room is *not* a death-free zone right now!", "C"),
    ("room", "noise", rf"You hear movement {WHERE}[.!]", "You hear movement to the north.", "C"),
    ("room", "hiding_seen", r"You see (?P<who>.+) hiding in the shadows\.", "You see Bob hiding in the shadows.", "C"),
    ("room", "search_nothing", r"Your search revealed nothing\.", "Your search revealed nothing.", "C"),
    ("room", "search_found_exit", r"You found an exit (?:upwards|downwards|to the (?P<dir>\w+))!", "You found an exit to the west!", "C"),
    ("room", "search_found_trap", r"You found a trap (?:above you|below you|to the (?P<dir>\w+))!", "You found a trap to the north!", "C"),
    ("room", "search_nothing_dir", r"You notice nothing different (?:above you|below you|to the (?P<dir>\w+))\.", "You notice nothing different to the north.", "C"),
    ("room", "search_others", r"(?P<who>.+) is searching (?:the area|for exits)\.", "Bob is searching the area.", "C"),
    ("room", "concealed_opens", r"A concealed passage opens to the (?P<dir>\w+)\.", "A concealed passage opens to the west.", "C"),
    ("room", "look_others", r"(?P<who>.+) is looking (?:around the room|up|down|to the (?P<dir>\w+)|at you)\.", "Bob is looking around the room.", "C"),
    ("room", "peek_in", r"(?P<who>.+) peeks in from (?:above|below|the (?P<dir>\w+))!", "Bob peeks in from the north!", "C"),
    ("room", "look_nothing", r"Why would you want to look at that\?", "Why would you want to look at that?", "C"),
    ("room", "look_no_exit", r"There are no exits (?:upwards|downwards|to the (?P<dir>\w+))!", "There are no exits to the north!", "C"),
    ("room", "look_door_closed", r"The door is closed in that direction!", "The door is closed in that direction!", "C"),
    ("room", "look_monster", r"(?P<who>.+) appears to be (?P<state>.+)\.", "The giant rat appears to be unwounded.", "C"),
    ("room", "look_poisoned", r"(?P<who>.+) is poisoned!", "Bob is poisoned!", "C"),
    ("room", "look_equipped", r"(?P<who>He|She|It) is equipped with:", "He is equipped with:", "C"),
    ("room", "look_player", r"(?P<who>[A-Z][A-Za-z]+) is a (?P<desc>.+?, .+)", "Bob is a Human Warrior, ...", "C"),
    ("room", "look_player_header", r"\[ (?P<name>.+?) \](?: \((?P<gang>.+)\))?", "[ Nester TheDupe ] (Old Guard)", "F"),
    ("room", "look_up_down", r"(?P<who>.+) looks (?P<target>.+) up and down\.", "Bob looks Ann up and down.", "C"),
    ("room", "coin_desc", r"The (?:copper farthings look like they've been around forever|silver nobles glitter with use|gold crowns are rustic and used|platinum pieces shine as though they were new)\.", "The silver nobles glitter with use.", "C"),
    ("room", "currency_rates", r"(?:The currency conversion rates are:|(?P<n>\d+) (?P<small>.+) == 1 (?P<big>.+))", "10 copper farthings == 1 silver noble", "C"),

    # ---- movement and arrivals --------------------------------------------------------------
    ("movement", "no_exit", r"There is no exit in that direction!", "There is no exit in that direction!", "C"),
    ("movement", "door_closed", r"The (?P<what>door|gate) is (?:C|c)losed!", "The gate is closed!", "C"),
    ("movement", "door_closed_drag", r"There is a closed door in that direction!", "There is a closed door in that direction!", "C"),
    ("movement", "need_spell", r"You need to cast a spell to go that way!", "You need to cast a spell to go that way!", "C"),
    ("movement", "need_item", r"You do not have the appropriate item to go that direction!", "You do not have the appropriate item to go that direction!", "C"),
    ("movement", "toll_paid", r"You just paid (?P<paid>.+) in toll charges\.", "You just paid 5 gold crowns in toll charges.", "C"),
    ("movement", "toll_short", r"You do not have enough to cover the toll of (?P<n>\d+) (?P<cur>.+)\.", "You do not have enough to cover the toll of 5 gold crowns.", "C"),
    ("movement", "not_permitted", r"You are not permitted in that room!", "You are not permitted in that room!", "C"),
    ("movement", "exit_forbidden", r"You may not go through this exit(?: during tournament play)?!", "You may not go through this exit!", "C"),
    ("movement", "exit_level_low", r"You have not progressed far enough to go through this exit!", "You have not progressed far enough to go through this exit!", "C"),
    ("movement", "exit_level_high", r"You have progressed too far to go through this exit!|You have progressed too far for this room\.", "You have progressed too far for this room.", "C"),
    ("movement", "exit_alignment", r"You are too (?P<align>good|evil) to go through this exit!", "You are too evil to go through this exit!", "C"),
    ("movement", "exit_combat", r"You may not enter that room (?:while in combat|during a retaliation time-period)\.", "You may not enter that room while in combat.", "C"),
    ("movement", "exit_unhealthy", r"You are not healthy enough to enter that room\.", "You are not healthy enough to enter that room.", "C"),
    ("movement", "exit_timed", r"You may not pass through that exit at this point in time\.", "You may not pass through that exit at this point in time.", "C"),
    ("movement", "exit_purchase", r"(?:Your sysop must purchase|You must purchase) the .+ before you may move through this exit\.", "Your sysop must purchase the ... before you may move through this exit.", "C"),
    ("movement", "cant_move", r"You can't seem to move anywhere(?: to hide)?!", "You can't seem to move anywhere!", "C"),
    ("movement", "stunned", r"You are too stunned to move anywhere(?: to hide)?!", "You are too stunned to move anywhere!", "C"),
    ("movement", "too_heavy", r"You are too heavy to (?:move(?: anywhere)?|be dragged)!", "You are too heavy to move!", "C"),
    ("movement", "ran_into_wall", r"(?P<who>.+) ran into the wall to the (?P<dir>\w+)\.", "Bob ran into the wall to the north.", "C"),
    ("movement", "sneaking", r"Sneaking\.\.\.", "Sneaking...", "C"),
    ("movement", "sneak_attempt", r"Attempting to sneak\.\.\.(?P<rest>.*)", "Attempting to sneak...", "C"),
    ("movement", "sneak_fail", r"You don't think you're sneaking\.", "You don't think you're sneaking.", "C"),
    ("movement", "sneak_not_now", r"You may not sneak right now!", "You may not sneak right now!", "C"),
    ("movement", "sneak_off", r"You are no longer sneaking\.", "You are no longer sneaking.", "C"),
    ("movement", "made_sound", r"You make a sound as you enter the room!", "You make a sound as you enter the room!", "C"),
    ("movement", "sneak_seen_in", r"You notice (?P<who>.+) sneak in from (?:above|below|the (?P<dir>\w+))(?P<x>.*)\.", "You notice Bob sneak in from the west.", "C"),
    ("movement", "sneak_seen_out", r"You notice (?P<who>.+) sneaking out (?:upwards|downwards|to the (?P<dir>\w+))(?P<x>.*)\.", "You notice Bob sneaking out to the north.", "C"),
    ("movement", "player_arrives", r"(?P<who>[A-Z][A-Za-z]+) walks into the room from (?:above|below|the (?P<dir>\w+))(?P<x>.*)\.", "Bob walks into the room from the south.", "C"),
    ("movement", "player_leaves", r"(?P<who>[A-Z][A-Za-z]+) just left (?:upwards|downwards|to the (?P<dir>\w+))(?P<x>.*)\.", "Bob just left to the north.", "C"),
    ("movement", "monster_follows_any", r"(?:The |A |An )?(?P<mon>.+?) (?:[a-z]+ )?in after you!", "The dark monk creeps in after you!", "C"),
    ("movement", "monster_arrives_generic", r"(?P<mon>.+) (?:moves into the room|just arrived|moves into the) from (?:nowhere|above|below|the (?P<dir>\w+))\.", "goblin moves into the room from the east.", "C"),
    ("movement", "monster_leaves_generic", r"(?P<mon>.+?) just left (?:upwards|downwards|to the (?P<dir>\w+))\.", "happy guardsman just left to the east.", "C"),
    ("movement", "follow_leader", r"-- Following your Party leader (?P<dir>.+) --", "-- Following your Party leader north --", "C"),
    ("movement", "dragging", r"(?:You are (?:now )?dragging (?P<who>.+)\.|(?P<who2>.+) is dragging you around\.|You are no longer dragging (?P<who3>.+?)\.?)", "You are now dragging Bob.", "C"),
    ("movement", "teleported", r"You find yourself\.\.\. ?elsewhere\.", "You find yourself...elsewhere.", "C"),

    # ---- doors, gates, locks ----------------------------------------------------------------
    ("door", "door_opened", r"The (?P<what>.+) is now open\.", "The door is now open.", "C"),
    ("door", "door_already_open", r"The (?P<what>.+) (?:was|is) already open\.", "The door was already open.", "C"),
    ("door", "door_locked", r"The (?P<what>.+) is locked\.", "The door is locked.", "C"),
    ("door", "not_a_door", r"That is not a door or a gate!", "That is not a door or a gate!", "C"),
    ("door", "door_closed_now", r"The (?P<what>.+) is now closed\.", "The gate is now closed.", "C"),
    ("door", "door_not_open", r"That (?P<what>\w+) is not open\. Closing it will do nothing!", "That door is not open. Closing it will do nothing!", "C"),
    ("door", "door_no_close_combat", r"You may not (?:close|lock) doors or gates while attacking or being attacked!", "You may not close doors or gates while attacking or being attacked!", "C"),
    ("door", "door_moved_remote", r"The (?P<what>.+?) (?:to the (?P<dir>\w+)|above you|below you) just (?P<how>opened|closed|flew open|locked)[.!]", "The door to the north just opened.", "C"),
    ("door", "exit_moved_remote", r"The exit to the (?P<dir>\w+) just (?P<how>opened|closed)!", "The exit to the north just opened!", "C"),
    ("door", "door_others", r"You see (?P<who>.+) (?P<act>open|close|bash|attempt to bash|pick the lock on) the (?P<what>.+?) (?:to the (?P<dir>\w+)|above you|below you)\.", "You see Bob bash the door to the north.", "C"),
    ("door", "door_locked_now", r"The (?P<what>door|gate) is now locked\.", "The door is now locked.", "C"),
    ("door", "lock_need_close", r"You must close the (?P<what>door|gate) before you may lock it\.", "You must close the door before you may lock it.", "C"),
    ("door", "lock_no_benefit", r"There is no benefit to locking in that direction\.", "There is no benefit to locking in that direction.", "C"),
    ("door", "bash_through", r"You bash the door open and walk through", "You bash the door open and walk through", "C"),
    ("door", "door_slams", r"The door slams shut behind you!", "The door slams shut behind you!", "C"),
    ("door", "bash_fail", r"Your attempts to bash through fail!", "Your attempts to bash through fail!", "C"),
    ("door", "must_wait", r"You must wait before you may do that!", "You must wait before you may do that!", "C"),
    ("door", "bash_ok", r"You bashed the (?P<what>.+) open\.", "You bashed the gate open.", "C"),
    ("door", "bash_hurt", r"You take (?P<dmg>\d+) damage for bashing the door!", "You take 3 damage for bashing the door!", "C"),
    ("door", "bash_no_weapon", r"You need a weapon to bash with!", "You need a weapon to bash with!", "C"),
    ("door", "cant_bash_smash", r"You don't know the first thing about (?P<what>bashing|smashing)!", "You don't know the first thing about bashing!", "C"),
    ("door", "unlocked", r"You successfully unlocked the (?P<what>.+)\.", "You successfully unlocked the door.", "C"),
    ("door", "picklock_fail", r"Your skill fails you this time\.", "Your skill fails you this time.", "C"),
    ("door", "key_not_locked", r"The (?P<what>.+) was not locked\.", "The door was not locked.", "C"),
    ("door", "key_wrong", r"The (?P<what>.+) doesn't seem to fit that lock\.", "The brass key doesn't seem to fit that lock.", "C"),
    ("door", "trap_disarmed", r"You successfully disarmed the trap to the (?P<dir>\w+)\.", "You successfully disarmed the trap to the north.", "C"),
    ("door", "trap_disarm_fail", r"You failed to disarm any trap to the (?P<dir>\w+)\.", "You failed to disarm any trap to the north.", "C"),

    # ---- combat ------------------------------------------------------------------------------
    ("combat", "combat_engaged", r"\*Combat Engaged\*", "*Combat Engaged*", "C"),
    ("combat", "combat_off", r"\*Combat Off\*", "*Combat Off*", "C"),
    ("combat", "breaks_off", r"(?P<who>.+) breaks off combat\.", "Bob breaks off combat.", "C"),
    ("combat", "attacks_you", r"(?P<who>.+) moves to attack you!", "Bob moves to attack you!", "C"),
    ("combat", "attacks_room", r"(?P<who>.+) moves to attack everyone in the room\.", "Bob moves to attack everyone in the room.", "C"),
    ("combat", "attacks_other", r"(?P<who>.+) moves to attack (?P<target>.+?)[.!]", "Bob moves to attack giant rat.", "C"),
    ("combat", "protects", r"(?P<who>.+) moves to protect (?P<target>.+)", "ice golem moves to protect ice sorceress", "C"),
    ("combat", "casts_upon", r"(?P<who>.+) moves to cast (?P<spell>.+) upon (?P<target>.+)\.", "Bob moves to cast mend upon Ann.", "C"),
    ("combat", "you_hit", r"You (?:(?P<crit>critically) )?(?:(?P<surprise>surprise) )?(?P<verb>{HITV}) (?P<target>.+) for (?P<dmg>\d+) damage!", "You smack giant rat for 4 damage!", "C"),
    ("combat", "you_miss", r"You (?P<verb>{MISSV}) (?P<target>.+?)!", "You swipe at thin carrion beast!", "C"),
    ("combat", "you_dodged", r"You (?P<verb>{MISSV}) (?P<target>.+) who dodges your attack!", "You swing at orc who dodges your attack!", "C"),
    ("combat", "you_glance", r"Your (?P<verb>{MISSV}) (?P<target>.+) hits, but glances off (?:its|his|her) armour[.!]", "Your swing at Y hits, but glances off its armour.", "C"),
    ("combat", "other_hits", r"(?P<who>[A-Z][A-Za-z]+) (?:(?P<crit>critically) )?(?:(?P<surprise>surprise) )?(?P<verb>{HITV3}) (?P<target>.+) for (?P<dmg>\d+) damage!", "Thornwyn whaps giant rat for 4 damage!", "C"),
    ("combat", "other_miss", r"(?P<who>[A-Z][A-Za-z]+) (?P<verb>{MISSV3}) (?P<target>.+?)!", "Kasimir swings at short kobold thief!", "C"),
    ("combat", "other_dodged", r"(?P<who>[A-Z][A-Za-z]+) (?P<verb>.+) but it dodges\.", "Bob swings at orc but it dodges.", "C"),
    ("combat", "other_glance", r"(?P<who>.+)'s (?P<verb>.+) hits, but glances off (?:its|his|her) armour\.", "Bob's swing at orc hits, but glances off its armour.", "C"),
    ("combat", "mon_deflect_builtin", r"(?P<mon>.+)'s (?P<verb>.+) hits you, but your armour deflects[.!]", "Orc's slash hits you, but your armour deflects.", "C"),
    ("combat", "pvp_dodges", r"(?P<who>.+) dodges your attack!", "Nester dodges your attack!", "C"),
    ("combat", "took_damage", r"You took (?P<dmg>\d+) damage[.!]", "You took 3 damage.", "C"),
    ("combat", "smashed_it", r"You smashed (?P<target>.+) to the ground!", "You smashed Bob to the ground!", "C"),
    ("combat", "smashed_you", r"You are smashed to the ground!", "You are smashed to the ground!", "C"),
    ("combat", "smashed_down", r"(?:The )?(?P<target>.+) is smashed to the (?:ground|floor) defenseless!", "The orc is smashed to the floor defenseless!", "C"),
    ("combat", "rises", r"Slightly dazed (?:you get back onto your feet|(?:the )?(?P<who>.+) rises from the floor)\.", "Slightly dazed the orc rises from the floor.", "C"),
    ("combat", "need_rest_attack", r"You must rest before you may attack again\.?", "You must rest before you may attack again.", "C"),
    ("combat", "unbalanced", r"Such an (?:attack|action) would result in a very unbalanced (?:combat round|game)\.", "Such an attack would result in a very unbalanced combat round.", "C"),
    ("combat", "no_effect_weapon", r"Your (?:weapon|(?P<what>.+)) (?:has|have) no effect against this monster!", "Your weapon has no effect against this monster!", "C"),
    ("combat", "wrong_weapon", r"You are not using the required (?:weapon|spell) to hit this monster!", "You are not using the required weapon to hit this monster!", "C"),
    ("combat", "target_missing", r"You do not see (?P<target>.+) here!|You don't see your target here!", "You do not see rat here!", "C"),
    ("combat", "attack_self", r"(?:You cannot attack yourself!|Why would you want to attack yourself\?)", "You cannot attack yourself!", "C"),
    ("combat", "too_afraid", r"You are too afraid!", "You are too afraid!", "C"),
    ("combat", "guilt", r"You are overcome with a feeling of guilt and (?:break off your attack|return your hands to your own pockets)\.?", "You are overcome with a feeling of guilt and break off your attack.", "C"),
    ("combat", "evil_warning", r"To do this action, you must turn off your evil warnings\.", "To do this action, you must turn off your evil warnings.", "C"),
    ("combat", "too_evil", r"You have progressed too far to the evil side to do this action\.", "You have progressed too far to the evil side to do this action.", "C"),
    ("combat", "lawful_block", r"You have chosen a way of life which (?:does not allow|prevents) this action\.", "You have chosen a way of life which prevents this action.", "C"),
    ("combat", "evil_gained", r"A dark cloud passes over you", "A dark cloud passes over you", "C"),
    ("combat", "no_backstab", r"You cannot backstab with this weapon\.", "You cannot backstab with this weapon.", "C"),
    ("combat", "exp_gain", r"You gain (?P<exp>\d+) experience\.", "You gain 13 experience.", "C"),
    ("combat", "exp_capped", r"You have progressed too far without training to acquire anymore experience!", "You have progressed too far without training to acquire anymore experience!", "C"),
    ("combat", "coins_drop", rf"(?P<n>\d+) (?P<coin>platinum|gold|silver|copper|.+?) drop to the ground\.", "9 copper drop to the ground.", "C"),
    ("combat", "pvp_killed", r"(?P<who>.+) just killed (?P<victim>.+)\.", "Bob just killed Ann.", "C"),
    ("combat", "pvp_attacked", r"(?P<who>.+) just (?:attacked (?P<v1>.+)!|dodged an attack from (?P<v2>.+)\.|missed an attack against (?P<v3>.+)\.)", "Bob just attacked Ann!", "C"),
    ("combat", "summoned_appears", r"(?P<who>.+) appears!", "skeleton appears!", "C"),
    ("combat", "confused", r"You look around stupidly and do nothing!", "You look around stupidly and do nothing!", "C"),
    ("combat", "confused_other", r"(?P<who>.+) looks around stupidly and foams at the mouth!", "Bob looks around stupidly and foams at the mouth!", "C"),
    ("combat", "shattered", r"(?:You have shattered (?P<who>.+)'s (?P<item>.+)|Your (?P<mine>.+) has been shattered!)", "Your shield has been shattered!", "C"),
    ("combat", "mon_hits_you_generic", r"(?P<mon>[A-Z].+?) (?P<verb>[a-z][a-z -]+?) you(?: with .+?)? for (?P<dmg>\d+) damage!", "The orc slashes you for 7 damage!", "C"),
    ("combat", "mon_dodge_builtin", r"(?P<mon>.+) (?P<verb>.+) you with (?P<weapon>.+), but you dodge!", "Orc slashes you with sword, but you dodge!", "C"),
    ("combat", "mon_miss_builtin", r"(?P<mon>[A-Z].+) (?P<verb>[a-z].+) (?:at )?you with (?P<weapon>.+)\.", "Orc swings at you with club.", "C"),

    # ---- spells ------------------------------------------------------------------------------
    ("spell", "cast_fail", r"You attempt to cast (?P<spell>.+?)(?: at (?P<target>.+))?, but fail\.", "You attempt to cast vine strike, but fail.", "C"),
    ("spell", "cast_resisted", r"You attempt to cast (?P<spell>.+) at (?P<target>.+), but the spell is resisted\.", "You attempt to cast mmis at orc, but the spell is resisted.", "C"),
    ("spell", "target_resists", r"(?:The )?(?P<target>.+) resists your spell!", "The orc resists your spell!", "C"),
    ("spell", "you_resisted", r"You resisted (?P<caster>.+)'s (?:cast of )?(?P<spell>.+)\.", "You resisted orc shaman's cast of chill.", "C"),
    ("spell", "other_resisted", r"(?P<who>.+) resisted (?P<caster>.+)'s (?:cast of )?(?P<spell>.+)\.", "Bob resisted orc shaman's cast of chill.", "C"),
    ("spell", "cast_at_you_failed", r"(?:The )?(?P<caster>.+) attempted to cast (?P<spell>.+) (?:at|on) you, but failed\.", "The orc shaman attempted to cast chill at you, but failed.", "C"),
    ("spell", "other_cast_failed", r"(?:The )?(?P<caster>.+) attempted to cast (?P<spell>.+?)(?: (?:at|on) (?P<target>.+))?, but failed\.", "Bob attempted to cast mend, but failed.", "C"),
    ("spell", "no_effect_on", r"Your spell has no effect on (?P<target>.+)\.", "Your spell has no effect on skeleton.", "C"),
    ("spell", "no_effect_room", r"Your spell has no effect in this room!", "Your spell has no effect in this room!", "C"),
    ("spell", "no_mana", r"You do not have enough (?P<what>mana|kai)(?: to cast that spell| to invoke that power)?\.", "You do not have enough mana to cast that spell.", "C"),
    ("spell", "too_powerful", r"This spell is too powerful for you(?: to control\.| - .*)", "This spell is too powerful for you to control.", "C"),
    ("spell", "already_this_round", r"You have already (?P<what>cast a spell|invoked a power) this round!", "You have already cast a spell this round!", "C"),
    ("spell", "need_target", r"You must specify a target for that spell!", "You must specify a target for that spell!", "C"),
    ("spell", "bad_target", r"(?:You may not cast that spell on (?:a user|a monster|an item)!|Why would you want to cast a spell on that\?|(?P<who>.+) is not in this room!)", "You may not cast that spell on a user!", "C"),
    ("spell", "unknown_spell", r"You do not know how to cast (?P<spell>.+)\.", "You do not know how to cast fireball.", "C"),
    ("spell", "no_spells", r"You have no (?:spells to cast|powers to invoke)!", "You have no spells to cast!", "C"),
    ("spell", "spell_fails", r"Your (?:spell|power) fails!", "Your spell fails!", "C"),
    ("spell", "may_not_cast", r"You may not cast this spell\.|You may not (?:invoke|cast)\.\.\. You are (?:not )?KAI!.*", "You may not cast this spell.", "C"),
    ("spell", "level_too_low", r"You are not of a high enough level to cast that spell\.", "You are not of a high enough level to cast that spell.", "C"),
    ("spell", "no_component", r"You do not have the required material component to cast this spell!", "You do not have the required material component to cast this spell!", "C"),
    ("spell", "alone", r"You are the only one in the room!", "You are the only one in the room!", "C"),
    ("spell", "bash_spell_fail", r"Your spell fails to bash one of the exits!", "Your spell fails to bash one of the exits!", "C"),
    ("spell", "spells_header", r"You have the following (?P<what>spells|powers):", "You have the following spells:", "C"),
    ("spell", "spells_columns", r"Level (?:Mana|Kai)\s+Short Spell Name", "Level Mana Short Spell Name", "C"),
    ("spell", "spells_none", r"You have no (?P<what>spells|powers)\.", "You have no spells.", "C"),
    ("spell", "spells_wrong_kind", r"You may not list your (?:spells|powers)\..*", "You may not list your spells. You are KAI! ...", "C"),
    ("spell", "learn_scroll", r"You read (?P<item>.+) and learn the spell (?P<spell>.+)\.", "You read scroll of vines and learn the spell vine strike.", "C"),
    ("spell", "learn_spell", r"You learn the spell (?P<spell>.+)\.", "You learn the spell vine strike.", "C"),
    ("spell", "already_known", r"You realize that you already know this scroll!", "You realize that you already know this scroll!", "C"),
    ("spell", "use_unknown", r"You don't know what to do with this!", "You don't know what to do with this!", "C"),
    ("spell", "spell_purged", r"Purging spell (?P<spell>.+) from your spellbook\.", "Purging spell x from your spellbook.", "C"),
    ("spell", "kai_learned", r"You learn the following Kai abilities:", "You learn the following Kai abilities:", "C"),
    ("spell", "detect_magic", r"(?:(?P<item>.+) glows (?:faintly|softly|brightly), indicating .+ magic within\.|You are almost blinded by the aura from .+|You detect no magic in that item!)", "torch glows faintly, indicating a small amount of magic within.", "C"),
    ("spell", "you_cast_generic", r"You cast (?P<spell>.+?)(?: (?:at|on|upon) (?P<target>.+?))?(?: for (?P<dmg>\d+) damage| for (?P<heal>\d+)|, regenerating (?P<regen>\d+) damage)?[.!]", "You cast vine strike at acid slime for 7 damage!", "C"),
    ("spell", "cast_on_you_generic", r"(?P<caster>.+) casts? (?P<spell>.+?) (?:on|upon|at) you(?P<rest>.*)[.!]", "Thornwyn casts mend on you, regenerating some of your wounds!", "C"),
    ("spell", "cast_other_generic", r"(?P<caster>.+) casts? (?P<spell>.+?)(?: (?:on|upon|at) (?P<target>.+?))?(?: for (?P<dmg>\d+) damage)?[.!]", "Thornwyn casts vine strike on rat for 5 damage!", "C"),

    # ---- loot, items -------------------------------------------------------------------------
    ("loot", "picked_up_coins", rf"You picked up (?P<n>\d+) (?P<coin>{COIN})", "You picked up 9 copper farthings", "C"),
    ("loot", "other_picks_coins", r"(?P<who>.+) picks up some (?P<coin>.+)\.", "Bob picks up some gold crowns.", "C"),
    ("loot", "took_item", r"You took (?P<item>.+)\.", "You took torch.", "C"),
    ("loot", "other_picks_item", r"(?P<who>.+) picks up (?P<item>.+)\.", "Bob picks up torch.", "C"),
    ("loot", "not_here", r"You don't see (?:any )?(?P<what>.+?)(?: here\.)?", "You don't see rto here.", "C"),
    ("loot", "too_much", r"You cannot carry that much!", "You cannot carry that much!", "C"),
    ("loot", "strange_force", r"A strange force stops you from getting this item\.", "A strange force stops you from getting this item.", "C"),
    ("loot", "dropped", r"You dropped (?P<item>.+)\.", "You dropped quarterstaff.", "C"),
    ("loot", "other_dropped", r"(?P<who>.+) dropped (?:some )?(?P<item>.+)\.", "Bob dropped torch.", "C"),
    ("loot", "drop_missing", r"You don't have (?P<item>.+) to drop!", "You don't have torch to drop!", "C"),
    ("loot", "drop_forbidden", r"You may not (?:drop|give|hide) (?:that item|that item away)!", "You may not drop that item!", "C"),
    ("loot", "room_full", r"There is no room to (?:drop|hide) (?P<item>.+) here\.", "There is no room to drop torch here.", "C"),
    ("loot", "hid_item", r"You hid (?P<item>.+)\.", "You hid torch.", "C"),
    ("loot", "hide_attempt", r"Attempting to hide\.\.\.(?P<rest>.*)", "Attempting to hide... You don't think you are hidden.", "C"),
    ("loot", "gave_coins", r"You gave (?P<who>.+) (?P<n>\d+) (?P<coin>\w+)", "You gave Bob 10 copper", "C"),
    ("loot", "got_coins", r"(?P<who>.+) gave you (?P<n>\d+) (?P<coin>.+)", "Bob gave you 10 copper", "C"),
    ("loot", "gave_item", r"You just gave (?P<item>.+) to (?P<who>.+)\.", "You just gave torch to Bob.", "C"),
    ("loot", "got_item", r"(?P<who>.+) just gave you (?P<item>.+)\.", "Bob just gave you torch.", "C"),
    ("loot", "give_refused", r"(?P<who>.+) (?:refuses your offer|cannot accept your offer)\.", "Bob refuses your offer.", "C"),
    ("loot", "not_enough_coins", r"You do not have (?:that much|that many) (?P<coin>.+)!", "You do not have that much copper!", "C"),
    ("loot", "share_none", r"You do not have enough to share!|There is nobody to share with\.", "There is nobody to share with.", "C"),
    ("loot", "uses_gone", r"It's uses gone, (?P<item>.+) disappears from your inventory!", "It's uses gone, potion disappears from your inventory!", "C"),
    ("loot", "no_uses", r"There are no more uses in (?P<item>.+)\.", "There are no more uses in wand.", "C"),
    ("loot", "use_forbidden", r"You may not use that item!|You must have that item readied to use it!|You must be wearing that item to use it!", "You may not use that item!", "C"),
    ("loot", "dont_have", r"You don't have (?P<item>.+)\.", "You don't have potion.", "C"),
    ("loot", "not_carrying", r"You are not carrying (?P<item>.+)\.", "You are not carrying wand.", "C"),
    ("loot", "receive_hp", r"You receive (?P<n>\d+) (?P<what>hit points|mana)\.", "You receive 10 hit points.", "C"),
    ("loot", "eat_drink", r"You (?P<act>eat|drink) the (?P<item>.+)\.", "You drink the potion.", "C"),
    ("loot", "other_eat_drink", r"(?P<who>.+) (?P<act>eats|drinks) (?P<item>.+)\.", "Bob drinks potion.", "C"),
    ("loot", "item_returned", r"Your (?P<item>.+) has returned to its rightful place\.", "Your ruby has returned to its rightful place.", "C"),
    ("loot", "item_removed", r"Your (?P<item>.+) has been removed\.", "Your sword has been removed.", "C"),
    ("loot", "rob", r"(?:You stole (?P<n>\d+) (?P<coin>\S+) from (?P<v1>.+)\.|You successfully stole (?P<item>.+) from (?P<v2>.+)\.|Your skills fail as you try to rob (?P<v3>.+)\.|You bump (?P<v4>.+) as you try to rob .+\.|(?P<who>.+) bumps you as .+ tries to rob you!)", "Your skills fail as you try to rob orc.", "C"),
    ("loot", "track", r"(?:(?P<who>.+) went (?P<dir>\w+) from here\.|Your tracking skills fail you this time\.)", "Your tracking skills fail you this time.", "C"),

    # ---- inventory and equipment ------------------------------------------------------------
    ("inventory", "inv_nothing", r"You are carrying Nothing!", "You are carrying Nothing!", "C"),
    ("inventory", "inv_carrying", r"You are carrying (?P<items>.+)", "You are carrying 18 silver nobles, padded vest (Torso), quarterstaff (Two handed)", "C"),
    ("inventory", "inv_no_keys", r"You have no keys\.", "You have no keys.", "C"),
    ("inventory", "inv_keys", r"You have the following keys:\s+(?P<keys>.+)\.", "You have the following keys:  brass key.", "C"),
    ("inventory", "inv_wealth", r"Wealth: (?P<copper>\d+) copper farthings?", "Wealth: 223 copper farthings", "C"),
    ("inventory", "inv_encumbrance", r"Encumbrance: (?P<cur>\d+)/(?P<max>\d+) - (?P<level>None|Light|Medium|Heavy) \[(?P<pct>\d+)%\]", "Encumbrance: 520/3792 - None [13%]", "C"),
    ("inventory", "money_none", r"No money!", "No money!", "C"),
    ("equipment", "wear_ok", r"You are now wearing (?P<item>.+)\.", "You are now wearing padded vest.", "C"),
    ("equipment", "wield_ok", r"You are now holding (?P<item>.+)\.", "You are now holding quarterstaff.", "C"),
    ("equipment", "removed", r"You have removed (?P<item>.+?)(?P<ext> and extinguished it)?\.", "You have removed torch and extinguished it.", "C"),
    ("equipment", "unwielded", r"You now have no weapon readied\.", "You now have no weapon readied.", "C"),
    ("equipment", "wear_fail", r"You may not wear that item!|You have no more room to wear that item!|(?P<item>.+) may not be worn!", "You may not wear that item!", "C"),
    ("equipment", "not_have_any", r"You do not have (?:any |a )?(?P<item>.+?)(?: left unequipped)?\.", "You do not have quarterstaff left unequipped.", "C"),
    ("equipment", "not_wearing", r"You are not wearing (?P<item>.+)\.", "You are not wearing chain coif.", "C"),
    ("equipment", "cant_remove", r"You cannot remove (?P<item>.+)!|You are already wearing (?P<item2>.+) and it may not be removed\.", "You cannot remove cursed ring!", "C"),
    ("equipment", "offhand_2h", r"You may not wear an off-hand item while you have a 2-handed weapon readied\.|You may not ready a 2-handed weapon with your (?P<item>.+) worn!", "You may not ready a 2-handed weapon with your buckler worn!", "C"),
    ("equipment", "weapon_forbidden", r"You may not use that weapon\.|(?P<item>.+) is not a weapon!", "You may not use that weapon.", "C"),
    ("equipment", "weapon_heavy", r"This weapon feels heavy in your hands\.", "This weapon feels heavy in your hands.", "C"),
    ("equipment", "weapon_quick", r"You are extremely quick and deadly with this weapon\.", "You are extremely quick and deadly with this weapon.", "C"),
    ("equipment", "other_wears", r"(?P<who>[A-Z]\w+) (?P<act>wears|wields|removes|lights) (?P<item>.+?)[.!]", "Bob wields quarterstaff!", "C"),
    ("equipment", "light_ok", r"You lit the (?P<item>.+)\.", "You lit the torch.", "C"),
    ("equipment", "light_fail", r"You may not light that item!|You already have something lit!|You cannot light (?P<item>.+)!|You must recharge that before you may light it again\.", "You already have something lit!", "C"),
    ("equipment", "light_out", r"(?P<item>.+) is no longer lit!", "torch is no longer lit!", "C"),
    ("equipment", "light_out_other", r"(?P<who>.+)'s (?P<item>.+) just went out\.", "Bob's torch just went out.", "C"),
    ("equipment", "not_lit", r"You do not have (?P<item>.+) lit\.", "You do not have torch lit.", "C"),
    ("equipment", "light_level", r"The current light level is (?P<light>.+)", "The current light level is dimly lit", "C"),

    # ---- shops, healer, bank --------------------------------------------------------------
    ("shop", "shop_list_intro", r"The following items are for sale here:", "The following items are for sale here:", "C"),
    ("shop", "shop_list_header", r"Item\s+Quantity\s+Price", "Item                          Quantity    Price", "C"),
    ("shop", "shop_list_rule", r"-{20,}", "------------------------------------------------------", "C"),
    ("shop", "bought", rf"You just bought (?P<item>.+) for (?P<price>{COINS})\.", "You just bought torch for 2 silver nobles, 5 copper farthings.", "C"),
    ("shop", "buy_not_here", r"You cannot buy (?P<item>.+) here!", "You cannot buy torch here!", "C"),
    ("shop", "buy_unknown", r"(?P<item>.+) is not a known item\.", "sowrd is not a known item.", "C"),
    ("shop", "buy_poor", r"You cannot afford (?P<item>.+)\.|You cannot afford to do that!", "You cannot afford chainmail.", "C"),
    ("shop", "not_in_shop", r"You cannot (?P<cmd>BUY|SELL|LIST|DEPOSIT|WITHDRAW|STOCK|UNSTOCK|MARKUP) if you are not in (?:a shop|a bank|your gangs shop)!", "You cannot BUY if you are not in a shop!", "C"),
    ("shop", "other_bought", r"You see (?P<who>.+) (?P<act>buy|sell) a (?P<item>.+)\.", "You see Bob buy a torch.", "C"),
    ("shop", "sold", r"You sold (?P<item>.+) for (?P<n>\d+) (?P<coin>.+)\.", "You sold club for 2 copper farthings.", "C"),
    ("shop", "sell_value", r"You would get (?P<n>\d+) (?P<coin>.+) for your (?P<item>.+)\.", "You would get 2 copper farthings for your club.", "C"),
    ("shop", "sell_refused", r"You cannot sell (?P<item>.+) here\.|You may not sell (?:that item!|items to a gang shop\.)|You don't have (?P<item2>.+) to sell!", "You cannot sell club here.", "C"),
    ("shop", "healed_paid", rf"You hand over (?P<paid>{COINS}) and all your wounds are healed\.", "You hand over 3 silver nobles and all your wounds are healed.", "C"),
    ("shop", "cured_paid", rf"You hand over (?P<paid>{COINS}) and (?:your poisoning is cured|find that you were not poisoned!?)\.?", "You hand over 1 silver noble and your poisoning is cured.", "C"),
    ("shop", "heal_poor", r"You do not have sufficient funds to buy (?:full healing|curing)!", "You do not have sufficient funds to buy full healing!", "C"),
    ("shop", "healer_done", r"There is nothing more that (?P<who>.+) can do for you\.", "There is nothing more that healer can do for you.", "C"),
    ("shop", "bank_deposit", rf"You deposit (?P<paid>{COINS})\.", "You deposit 5 gold crowns.", "C"),
    ("shop", "bank_withdraw", r"You withdrew (?P<n>\d+) (?P<coin>.+)\.", "You withdrew 5 gold crowns.", "C"),
    ("shop", "bank_balance", r"Your balance at (?P<bank>.+) \(#(?P<num>\d+)\) is:|On deposit: (?P<dep>.+)", "Your balance at First Bank (#7) is:", "C"),
    ("shop", "bank_misc", r"The bank cannot accept your deposit at this time\.|Please specify a more reasonable amount\.|You see (?P<who>.+) making a (?:deposit|withdrawal)\.", "You see Bob making a deposit.", "C"),
    ("shop", "hand_over_generic", rf"You hand over (?P<paid>{COINS}) and (?P<what>.+)", "You hand over 1 gold crown and you are now female!", "C"),

    # ---- training and levels ----------------------------------------------------------------
    ("training", "trained", rf"You hand over (?P<paid>{COINS}) and you receive training to attain level (?P<level>\d+)\.", "You hand over 5 silver nobles and you receive training to attain level 2.", "C"),
    ("training", "train_receive", r"You receive the following:", "You receive the following:", "C"),
    ("training", "train_cp", r"(?P<cp>\d+) additional character points", "10 additional character points", "C"),
    ("training", "train_lives", r"(?P<lives>\d+) additional lives", "1 additional lives", "C"),
    ("training", "train_no_exp", r"You do not have the required experience to train yet!", "You do not have the required experience to train yet!", "C"),
    ("training", "train_no_money", r"You do not have the money required for your training\.", "You do not have the money required for your training.", "C"),
    ("training", "train_wrong_room", r"You must be in an appropriate training room to train!|This shop is not suitable for your training\.", "You must be in an appropriate training room to train!", "C"),
    ("training", "train_level_band", r"You have (?P<which>not progressed far enough|progressed too far) to use the training provided here\.", "You have progressed too far to use the training provided here.", "C"),
    ("training", "train_capped", r"You may not train any further\.  Please contact your sysop|for appropriate access so that you may continue your training\.", "You may not train any further.  Please contact your sysop", "C"),
    ("training", "train_stats_refused", r"You may not train your stats here!|Your stats are unnaturally altered!  You may not train stats now\.|You may not do any training while in tournament mode!", "You may not train your stats here!", "C"),
    ("training", "stats_reset_notice", r"Please TRAIN STATS as soon as possible to re-apply your CPs|Your stats have been reset due to an earlier bug\.", "Please TRAIN STATS as soon as possible to re-apply your CPs", "C"),

    # ---- status screens ----------------------------------------------------------------------
    ("status", "st_name", r"Name: (?P<name>.+?)\s+Lives/CP:\s+(?P<lives>-?\d+)/(?P<cp>-?\d+)", "Name: Kasimir Drakemoor                Lives/CP:      9/12", "C"),
    ("status", "st_race", r"Race: (?P<race>.+?)\s+Exp: (?P<exp>\d+)\s+Perception:\s+" + STAT, "Race: Kang        Exp: 1712            Perception:     33", "C"),
    ("status", "st_class", r"Class: (?P<cls>.+?)\s+Level: (?P<level>\d+)\s+Stealth:\s+" + STAT, "Class: Witchunter Level: 2             Stealth:         0", "C"),
    ("status", "st_hits", r"Hits:\s+(?P<hp>-?\d+)/(?P<maxhp>\d+)\s+Armour Class:\s+(?P<ac>-?\d+)/(?P<dr>-?\d+)\s+Thievery:\s+" + STAT, "Hits:    48/56    Armour Class:  19/1  Thievery:        0", "C"),
    ("status", "st_mana", r"(?P<mtype>Mana|Kai):\s+\*?\s*(?P<mana>-?\d+)/(?P<maxmana>\d+)(?:\s+Spellcasting:\s+" + STAT + r")?\s+Traps:\s+" + STAT, "Mana: *  9/9    Spellcasting: 40     Traps:           0", "C"),
    ("status", "st_traps", r"Traps:\s+" + STAT, "                                       Traps:           0", "C"),
    ("status", "st_picklocks", r"Picklocks:\s+" + STAT, "                                       Picklocks:       0", "C"),
    ("status", "st_str", r"Strength:\s+" + STAT + r"\s+Agility:\s*" + STAT + r"\s+Tracking:\s+" + STAT, "Strength:  79     Agility: 42          Tracking:        0", "C"),
    ("status", "st_int", r"Intellect:\s*" + STAT + r"\s+Health:\s+" + STAT + r"\s+Martial Arts:\s*" + STAT, "Intellect: 40     Health:  64          Martial Arts:   0", "C"),
    ("status", "st_wil", r"Willpower:\s*" + STAT + r"\s+Charm:\s+" + STAT + r"\s+MagicRes:\s+" + STAT, "Willpower: 40     Charm:   30          MagicRes:       30", "C"),
    ("status", "st_poisoned", r"You are Poisoned!", "You are Poisoned!", "C"),
    ("status", "st_rank", r"You are in the (?P<rank>front|back) rank of your group\.", "You are in the front rank of your group.", "C"),
    ("status", "st_spells_on", r"You have the following spell cast upon you:", "You have the following spell cast upon you:", "C"),
    ("status", "exp", r"Exp: (?P<exp>\d+) Level: (?P<level>\d+) Exp needed for next level: (?P<left>\d+) \((?P<total>\d+)\) \[(?P<pct>\d+)%\]", "Exp: 346 Level: 1 Exp needed for next level: 1304 (1650) [21%]", "C"),
    ("status", "exp_not_restructured", r"Your experience has not been restructured for the new system\.|Please exit and re-enter The Realm\. This should restructure your experience points\.", "Your experience has not been restructured for the new system.", "C"),
    ("status", "health", r"Health:\s+(?P<hp>-?\d+)/(?P<maxhp>\d+)\s+\[(?P<hppct>-?\d+)%\](?:\s+(?P<mtype>Mana|Kai):\s+(?P<mana>-?\d+)/(?P<maxmana>\d+)\s+\[(?P<mpct>-?\d+)%\])?", "Health:    48/56    [85%]", "C"),

    # ---- resting, healing -------------------------------------------------------------------
    ("rest", "rest_start", r"You are now resting\.", "You are now resting.", "C"),
    ("rest", "rest_other", r"(?P<who>.+) stops to rest\.", "Bob stops to rest.", "C"),
    ("rest", "rest_sick", r"You are too sick to (?P<what>rest|meditate)!", "You are too sick to rest!", "C"),
    ("rest", "meditate_start", r"You are now meditating\.", "You are now meditating.", "C"),
    ("rest", "meditate_other", r"(?P<who>.+) kneels to meditate\.", "Bob kneels to meditate.", "C"),
    ("rest", "meditate_useless", r"Meditation will not help at this time\.", "Meditation will not help at this time.", "C"),
    ("rest", "meditate_done", r"You awake from deep meditation feeling stronger!", "You awake from deep meditation feeling stronger!", "C"),
    ("rest", "poison_tick", r"You feel ill\.", "You feel ill.", "C"),
    ("rest", "aided", r"You have aided (?P<who>.+), (?P<pron>\w+) wounds are now healing\.|(?P<who2>.+) has aided you\.|(?P<who3>.+) is in no need of assistance\.", "Bob has aided you.", "C"),

    # ---- party / group / gang ----------------------------------------------------------------
    ("party", "party_header", r"The following people are in your travel party:", "The following people are in your travel party:", "C"),
    ("party", "party_none", r"You are not in a party at the present time\.", "You are not in a party at the present time.", "C"),
    ("party", "following", r"You are (?:now )?following (?P<who>.+?)\.?", "You are now following Bob", "C"),
    ("party", "follower_joined", r"(?P<who>.+) started to follow you\.", "Bob started to follow you.", "C"),
    ("party", "invited_follow", r"(?P<who>.+) has invited you to (?P<what>follow|join) (?P<target>.+)\.", "Bob has invited you to follow him.", "C"),
    ("party", "you_invited", r"You have invited (?P<who>.+) to (?:follow you|join your gang)\.", "You have invited Bob to follow you.", "C"),
    ("party", "invite_needed", r"You must be invited first!", "You must be invited first!", "C"),
    ("party", "unfollowed", r"You are no longer following (?P<who>.+)\.", "You are no longer following Bob.", "C"),
    ("party", "follower_removed", r"(?P<who>.+) has been removed from your followers\.", "Bob has been removed from your followers.", "C"),
    ("party", "rank_moved", r"(?:You have moved to the (?P<rank>front|middle|back) ranks of your group\.|(?P<who>.+) just moved to the (?P<rank2>front rank|middle|back rank) (?:in|of) your group\.)", "You have moved to the front ranks of your group.", "C"),
    ("party", "rank_refused", r"You (?:may not (?:enter the backrank of your own party|leave your current rank in your own party)|are the only one in the (?:front|back) rank\.  You may not leave)\.|There would not be enough people left in the frontrank if you did that\.", "You are the only one in the front rank.  You may not leave.", "C"),
    ("party", "group_left", r"You just left group (?P<n>\d+)\.|(?P<who>.+) just left your group \((?P<n2>\d+)\)", "You just left group 3.", "C"),
    ("party", "gang_msg", r"(?:You have joined the gang (?P<gang>.+)\.|(?P<who>.+) just joined your gang\.|You have left (?P<gang2>.+)\.|(?P<who2>.+) has left (?P<gang3>.+)\.)", "You have joined the gang Wolves.", "C"),

    # ---- talk ---------------------------------------------------------------------------------
    ("comms", "says_to_you", r"(?P<who>.+) says \(to (?P<target>you|.+)\) \"(?P<msg>.*)\"", 'Bob says (to you) "hi"', "C"),
    ("comms", "says", r"(?P<who>\S+) says \"(?P<msg>.*)\"", 'Bob says "hi"', "C"),
    ("comms", "you_say", r"You (?P<verb>say|ask|exclaim|\w+) \"(?P<msg>.*)\"", 'You say "sysop goto silvermere"', "C"),
    ("comms", "monster_says", r"The (?P<mon>.+?) (?P<verb>says|screams|shouts|yells|whispers|mutters|growls|cries) \"(?P<msg>.*)\"", 'The fanatic screams "Death to those who oppose the Blood God!"', "C"),
    ("comms", "well_spoken", r"Well Spoken\.\.\.", "Well Spoken...", "C"),
    ("comms", "yell", r"From (?P<who>.+) \(Yelling\): \"(?P<msg>.*)\"|Someone yells from (?:above|below|the (?P<dir>\w+)) \"(?P<msg2>.*)\"", 'Someone yells from the north "help"', "C"),
    ("comms", "telepath", r"(?P<who>.+) telepaths:\s+(?P<msg>.*)", "Bob telepaths: @heal", "C"),
    ("comms", "telepath_status", r"--- (?:Telepath Sent to (?P<who>.+)|Telepath Not Sent|Message Directed to (?P<who2>.+)|Message Not Sent) ---", "--- Telepath Sent to Bob ---", "C"),
    ("comms", "maxpages_set", r"Maxpages set to (?P<n>\d+)", "Maxpages set to 30", "C"),
    ("comms", "user_unknown", r"Cannot find user!", "Cannot find user!", "C"),
    ("comms", "gossip", r"(?P<who>.+) (?P<chan>gossips|auctions|gangpaths): (?P<msg>.*)", "Bob gossips: hello", "C"),
    ("comms", "broadcast", r"Broadcast from (?P<who>.+) \"(?P<msg>.*)\"", 'Broadcast from Bob "hi"', "C"),
    ("comms", "profanity", r"You are using too much profanity - your message is not sent\.", "You are using too much profanity - your message is not sent.", "C"),
    ("comms", "cannot_speak", r"You cannot speak!", "You cannot speak!", "C"),
    ("comms", "channel", r"(?:You just joined channel (?P<n>\d+)\.|(?P<who>.+) just (?:joined|left) your channel \((?P<n2>\d+)\))", "You just joined channel 1.", "C"),
    ("comms", "gods_tell", r"The gods tell you: (?P<msg>.*)", "The gods tell you: behave", "C"),

    # ---- death and unconsciousness ------------------------------------------------------------
    ("death", "drops_unconscious", r"(?P<who>.+) drops to the ground!", "Thornwyn drops to the ground!", "C"),
    ("death", "you_killed", r"You have been killed!", "You have been killed!", "C"),
    ("death", "other_dead", r"(?P<who>.+) is dead\.", "Bob is dead.", "C"),
    ("death", "saved", r"But, (?:due to a miracle|because you were in a colliseum), you have been saved\.", "But, due to a miracle, you have been saved.", "C"),
    ("death", "lives_left", r"You (?:have|now have) (?P<lives>\d+) lives (?:left|remaining)\.", "You have 8 lives left.", "C"),
    ("death", "body_appears", r"(?P<who>.+) appeared on the floor in the middle of the room\.", "Bob appeared on the floor in the middle of the room.", "C"),
    ("death", "no_lives", r"You have no lives remaining!", "You have no lives remaining!", "C"),
    ("death", "life_ebbing", r"You feel your life-force ebbing\.", "You feel your life-force ebbing.", "C"),
    ("death", "suicide", r"After a LONG thought, you take your own life\.?|(?P<who>.+) took their own life!", "After a LONG thought, you take your own life.", "C"),
    ("death", "mortally_wounded", r"You may not do that while you are mortally wounded!", "You may not do that while you are mortally wounded!", "C"),

    # ---- errors / generic ----------------------------------------------------------------------
    ("error", "no_effect", r"Your command had no effect\.", "Your command had no effect.", "C"),
    ("error", "why_would_you", r"Why (?:would|are) you .+[?.]", "Why would you want to follow yourself?", "C"),
    ("error", "not_implemented", r"Following a monster - Not implemented yet\.", "Following a monster - Not implemented yet.", "C"),
    ("error", "may_not_here", r"You may not do that here!", "You may not do that here!", "C"),
    ("error", "no_autocombat", r"You may not be in autocombat when you do that\.|You may not teleport during a PVP retaliation period\.", "You may not be in autocombat when you do that.", "C"),
]

# (event -> row event) tables: rows are tried only right after the opener
TABLES = {
    "shop_list_rule": ("shop_row", "shop", r"(?P<item>.+?)\s+(?P<qty>\d+)\s+(?:(?P<price>\d+) (?P<cur>[a-z ]+?)|Free)(?: \((?P<flag>Too powerful|You can't use)\))?", "quarterstaff                  31           Free"),
    "spells_columns": ("spell_row", "spell", r"(?P<level>\d+)\s+(?P<mana>\d+)\s+(?P<short>\S{1,4})\s+(?P<name>.+?)", "  1   2    vine  vine strike"),
    "party_header": ("party_row", "party", r"(?P<name>.+?)\s+\((?P<cls>[^)]+)\)\s*(?:\[(?P<mtype>[MK]):\s*(?P<mpct>\d+)%\]\s*)?\[H:\s*(?P<hpct>-?\d+)%\](?P<flags>.*)|(?P<invited>.+?)\s+\[Invited\]", "  Bob Smith  (Warrior)      [H:100%]  - Frontrank"),
    "look_equipped": ("equip_row", "room", r"(?P<item>.+?)\s+\((?P<slot>[^)]+)\)", "padded vest                    (Torso)"),
    "who_rule": ("who_row", "system", r"(?:(?P<align>[A-Z][A-Za-z]+) )?(?P<name>[A-Z]\w*)(?: (?P<last>\S+))?\s+-\s+(?P<title>.+?)(?:\s+of (?P<gang>.+))?", "Good Nester TheDupe   -  Monk  of Old Guard"),
    "st_spells_on": ("spells_on_row", "status", r"(?P<spells>.+)", "  bless (3,10), mend.", 3),
    "kai_learned": ("kai_row", "spell", r"(?P<ability>.+)", "Kai: punch", 12),
    "train_receive": ("train_row", "training", r"(?P<what>.+)", "10 additional character points", 3),
}

# joining wrapped displays: event-name prefix -> continuation rule
JOIN = [
    ("Obvious exits:", "exits"),
    ("You notice ", "dot_here"),
    ("Also here:", "dot"),
    ("You are carrying ", "until_have"),
    ("You have the following keys:", "dot"),
]
EXIT_TAIL = ("door", "gate", "passage", "trap", "closed", "open", "secret", "Secret")

# monster-message roles (the monster records' message texts, shown in combat and movement).
# Arguments in the order the game fills them in.
ROLE_ARGS = {
    "mon_hits_you": ("mon", "dmg"), "mon_hits_other": ("mon", "victim", "dmg"),
    "mon_deflect_you": ("mon", "verb"), "mon_deflect_other": ("mon", "verb", "victim", "pron"),
    "mon_dodge_you": ("mon", "verb", "weapon"), "mon_dodge_other": ("mon", "verb", "victim", "weapon", "pron"),
    "mon_miss_you": ("mon", "verb", "weapon"), "mon_miss_other": ("mon", "verb", "victim", "weapon"),
    "monster_arrives": ("mon", "frm"), "monster_leaves": ("mon", "to"), "monster_follows": (),
    "monster_death": (),
    # spell texts: order as most records use them; a few records differ
    "spell_cast_you": ("spell", "target", "dmg"), "spell_cast_on_you": ("caster", "spell", "dmg"),
    "spell_cast_room": ("caster", "spell", "target", "dmg"),
}
ROLE_CAT = {"monster_arrives": "movement", "monster_leaves": "movement", "monster_follows": "movement",
            "monster_death": "combat"}


# --------------------------------------------------------------------------------------------
# Pattern objects and the index
# --------------------------------------------------------------------------------------------
class Pattern:
    __slots__ = ("name", "cat", "rx", "example", "seen", "prio", "key", "spec", "role",
                 "argmap", "msgnum", "extra")

    def __init__(self, name, cat, rx, example="", seen="C", prio=50, role=None,
                 argmap=None, msgnum=None):
        self.name, self.cat, self.example, self.seen = name, cat, example, seen
        self.rx = rx if hasattr(rx, "fullmatch") else re.compile(rx)
        self.prio, self.role, self.argmap, self.msgnum = prio, role, argmap, msgnum
        self.key = None
        self.extra = None
        self.spec = len(re.sub(r"\(\?P<\w+>|\\.|[()?*+.|\[\]{}^$]", "", self.rx.pattern))

    @property
    def groups(self):
        return list(self.rx.groupindex)

    def __repr__(self):
        return f"<Pattern {self.name} [{self.cat}] {self.rx.pattern[:60]!r}>"


class Event:
    __slots__ = ("name", "cat", "fields", "line", "lineno", "pattern")

    def __init__(self, name, cat, fields, line, lineno, pattern=None):
        self.name, self.cat, self.fields, self.line, self.lineno = name, cat, fields, line, lineno
        self.pattern = pattern

    def __repr__(self):
        return f"Event({self.name}, {self.fields}, {self.line!r})"


def _regex_tokens(src):
    """Literal words of a regex that are always present: depth-0, space-bounded, no metachar."""
    out, cur, depth, i, ok = [], [], 0, 0, True
    s = src
    while i < len(s):
        c = s[i]
        if c == "\\" and i + 1 < len(s):
            nxt = s[i + 1]
            if nxt in ".!*()[]?+{}|$^-/\"'#&%:,;=<>@ ":
                if depth == 0:
                    cur.append(nxt)
            else:          # \d \s \w ...: not literal
                ok = False
                cur.append("\0")
            i += 2
            continue
        if c == "[":
            j = s.find("]", i + 1)
            j = len(s) if j < 0 else j
            cur.append("\0")
            i = j + 1
            continue
        if c == "(":
            depth += 1
            cur.append("\0")
        elif c == ")":
            depth -= 1
            cur.append("\0")
        elif c in ".*+?{}|^$":
            if c == "|" and depth == 0:
                return []  # top-level alternation: no always-present word
            cur.append("\0")
        elif depth == 0:
            cur.append(c)
        else:
            cur.append("\0")
        i += 1
    text = "".join(cur)
    # quantifier after a literal char makes it optional: crude guard handled by \0 markers
    words = []
    parts = text.split(" ")
    for p in parts:
        if p and "\0" not in p:
            words.append(p)
    return words


def _template_tokens(t):
    return [w for w in t.split(" ") if w and "%" not in w]


def template_regex(t, argnames=()):
    """WCCMSG text -> regex source.  %s -> lazy group, %d -> number; groups named after the
    role's arguments where known (s1.. / n1.. otherwise).  The game upper-cases the first
    letter of monster combat lines, so the first letter matches either case."""
    out, si, used = [], 0, set()
    tail = re.search(r"(?:\s*%s)+$", t)
    opt_tail = ""
    if tail and tail.start() > 0:
        t = t[:tail.start()]
        opt_tail = r"(?:\s.*)?"
    parts = re.split(r"(%[sd]|%%)", t)
    first = True
    has_d = "%d" in t and "dmg" in argnames
    snames = [a for a in argnames if not (has_d and a == "dmg")]
    for p in parts:
        if p in ("%s", "%d"):
            if p == "%d" and has_d:
                nm = "dmg"
            else:
                nm = snames[si] if si < len(snames) else f"{'s' if p == '%s' else 'n'}{si + 1}"
                si += 1
            rx = r".*?" if p == "%s" else r"-?\d+"
            if nm in used or not nm:
                out.append(f"(?:{rx})")
            else:
                used.add(nm)
                out.append(f"(?P<{nm}>{rx})")
        elif p == "%%":
            out.append("%")
        elif p:
            if first and p[0].isalpha():
                out.append(f"[{p[0].upper()}{p[0].lower()}]" + re.escape(p[1:]))
            else:
                out.append(re.escape(p))
        first = False
    return "".join(out) + opt_tail


SENT = "\x00>"   # a line starting with this marks a command the client sent (for echo detection)


def sent_marker(cmd):
    """Line to splice into a line list before the output of a command typed by the client."""
    return SENT + cmd


def _take_echo(pending, s):
    """Is s the echo of one of the commands still waiting for theirs?  (MBBS echoes what is
    typed; type-ahead can also land inside other output, which is not handled here.)"""
    for k, c in enumerate(pending):
        if c == s:
            for _ in range(k + 1):
                pending.popleft()
            return True
    return False


class Registry:
    def __init__(self, prompt=None, room_names=()):
        self.patterns = []
        self.byname = {}
        self.prompt = prompt or PromptParser("std")
        self.room_names = set(room_names)
        self.room_name_lower = {}
        self.death_texts = collections.defaultdict(list)   # literal death text -> monster names
        self.monster_names = set()
        self.verbs = {"hit": set(), "hit3": set(), "miss": set(), "miss3": set()}
        self.index = {}
        self.always = []
        self.tables = {}

    # ---- building ----------------------------------------------------------------------------
    def add(self, p):
        self.patterns.append(p)
        self.byname.setdefault(p.name, p)
        return p

    @classmethod
    def load(cls, data=True, prompt=None, world=None):
        reg = cls(prompt=prompt)
        if data:
            try:
                if world is None:
                    sys.path.insert(0, HERE)
                    import mmud_data as md
                    world = md.World()
                reg.load_world(world)
            except Exception as e:   # noqa: BLE001  (no data: static patterns only)
                print(f"patterns: game data not loaded ({e})", file=sys.stderr)
        reg.load_static()
        reg.build()
        return reg

    def load_static(self):
        hit = "|".join(sorted(map(re.escape, self.verbs["hit"]), key=len, reverse=True)) or r"[a-z]+(?: [a-z]+)?"
        miss3 = "|".join(sorted(map(re.escape, self.verbs["miss3"]), key=len, reverse=True)) or r"[a-z]+ at"
        hit3 = "|".join(sorted(map(re.escape, self.verbs["hit3"]), key=len, reverse=True)) or r"[a-z]+(?: [a-z]+)?"
        miss = "|".join(sorted(map(re.escape, self.verbs["miss"]), key=len, reverse=True)) or r"[a-z]+ at"
        for cat, name, rx, ex, seen in STATIC:
            rx = rx.replace("{HITV}", hit).replace("{HITV3}", hit3).replace("{MISSV3}", miss3).replace("{MISSV}", miss)
            prio = 90 if name.endswith("_generic") or name in ("not_here", "why_would_you", "set_confirm",
                                                               "summoned_appears", "look_player", "other_hits",
                                                               "hand_over_generic", "dont_have", "not_have_any",
                                                               "syntax", "following", "settings_line",
                                                               "chargen_box", "drops_unconscious",
                                                               "other_picks_item", "look_monster") else 50
            if name.startswith(("other_", "player_")) or name.endswith("_builtin"):
                prio = 52 if name == "other_hits" else (80 if name.endswith("_builtin") else 60)
            self.add(Pattern(name, cat, rx, ex, seen, prio))
        for opener, spec in TABLES.items():
            name, cat, rx, ex = spec[:4]
            p = Pattern(name, cat, rx, ex, "C", 50)
            p.extra = None
            p.argmap = spec[4] if len(spec) > 4 else 999    # max rows
            self.tables[opener] = p
            self.byname[name] = p

    def load_world(self, w):
        """Patterns from the game data: monster texts, weapon verbs, spell texts, every other
        WCCMSG text, room names (titles) and Text-exit phrases."""
        self.room_names = {r.name for r in w.rooms.values()}
        msgs = w.msgs
        seen_tmpl = {}

        def add_t(text, role, cat, prio, num, mon=None):
            text = text.rstrip()
            if not text or len(text) < 4:
                return
            if role == "monster_death" or ("%" not in text and role in ("monster_follows",)):
                if role == "monster_death" and mon:
                    self.death_texts[text].append(mon)
            toks = _template_tokens(text)
            if not any(len(t) >= 3 for t in toks):
                return
            old = seen_tmpl.get(text)
            if old is not None and old.prio <= prio:
                return
            args = ROLE_ARGS.get(role, ())
            try:
                rx = re.compile(template_regex(text, args))
            except re.error:
                return
            p = Pattern(role, cat, rx, text, "C", prio, role=role, msgnum=num)
            p.extra = text
            if old is not None:
                self.patterns.remove(old)
            seen_tmpl[text] = p
            self.add(p)

        # monsters (fields of the monster records)
        import mmud_data as md
        for d in md._rows("WCCKNMSR"):
            name = md._s(d[0x36:0x36 + 30])
            self.monster_names.add(name.lower())
            arr, dth = struct.unpack_from("<ii", d, 0xB2)
            hits = struct.unpack_from("<5i", d, 0x146)
            defs = struct.unpack_from("<5i", d, 0x15A)
            miss = struct.unpack_from("<5i", d, 0x16E)
            types = d[0x11E:0x123]
            if arr in msgs:
                a = msgs[arr]
                add_t(a[0], "monster_arrives", "movement", 55, arr)
                add_t(a[1], "monster_leaves", "movement", 55, arr)
                add_t(a[2], "monster_follows", "movement", 55, arr)
            if dth in msgs:
                add_t(msgs[dth][2], "monster_death", "combat", 55, dth, name)
            for t, h, df, ms in zip(types, hits, defs, miss):
                if not t:
                    continue
                if h in msgs:
                    add_t(msgs[h][0], "mon_hits_you", "combat", 55, h)
                    add_t(msgs[h][1], "mon_hits_other", "combat", 56, h)
                if df in msgs:
                    add_t(msgs[df][0], "mon_deflect_you", "combat", 55, df)
                    add_t(msgs[df][1], "mon_deflect_other", "combat", 56, df)
                    add_t(msgs[df][2], "mon_dodge_you", "combat", 55, df)
                if ms in msgs:
                    add_t(msgs[ms][0], "mon_dodge_other", "combat", 56, ms)
                    add_t(msgs[ms][1], "mon_miss_you", "combat", 55, ms)
                    add_t(msgs[ms][2], "mon_miss_other", "combat", 56, ms)
        # weapon verbs (the item records' hit/miss texts)
        for d in md._rows("WCCITEMS"):
            h, mi = struct.unpack_from("<ii", d, 0x404)
            dm, = struct.unpack_from("<i", d, 0x410)
            for num, kind in ((h, "hit"), (mi, "miss")):
                m = msgs.get(num)
                if m:
                    self.verbs[kind].update(v.strip() for v in m[0].split("|") if v.strip())
                    self.verbs[kind + "3"].update(v.strip() for v in m[2].split("|") if v.strip())
            if dm in msgs:
                add_t(msgs[dm][0], "item_destroyed", "equipment", 58, dm)
                add_t(msgs[dm][1], "item_destroyed_other", "equipment", 59, dm)
        # built-in verbs: fists, kicks, default swing
        self.verbs["hit"].update(["punch", "kick", "jumpkick", "attack"])
        self.verbs["hit3"].update(["punches", "kicks", "jumpkicks", "attacks"])
        self.verbs["miss"].update(["swing at"])
        self.verbs["miss3"].update(["swings at"])
        # spells: cast texts to caster / target / room
        for d in md._rows("WCCSPELS"):
            n, = struct.unpack_from("<i", d, 0xE7)
            m = msgs.get(n)
            if m:
                for t in m:
                    role = ("spell_cast_you" if t.startswith("You ") else
                            "spell_cast_on_you" if re.search(r"\byou(?:r)?\b", t) else "spell_cast_room")
                    add_t(t, role, "spell", 57, n)
        # every other WCCMSG text (spell start/end texts, quests, room actions, potions, ...)
        for num, m in msgs.items():
            for i, t in enumerate(m):
                add_t(t, "wccmsg", "message", 70, num)

    def build(self):
        """Choose each pattern's key (rarest always-present word) and build the index."""
        freq = collections.Counter()
        toks = {}
        for p in self.patterns + list(self.tables.values()):
            ws = _template_tokens(p.extra) if p.extra is not None else _regex_tokens(p.rx.pattern)
            # a template's first word may be re-cased; skip it if it starts with a letter
            if p.extra is not None and ws and p.extra.startswith(ws[0]) and ws[0][:1].isalpha():
                ws = ws[1:] or ws
            toks[id(p)] = ws
            freq.update(set(ws))
        self.index = collections.defaultdict(list)
        self.always = []
        for p in self.patterns:
            ws = [w for w in toks[id(p)] if len(w) >= 2]
            if ws:
                p.key = min(ws, key=lambda w: (freq[w], -len(w)))
                self.index[p.key].append(p)
            else:
                self.always.append(p)
        for lst in self.index.values():
            lst.sort(key=lambda p: (p.prio, -p.spec))
        self.always.sort(key=lambda p: (p.prio, -p.spec))
        self.room_name_lower = {n.lower(): n for n in self.room_names}

    # ---- matching -----------------------------------------------------------------------------
    def match_line(self, line):
        """(Pattern, match) for one line (already stripped), or (None, None)."""
        cands = []
        seen = set()
        for w in set(line.split(" ")):
            lst = self.index.get(w)
            if lst:
                for p in lst:
                    if id(p) not in seen:
                        seen.add(id(p))
                        cands.append(p)
        cands.extend(self.always)
        cands.sort(key=lambda p: (p.prio, -p.spec))
        for p in cands:
            m = p.rx.fullmatch(line)
            if m:
                return p, m
        return None, None

    def fields(self, p, m):
        d = {k: v for k, v in m.groupdict().items() if v is not None}
        if p.role == "monster_death":
            d["monsters"] = sorted(set(self.death_texts.get(p.extra, [])))
        if "mon" in d and self.monster_names:
            self._fix_monster(d)
        if p.msgnum is not None:
            d["msgnum"] = p.msgnum
        return d

    def _fix_monster(self, d):
        """'The %s %slunges at' leaves the split between the monster's name and a variable verb
        to the lazy regex ('big' / 'lashworm lunges at').  Re-split on the known monster names
        (with one leading adjective allowed): the longest prefix that names a monster wins."""
        mon = d["mon"].strip()
        verb = d.get("verb", "")
        words = (mon + " " + verb).split() if verb else mon.split()
        best = None
        for k in range(1, len(words) + 1):
            adj, base = split_monster(" ".join(words[:k]), self.monster_names)
            if base in self.monster_names:
                best = k
        if best:
            d["mon"] = " ".join(words[:best])
            if verb:
                d["verb"] = " ".join(words[best:])
            adj, base = split_monster(d["mon"], self.monster_names)
            d["base"], d["adj"] = base, adj

    def classify(self, text, sent=(), raw=None):
        """Text -> [Event] in order.  text: plain text (bridge.plain / transcript) or raw
        bytes/str with ANSI (raw=True, or bytes).  sent: recent commands we typed, so their
        echo is reported as 'echo' instead of being misread."""
        if raw is None:
            raw = isinstance(text, bytes) or "\x1b[" in text
        lines = raw_to_lines(text) if raw else plain_to_lines(text)
        return list(self.iter_events(lines, sent))

    def iter_events(self, lines, sent=()):
        pending_echo = collections.deque(s.strip() for s in sent)
        table, table_rows = None, 0
        after_title = False
        i, n = 0, len(lines)
        while i < n:
            line = lines[i].replace("\x07", "")
            lineno = i
            i += 1
            if line.startswith(SENT):          # marker: the client typed this (see sent_marker)
                pending_echo.append(line[len(SENT):].strip())
                while len(pending_echo) > 4:
                    pending_echo.popleft()
                continue
            # prompts at the start of the line (several can pile up on one line)
            while True:
                pf, rest = self.prompt.peel(line)
                if pf is None:
                    break
                if not pf.get("partial"):
                    yield Event("prompt", "prompt", pf, line, lineno)
                after_title = False
                table = None
                line = rest
                if rest.strip() and _take_echo(pending_echo, rest.strip()):
                    yield Event("echo", "echo", {"cmd": rest.strip()}, rest, lineno)
                    line = ""
                    break
            s = line.strip()
            if not s:
                continue
            if _take_echo(pending_echo, s):
                yield Event("echo", "echo", {"cmd": s}, line, lineno)
                continue
            # wrapped displays
            for prefix, rule in JOIN:
                if s.startswith(prefix):
                    s, i = self._join(s, lines, i, rule)
                    break
            # table rows
            if table is not None:
                m = table.rx.fullmatch(s) if table_rows < table.argmap else None
                if m:
                    table_rows += 1
                    yield Event(table.name, table.cat, {k: v for k, v in m.groupdict().items() if v is not None}, s, lineno, table)
                    continue
                table = None
            # room title (exact name from the room data)
            if s in self.room_names:
                after_title = True
                yield Event("room_title", "room", {"name": s}, s, lineno)
                continue
            p, m = self.match_line(s)
            if p is not None:
                if p.name in self.tables:
                    table, table_rows = self.tables[p.name], 0
                if p.cat == "room" and p.name.startswith("room_"):
                    after_title = False
                yield Event(p.name, p.cat, self.fields(p, m), s, lineno, p)
                continue
            if after_title:
                yield Event("room_desc", "room", {}, s, lineno)
                continue
            yield Event("unknown", "unclassified", {}, s, lineno)

    @staticmethod
    def _join(s, lines, i, rule):
        n = len(lines)
        for _ in range(12):
            if i >= n:
                break
            nxt = lines[i].strip()
            if rule == "exits":
                last = s.rsplit(" ", 1)[-1]
                if not (s.endswith(",") or last in EXIT_TAIL) or not nxt or nxt.startswith("["):
                    break
            elif rule == "dot_here":
                if s.endswith(" here.") or not nxt or nxt.startswith("["):
                    break
            elif rule == "dot":
                if s.endswith(".") or not nxt or nxt.startswith("["):
                    break
            elif rule == "until_have":
                if not nxt or nxt.startswith(("You have", "Wealth:", "[")):
                    break
            s = s + " " + nxt
            i += 1
        return s, i


# --------------------------------------------------------------------------------------------
# Field helpers
# --------------------------------------------------------------------------------------------
DIRS = ["north", "south", "east", "west", "northeast", "northwest", "southeast", "southwest",
        "up", "down"]
_EXIT_RE = re.compile(r"(?:(?P<state>closed|open) (?P<kind>door|gate|trap door) |(?P<secret>[Ss]ecret passage) )?"
                      r"(?P<dir>north|south|east|west|northeast|northwest|southeast|southwest|up|down|above|below|East)")


def parse_exits(exits):
    """'closed door north, up, open trap door below' -> [(dir, kind, state)].
    kind: 'normal' | 'door' | 'gate' | 'trap door' | 'secret' | 'custom' (a found hidden exit
    with its own WCCMSG text: dir unknown).  Trap doors name up/down as above/below."""
    out = []
    if exits.strip() == "NONE!!!":
        return out
    for part in exits.split(","):
        part = part.strip().rstrip(".")
        if not part:
            continue
        m = _EXIT_RE.fullmatch(part)
        if not m:
            out.append((None, "custom", part))
            continue
        d = m.group("dir").lower()
        d = {"above": "up", "below": "down"}.get(d, d)
        kind = m.group("kind") or ("secret" if m.group("secret") else "normal")
        out.append((d, kind, m.group("state")))
    return out


_SLOT_RE = re.compile(r"(?P<name>.+?) \((?P<slot>Two handed|Weapon Hand|Readied/\d+|Nowhere|Worn|Head|Hands|"
                      r"Finger|Feet|Arms|Back|Neck|Legs|Waist|Torso|Off-Hand|Wrist|Ears|Eyes|Face)\)")
_COIN_ITEM = re.compile(rf"(?P<n>\d+) (?P<coin>{COIN})")


def parse_carrying(items):
    """'You are carrying' list -> (coins {name: n}, [(item, slot or None, count)]).
    Slots: the worn-location names, '(Readied/N)' (uses/10), '(Two handed)', '(Weapon Hand)'."""
    coins, out = {}, []
    if items.strip() in ("Nothing!", "Nothing"):
        return coins, out
    for part in items.split(", "):
        part = part.strip()
        m = _COIN_ITEM.fullmatch(part)
        if m:
            coins[m.group("coin")] = int(m.group("n"))
            continue
        m = _SLOT_RE.fullmatch(part)
        if m:
            out.append((m.group("name"), m.group("slot"), 1))
            continue
        m = re.fullmatch(r"(\d+) (.+)", part)
        if m:
            out.append((m.group(2), None, int(m.group(1))))
        else:
            out.append((part, None, 1))
    return coins, out


def parse_also_here(who):
    """'Also here:' list -> [(name, is_player, flags)].  Players are capitalised first names,
    with '*' (outlaw/evil star) and '(Hidden)'; monsters may carry ' (Charmed)'.
    Some monsters are capitalised too (named NPCs): check the
    monster name list before deciding."""
    out = []
    for part in who.split(", "):
        part = part.strip()
        flags = set()
        for f in ("(Hidden)", " (Charmed)", "*"):
            if part.endswith(f):
                flags.add(f.strip(" ()*") or "star")
                part = part[: -len(f)]
        out.append((part, part[:1].isupper() and " " not in part, flags))
    return out


def split_monster(name, known):
    """'fierce kobold thief' -> ('fierce', 'kobold thief') using the known monster names."""
    words = name.split()
    for i in range(len(words)):
        base = " ".join(words[i:]).lower()
        if base in known:
            return " ".join(words[:i]), base
    return "", name.lower()


# --------------------------------------------------------------------------------------------
# Export (machine-readable registry for the server project's screen-snapshot tests)
# --------------------------------------------------------------------------------------------
VERB_PLACEHOLDERS = ("{HITV}", "{HITV3}", "{MISSV}", "{MISSV3}")

TEMPLATE_RULES = (
    "Generated families (WCCMSG texts) are built from the game database by template_regex(): "
    "split the template on %s, %d and %%; %s becomes a lazy group (.*?), %d becomes -?\\d+, "
    "%% becomes a literal percent sign; a group is named after the role's argument (see "
    "roles.args) or s<N>/n<N> otherwise, and a repeated name becomes a non-capturing group; "
    "the first letter of the text matches either case ([Xx]); literal text is re.escape()d; a "
    "trailing run of %s is dropped and replaced by an optional (?:\\s.*)? tail."
)

CONTEXT_RULES = {
    "room_title": "A line equal to a room name from the game data; not a regex.",
    "room_desc": "Any unmatched line after a room title and before the next room_* pattern; "
                 "a context rule, not a regex.",
}


def _expand_placeholders(rx):
    for ph in VERB_PLACEHOLDERS:
        rx = rx.replace(ph, "x")
    return rx


def export_registry(with_data=False):
    """The static registry as a JSON-ready dict (see tests/fixtures/patterns.json)."""
    reg = Registry.load(data=with_data)
    static = []
    for cat, name, rx, ex, seen in sorted(STATIC, key=lambda t: (t[0], t[1])):
        static.append({"name": name, "cat": cat, "rx": rx, "example": ex,
                       "seen": seen, "prio": reg.byname[name].prio,
                       "groups": list(re.compile(_expand_placeholders(rx)).groupindex)})
    tables = []
    for opener in sorted(TABLES):
        spec = TABLES[opener]
        name, cat, rx, ex = spec[:4]
        tables.append({"opener": opener, "row_name": name, "cat": cat, "rx": rx,
                       "example": ex, "max_rows": spec[4] if len(spec) > 4 else None})
    known_failures = []
    for cat, name, rx, ex, seen in STATIC:
        if ex and "..." not in ex and seen != "I":
            p, m = reg.match_line(ex.strip())
            if p is None:
                known_failures.append({"name": name, "example": ex, "matched": None})
            elif p.name != name and p.prio >= reg.byname[name].prio and p.cat != cat:
                known_failures.append({"name": name, "example": ex, "matched": p.name})
    verbs = {k: sorted(reg.verbs[k]) if with_data else [] for k in ("hit", "hit3", "miss", "miss3")}
    return {
        "version": 1,
        "generated_from": "tools/differential/vendor/play/patterns.py",
        "counts": {
            "static": len(static), "tables": len(tables), "join": len(JOIN),
            "by_seen": dict(sorted(collections.Counter(t[4] for t in STATIC).items())),
            "by_cat": dict(sorted(collections.Counter(t[0] for t in STATIC).items())),
            "verbs_available": bool(with_data),
        },
        "static": static,
        "tables": tables,
        "join": [{"prefix": p, "rule": r} for p, r in JOIN],
        "exit_tail": list(EXIT_TAIL),
        "prompt": {
            "std": STD_PROMPT,
            "generic": GENERIC_PROMPT.pattern,
            "statline_codes": {k: {"field": v[0], "rx": v[1]} for k, v in STATLINE_CODES.items()},
            "statline_len": STATLINE_LEN,
        },
        "roles": {"args": {k: list(v) for k, v in ROLE_ARGS.items()}, "cat": ROLE_CAT},
        "verbs": verbs,
        "template_rules": TEMPLATE_RULES,
        "context": CONTEXT_RULES,
        "known_failures": known_failures,
    }


def export_main(argv):
    import argparse
    import json
    ap = argparse.ArgumentParser(prog="patterns.py export",
                                 description="Write the static pattern registry as JSON")
    ap.add_argument("--out", help="output path (default: stdout)")
    ap.add_argument("--with-data", action="store_true", help="also export the weapon verb lists (needs game data)")
    a = ap.parse_args(argv)
    text = json.dumps(export_registry(a.with_data), indent=1, ensure_ascii=False, sort_keys=False) + "\n"
    if a.out:
        with open(a.out, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    return 0


# --------------------------------------------------------------------------------------------
# Self-test
# --------------------------------------------------------------------------------------------
def _norm(s):
    return re.sub(r"\d+", "N", s)


def selftest(argv):
    import argparse
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--top", type=int, default=40)
    ap.add_argument("--raw", action="store_true", help="read raw.bin (ANSI) instead of transcript.txt")
    ap.add_argument("--logs", nargs="*", help="log directories (default logs/* next to this script)")
    ap.add_argument("--no-data", action="store_true", help="static patterns only")
    ap.add_argument("--events", action="store_true", help="list every event name with its count")
    a = ap.parse_args(argv)

    t0 = time.time()
    reg = Registry.load(data=not a.no_data)
    t1 = time.time()
    print(f"registry: {len(reg.patterns)} patterns ({sum(1 for p in reg.patterns if p.extra is None)} static, "
          f"{sum(1 for p in reg.patterns if p.extra is not None)} from game data), "
          f"{len(reg.tables)} table-row patterns, {len(reg.index)} keys, {len(reg.always)} unkeyed; "
          f"{len(reg.room_names)} room titles; built in {t1 - t0:.2f}s")
    self_check = []
    for cat, name, rx, ex, seen in STATIC:
        if ex and "..." not in ex and seen != "I":
            p, m = reg.match_line(ex.strip())
            if p is None:
                self_check.append((name, ex, None))
            elif p.name != name and p.prio >= reg.byname[name].prio and p.cat != cat:
                self_check.append((name, ex, p.name))
    if self_check:
        print(f"\nexamples not matched by their own pattern ({len(self_check)}):")
        for name, ex, got in self_check:
            print(f"  {name:24} {got or '-':24} {ex}")

    dirs = a.logs or sorted(glob.glob(os.path.join(HERE, "logs", "*")))
    counts, cats = collections.Counter(), collections.Counter()
    unknown = collections.Counter()
    nlines = 0
    t2 = time.time()
    for d in dirs:
        if a.raw:
            f = os.path.join(d, "raw.bin")
            if not os.path.exists(f):
                continue
            lines = raw_to_lines(open(f, "rb").read())
            sent = ()
        else:
            f = os.path.join(d, "transcript.txt")
            if not os.path.exists(f):
                continue
            src = open(f, encoding="utf-8", errors="replace").read().split("\n")
            lines, sent = [], ()
            for l in src:
                if l.startswith("### "):      # bot annotations
                    continue
                if l.startswith(">>> "):      # what the bot typed
                    m = re.match(r">>> '(.*)'$", l)
                    if m:
                        lines.append(sent_marker(m.group(1).encode().decode("unicode_escape")))
                    continue
                lines.append(l)
        nlines += len(lines)
        for ev in reg.iter_events(lines, sent):
            counts[ev.name] += 1
            cats[ev.cat] += 1
            if ev.name == "unknown":
                unknown[_norm(ev.line)] += 1
    t3 = time.time()
    total = sum(counts.values())
    print(f"\n{nlines} lines from {len(dirs)} logs -> {total} events in {t3 - t2:.2f}s "
          f"({(t3 - t2) / max(1, nlines) * 1e6:.0f} us/line)")
    print("per category: " + ", ".join(f"{c} {n}" for c, n in cats.most_common()))
    print(f"unclassified: {counts['unknown']} lines ({counts['unknown'] / max(1, total):.1%}), "
          f"{len(unknown)} distinct")
    if a.events:
        print("\nper event:")
        for k, v in sorted(counts.items(), key=lambda kv: -kv[1]):
            print(f"  {v:7} {k}")
    else:
        print("\ntop events: " + ", ".join(f"{k} {v}" for k, v in counts.most_common(30)))
    print(f"\ntop {a.top} unclassified:")
    for line, c in unknown.most_common(a.top):
        print(f"  {c:6}  {line[:110]}")
    return 0


if __name__ == "__main__":
    if sys.argv[1:2] == ["export"]:
        sys.exit(export_main(sys.argv[2:]))
    sys.exit(selftest(sys.argv[1:]))
