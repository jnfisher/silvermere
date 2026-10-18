#!/usr/bin/env python3
# Copied from the project's MBBSEmu test harness; edit here; see vendor/SOURCE.md. Stdlib only.
"""
Read-only access to MajorMUD's game data as MBBSEmu stores it (one SQLite file per Btrieve
file, record bytes in data_t.data).  Offsets follow the game's record layouts (byte-packed).

  mmud_data.py room NAME|MAP:ROOM        rooms matching a name, with exits and spawn rules
  mmud_data.py path FROM TO              shortest walk (MAP:ROOM or exact room name)
  mmud_data.py monster NAME              known monsters matching a name
  mmud_data.py shops [TYPE]              shops (Training, Weapons, Armour, ...) and where they are
  mmud_data.py hunt --hp N [--map 1]     rank hunting areas for a character with N max HP
  mmud_data.py export --data DIR --out DIR [--tables T ...]   every record as JSON lines (T-006)
"""
import argparse
import base64
import collections
import datetime
import gzip
import hashlib
import json
import os
import sqlite3
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "..", "run", "built", "WCCMMUD")

DIRS = ["north", "south", "east", "west", "northeast", "northwest", "southeast", "southwest",
        "up", "down"]
DIRCMD = ["n", "s", "e", "w", "ne", "nw", "se", "sw", "u", "d"]
EXITTYPES = ["Normal", "Spell", "Key", "Item", "Toll", "Action", "Hidden", "Door", "Map Change",
             "Trap", "Text", "Gate", "Remote Action", "Class", "Race", "Level", "Timed",
             "Ticket", "User Count", "Block Guard", "Alignment", "Delay", "Cast", "Ability",
             "Spell Trap"]
ROOMTYPES = ["Normal", "Shop", "Arena", "Lair", "Hotel", "Colliseum", "Jail", "Library"]
SHOPTYPES = ["General", "Weapons", "Armour", "Items", "Spells", "Hospital", "Tavern", "Bank",
             "Training", "Inn", "Specific", "Gang Shop", "Deed Shop"]
ALIGNMENTS = ["Good", "Evil", "Chaotic Evil", "Neutral", "Lawful Good", "Neutral Evil",
              "Lawful Evil"]
CURRENCY = ["copper", "silver", "gold", "platinum", "runic"]  # item.currency codes (CUR_*)
# exits a plain walking character can use without keys, items, spells or tolls (doors open)
WALKABLE = {"Normal", "Door", "Gate", "Map Change", "Text", "Delay", "Level", "Class", "Race",
            "Alignment", "Trap", "Timed", "User Count", "Block Guard",
            "Action"}  # an Action exit is walked by direction (with a message)


def _s(b):
    return b.split(b"\0")[0].decode("cp437").strip()


def _rows(name):
    c = sqlite3.connect(os.path.join(DATA, name + ".db"))
    return [r[0] for r in c.execute("select data from data_t")]


class Room:
    def __init__(self, d):
        self.map, self.num = struct.unpack_from("<II", d, 0)
        self.name = _s(d[0x105:0x105 + 53])
        self.exits = struct.unpack_from("<10I", d, 0x338)
        self.exittypes = struct.unpack_from("<10h", d, 0x360)
        self.exitpar1 = struct.unpack_from("<10I", d, 0x374)
        self.exitpar2 = struct.unpack_from("<10h", d, 0x39C)
        self.type = struct.unpack_from("<h", d, 0x43C)[0]
        self.shop = struct.unpack_from("<i", d, 0x43E)[0]
        self.minlevel, self.maxlevel = struct.unpack_from("<hh", d, 0x460)
        self.genmonster = struct.unpack_from("<i", d, 0x464)[0]
        self.light = struct.unpack_from("<h", d, 0x468)[0]  # below -150: too dark to see
        self.regendelay = struct.unpack_from("<h", d, 0x5B3)[0]  # minutes, 0 = REGENMINS (5)
        self.maxmonsters = d[0x554]
        self.mongroup = struct.unpack_from("<h", d, 0x557)[0]

    @property
    def key(self):
        return (self.map, self.num)

    def links(self):
        """(direction index, (map, room), exit type name) for each exit."""
        out = []
        for i, dest in enumerate(self.exits):
            if not dest:
                continue
            t = EXITTYPES[self.exittypes[i]] if 0 <= self.exittypes[i] < len(EXITTYPES) else "?"
            m = self.map
            if t == "Map Change" and self.exitpar1[i]:
                m = self.exitpar1[i]  # (inferred) map change: parameter 1 is the target map
            out.append((i, (m, dest), t))
        return out


class Monster:
    def __init__(self, d):
        self.num = struct.unpack_from("<i", d, 0)[0]
        self.name = _s(d[0x36:0x36 + 30])
        self.montype, = struct.unpack_from("<h", d, 0x54)
        self.level, = struct.unpack_from("<I", d, 0x56)
        self.genindex, = struct.unpack_from("<h", d, 0x5A)
        self.dr, self.ac = struct.unpack_from("<hh", d, 0x64)
        self.magicres, = struct.unpack_from("<h", d, 0x6C)
        self.exp, = struct.unpack_from("<i", d, 0x6F)
        self.maxhp, = struct.unpack_from("<h", d, 0x73)
        self.energy, = struct.unpack_from("<H", d, 0x75)
        self.grouprole, = struct.unpack_from("<h", d, 0xA5)
        self.alignment, = struct.unpack_from("<h", d, 0xA9)
        self.cash = struct.unpack_from("<5I", d, 0x100)
        self.atktypes = list(d[0x11E:0x123])
        self.atkpcts = list(d[0x12D:0x132])
        self.atkmins = struct.unpack_from("<5h", d, 0x132)
        self.atkmaxs = struct.unpack_from("<5h", d, 0x13C)
        self.atkenergies = struct.unpack_from("<5h", d, 0x182)  # energy each attack costs

    @property
    def maxhit(self):
        """Largest single melee hit (attack type 1 = physical, inferred)."""
        return max([mx for t, mx in zip(self.atktypes, self.atkmaxs) if t] or [0])

    @property
    def coin_value(self):
        """Average coins dropped, in copper (coins rolled up to the maxima: half on average)."""
        rate = [1000000, 10000, 100, 10, 1]
        return sum(c * r for c, r in zip(self.cash, rate)) / 2


class Shop:
    def __init__(self, d):
        self.num = struct.unpack_from("<i", d, 0)[0]
        self.name = _s(d[4:44])
        self.type, self.minlevel, self.maxlevel, self.markup = struct.unpack_from("<hHHH", d, 0xCB)
        self.trainclass, = struct.unpack_from("<H", d, 0xD5)
        items = struct.unpack_from("<20i", d, 0xD7)
        maxq = struct.unpack_from("<20H", d, 0x127)
        qty = struct.unpack_from("<20H", d, 0x14F)
        self.items = [i for i in items if i]
        # items the shop restocks itself (recyclers only resell what players sold them)
        self.stocked = [i for i, m in zip(items, maxq) if i and m]
        self.qty = {i: q for i, q in zip(items, qty) if i}


class Item:
    def __init__(self, d):
        self.num = struct.unpack_from("<i", d, 0)[0]
        self.name = _s(d[0xAD:0xAD + 30])
        self.weight, self.type = struct.unpack_from("<hh", d, 0x2F2)
        self.cost, = struct.unpack_from("<H", d, 0x322)
        self.classes = [c for c in struct.unpack_from("<10h", d, 0x324) if c]
        self.mindmg, self.maxdmg, self.ac = struct.unpack_from("<hhh", d, 0x33E)
        self.worn = d[0x398]
        self.dr, = struct.unpack_from("<h", d, 0x39B)
        self.strreq, = struct.unpack_from("<h", d, 0x39F)
        self.currency = d[0x421]
        self.charges, = struct.unpack_from("<h", d, 0x31E)
        self.abilities = {a: v for a, v in zip(struct.unpack_from("<20h", d, 0x2F6),
                                               struct.unpack_from("<20h", d, 0x3DC)) if a}


SPELLTARGETS = ["Single", "Self", "Self or Single", "Divided Area Not You", "Monster",
                "Divided Area Incl. You", "Any", "Item", "Monster or Single",
                "Divided Attack Area", "Divided Party Area", "Full Area", "Full Attack Area",
                "Full Party Area"]
SINGLE_TARGETS = {0, 4, 8}      # attack spells cast at one monster
AREA_TARGETS = {3, 5, 9, 11, 12}  # attack spells that hit the room / everyone in the fight


class Spell:
    """WCCSPELS record (a spell)."""
    def __init__(self, d):
        self.num, = struct.unpack_from("<h", d, 0)
        self.name = _s(d[2:32])
        self.levelcap, = struct.unpack_from("<h", d, 0x9F)
        self.energy, self.level = struct.unpack_from("<HH", d, 0xB9)
        self.minval, self.maxval, self.type = struct.unpack_from("<hhh", d, 0xBD)
        self.diff, self.durlvl, self.target, self.duration = struct.unpack_from("<hhhh", d, 0xC4)
        self.magery, = struct.unpack_from("<h", d, 0xD1)
        self.abilities = {a: v for a, v in zip(struct.unpack_from("<10h", d, 0xD3),
                                               struct.unpack_from("<10h", d, 0xA5)) if a}
        self.mana, = struct.unpack_from("<h", d, 0xEB)
        self.maxinc, self.maxlvls = d[0xED], d[0xEE]
        self.magelvl, = struct.unpack_from("<h", d, 0xEF)
        self.mininc, self.minlvls = d[0xF1], d[0xF2]
        self.short = _s(d[0xF5:0xFB]).lower()

    @property
    def attack(self):
        return self.type == 0 and self.target in SINGLE_TARGETS | AREA_TARGETS

    @property
    def area(self):
        return self.type == 0 and self.target in AREA_TARGETS

    @property
    def heal(self):
        return 18 in self.abilities and self.target in (1, 2)  # ABL_HEAL on self

    def damage(self, level):
        """Average damage at a caster level (min/max grow by inc per lvls levels)."""
        lo = self.minval + (self.mininc * level // self.minlvls if self.minlvls else 0)
        hi = self.maxval + (self.maxinc * level // self.maxlvls if self.maxlvls else 0)
        if self.levelcap:
            pass  # (growth stops at the cap; approximate)
        return (lo + hi) / 2


# the record layouts (T-006): (name, offset, kind, dims). kinds: B/b/h/H/i/I = u8/i8/i16/u16/i32/u32,
# s = NUL-terminated CP437 string(s), x = raw bytes as base64. Offsets checked against the record sizes.
RECORDS = {
    "WCCMP001": ("room", 1528, [
        ("mapnum", 0x000, "I", ()),
        ("roomnum", 0x004, "I", ()),
        ("name", 0x105, "s", (53,)),
        ("desc", 0x13A, "s", (7, 71)),
        ("mapfile", 0x32B, "s", (13,)),
        ("exits", 0x338, "I", (10,)),
        ("exittypes", 0x360, "h", (10,)),
        ("exitpar1s", 0x374, "I", (10,)),
        ("exitpar2s", 0x39C, "h", (10,)),
        ("exitpar3s", 0x3B0, "i", (10,)),
        ("exitpar4s", 0x3D8, "i", (10,)),
        ("monsters", 0x400, "I", (15,)),
        ("type", 0x43C, "h", ()),
        ("shopnum", 0x43E, "i", ()),
        ("owner", 0x442, "s", (30,)),
        ("minlevel", 0x460, "h", ()),
        ("maxlevel", 0x462, "h", ()),
        ("genmonster", 0x464, "i", ()),
        ("light", 0x468, "h", ()),
        ("ganghouse", 0x46A, "H", ()),
        ("items", 0x46C, "i", (17,)),
        ("itemuses", 0x4B0, "h", (17,)),
        ("hiditems", 0x4D2, "i", (15,)),
        ("hiduses", 0x50E, "h", (15,)),
        ("runic", 0x52C, "I", ()),
        ("platinum", 0x530, "I", ()),
        ("gold", 0x534, "I", ()),
        ("silver", 0x538, "I", ()),
        ("copper", 0x53C, "I", ()),
        ("hidrunic", 0x540, "I", ()),
        ("hidplatinum", 0x544, "I", ()),
        ("hidgold", 0x548, "I", ()),
        ("hidsilver", 0x54C, "I", ()),
        ("hidcopper", 0x550, "I", ()),
        ("maxmonsters", 0x554, "B", ()),
        ("unk_0x555", 0x555, "h", ()),
        ("mongroup", 0x557, "h", ()),
        ("lastkill", 0x559, "H", ()),
        ("flags", 0x55B, "I", ()),
        ("foundmask", 0x55F, "I", ()),
        ("deathroom", 0x563, "I", ()),
        ("huproom", 0x567, "I", ()),
        ("itemcounts", 0x56B, "h", (17,)),
        ("hidcounts", 0x58D, "h", (15,)),
        ("cmdtext", 0x5AB, "i", ()),
        ("unk_0x5AF", 0x5AF, "i", ()),
        ("regendelay", 0x5B3, "h", ()),
        ("areamax", 0x5B5, "h", ()),
        ("areacount", 0x5B7, "h", ()),
        ("ctlroom", 0x5B9, "I", ()),
        ("specmon", 0x5BD, "i", ()),
        ("placed", 0x5C1, "i", (10,)),
        ("protmsg", 0x5E9, "i", ()),
        ("changed", 0x5ED, "B", ()),
        ("picktrap", 0x5EE, "h", ()),
        ("roomspell", 0x5F1, "i", ()),
        ("searchers", 0x5F6, "B", ()),
        ("nummonsters", 0x5F7, "B", ()),
    ]),
    "WCCITEMS": ("item", 1061, [
        ("number", 0x000, "i", ()),
        ("level", 0x004, "H", ()),
        ("limit", 0x006, "H", ()),
        ("name", 0x0AD, "s", (30,)),
        ("desc", 0x0CB, "s", (6, 61)),
        ("hiddesc", 0x239, "s", (3, 61)),
        ("active", 0x2F0, "H", ()),
        ("weight", 0x2F2, "h", ()),
        ("type", 0x2F4, "h", ()),
        ("abilities", 0x2F6, "h", (20,)),
        ("charges", 0x31E, "h", ()),
        ("energy", 0x320, "h", ()),
        ("cost", 0x322, "H", ()),
        ("classes", 0x324, "h", (10,)),
        ("unk_0x338", 0x338, "h", ()),
        ("unk_0x33a", 0x33A, "h", ()),
        ("unk_0x33c", 0x33C, "h", ()),
        ("mindmg", 0x33E, "h", ()),
        ("maxdmg", 0x340, "h", ()),
        ("ac", 0x342, "h", ()),
        ("races", 0x344, "i", (10,)),
        ("negates", 0x36C, "i", (10,)),
        ("wpntype", 0x394, "h", ()),
        ("armtype", 0x396, "h", ()),
        ("worn", 0x398, "B", ()),
        ("accuracy", 0x399, "h", ()),
        ("dr", 0x39B, "h", ()),
        ("gettable", 0x39D, "h", ()),
        ("strreq", 0x39F, "h", ()),
        ("unk_0x3a1", 0x3A1, "i", ()),
        ("unk_0x3a5", 0x3A5, "i", ()),
        ("unk_0x3a9", 0x3A9, "h", ()),
        ("cash", 0x3AB, "i", (5,)),
        ("unk_0x3d7", 0x3D7, "h", ()),
        ("speed", 0x3D9, "h", ()),
        ("dirty", 0x3DB, "B", ()),
        ("abilvals", 0x3DC, "h", (20,)),
        ("hitmsg", 0x404, "i", ()),
        ("missmsg", 0x408, "i", ()),
        ("textnum", 0x40C, "i", ()),
        ("destroymsg", 0x410, "i", ()),
        ("unk_0x414", 0x414, "i", ()),
        ("unk_0x418", 0x418, "i", ()),
        ("unk_0x41c", 0x41C, "i", ()),
        ("nodrop", 0x420, "B", ()),
        ("currency", 0x421, "B", ()),
        ("retain", 0x422, "B", ()),
        ("robable", 0x423, "B", ()),
        ("destroy", 0x424, "B", ()),
    ]),
    "WCCKNMSR": ("monster", 732, [
        ("number", 0x000, "i", ()),
        ("name", 0x036, "s", (30,)),
        ("montype", 0x054, "h", ()),
        ("level", 0x056, "I", ()),
        ("genindex", 0x05A, "h", ()),
        ("wornitem", 0x05C, "i", ()),
        ("weapon", 0x060, "i", ()),
        ("dr", 0x064, "h", ()),
        ("ac", 0x066, "h", ()),
        ("group", 0x068, "h", ()),
        ("followpct", 0x06A, "h", ()),
        ("magicres", 0x06C, "h", ()),
        ("bsdefense", 0x06E, "B", ()),
        ("exp", 0x06F, "i", ()),
        ("maxhp", 0x073, "h", ()),
        ("energy", 0x075, "H", ()),
        ("hpregen", 0x077, "h", ()),
        ("abilities", 0x079, "h", (10,)),
        ("abilvals", 0x08D, "h", (10,)),
        ("maxalive", 0x0A1, "H", ()),
        ("curalive", 0x0A3, "H", ()),
        ("grouprole", 0x0A5, "h", ()),
        ("maxfollow", 0x0A7, "B", ()),
        ("undead", 0x0A8, "B", ()),
        ("alignment", 0x0A9, "h", ()),
        ("unk_0x0AB", 0x0AB, "h", ()),
        ("regenhrs", 0x0AD, "B", ()),
        ("killdate", 0x0AE, "H", ()),
        ("killtime", 0x0B0, "H", ()),
        ("arrivemsg", 0x0B2, "i", ()),
        ("deathmsg", 0x0B6, "i", ()),
        ("items", 0x0BA, "i", (10,)),
        ("itemuses", 0x0E2, "h", (10,)),
        ("itempcts", 0x0F6, "B", (10,)),
        ("cash", 0x100, "I", (5,)),
        ("talktext", 0x114, "i", ()),
        ("charmlvl", 0x118, "H", ()),
        ("namelist", 0x11A, "i", ()),
        ("atktypes", 0x11E, "B", (5,)),
        ("atkaccs", 0x123, "h", (5,)),
        ("atkpcts", 0x12D, "B", (5,)),
        ("atkmins", 0x132, "h", (5,)),
        ("atkmaxs", 0x13C, "h", (5,)),
        ("hitmsgs", 0x146, "i", (5,)),
        ("defmsgs", 0x15A, "i", (5,)),
        ("missmsgs", 0x16E, "i", (5,)),
        ("atkenergies", 0x182, "h", (5,)),
        ("randtext", 0x18C, "i", ()),
        ("charmres", 0x190, "h", ()),
        ("unk_0x192", 0x192, "B", ()),
        ("hitspells", 0x193, "h", (5,)),
        ("deathspell", 0x19D, "h", ()),
        ("unk_0x19F", 0x19F, "h", ()),
        ("unk_0x1A1", 0x1A1, "h", ()),
        ("unk_0x1A3", 0x1A3, "B", ()),
        ("unk_0x1A4", 0x1A4, "i", ()),
        ("dirty", 0x1A8, "B", ()),
        ("createspell", 0x1A9, "h", ()),
        ("spells", 0x1AB, "h", (5,)),
        ("spellpcts", 0x1B5, "B", (5,)),
        ("spelllvls", 0x1BA, "B", (5,)),
        ("desc", 0x1BF, "s", (4, 71)),
        ("gender", 0x2DB, "B", ()),
    ]),
    "WCCACMSR": ("actmon", 386, [
        ("number", 0x000, "i", ()),
        ("knownnum", 0x004, "i", ()),
        ("level", 0x008, "I", ()),
        ("map", 0x00C, "i", ()),
        ("room", 0x010, "i", ()),
        ("poison", 0x014, "h", ()),
        ("energy", 0x016, "H", ()),
        ("hp", 0x018, "h", ()),
        ("following", 0x01A, "s", (30,)),
        ("trail", 0x038, "i", (10,)),
        ("hunters", 0x060, "i", (10,)),
        ("hunting", 0x088, "i", ()),
        ("name", 0x08E, "s", (30,)),
        ("wornitem", 0x0AC, "i", ()),
        ("weapon", 0x0B0, "i", ()),
        ("items", 0x0B4, "i", (10,)),
        ("itemuses", 0x0DC, "h", (10,)),
        ("cash", 0x0F0, "I", (5,)),
        ("maxhp", 0x104, "h", ()),
        ("alignment", 0x106, "h", ()),
        ("followpct", 0x108, "h", ()),
        ("dr", 0x10A, "h", ()),
        ("ac", 0x10C, "h", ()),
        ("exp", 0x10E, "i", ()),
        ("maxenergy", 0x112, "h", ()),
        ("friendly", 0x114, "B", ()),
        ("genroom", 0x11B, "i", ()),
        ("inactive", 0x11F, "B", ()),
        ("flags", 0x120, "H", ()),
        ("montype", 0x124, "h", ()),
        ("target", 0x126, "h", ()),
        ("hpregen", 0x128, "h", ()),
        ("lastdir", 0x12A, "h", ()),
        ("dirty", 0x135, "B", ()),
        ("lastupd", 0x136, "i", ()),
        ("grouprole", 0x13A, "h", ()),
        ("spells", 0x13C, "h", (5,)),
        ("spellvals", 0x146, "h", (5,)),
        ("spelldurs", 0x150, "h", (5,)),
        ("downrounds", 0x15A, "h", ()),
        ("created", 0x15C, "h", ()),
    ]),
    "WCCSPELS": ("spell", 253, [
        ("number", 0x000, "h", ()),
        ("name", 0x002, "s", (30,)),
        ("descs", 0x020, "s", (2, 51)),
        ("resist", 0x086, "i", ()),
        ("levelcap", 0x09F, "h", ()),
        ("flags", 0x0A1, "B", ()),
        ("abilvals", 0x0A5, "h", (10,)),
        ("energy", 0x0B9, "H", ()),
        ("level", 0x0BB, "H", ()),
        ("minval", 0x0BD, "h", ()),
        ("maxval", 0x0BF, "h", ()),
        ("type", 0x0C1, "h", ()),
        ("resmode", 0x0C3, "B", ()),
        ("diff", 0x0C4, "h", ()),
        ("durlvl", 0x0C6, "h", ()),
        ("target", 0x0C8, "h", ()),
        ("duration", 0x0CA, "h", ()),
        ("element", 0x0CC, "h", ()),
        ("resstat", 0x0D0, "B", ()),
        ("magery", 0x0D1, "h", ()),
        ("abilities", 0x0D3, "h", (10,)),
        ("msgnum", 0x0E7, "i", ()),
        ("mana", 0x0EB, "h", ()),
        ("maxinc", 0x0ED, "B", ()),
        ("maxlvls", 0x0EE, "B", ()),
        ("magelvl", 0x0EF, "h", ()),
        ("mininc", 0x0F1, "B", ()),
        ("minlvls", 0x0F2, "B", ()),
        ("durinc", 0x0F3, "B", ()),
        ("durlvls", 0x0F4, "B", ()),
        ("shortnm", 0x0F5, "s", (6,)),
        ("modified", 0x0FB, "B", ()),
    ]),
    "WCCCLASS": ("class", 153, [
        ("number", 0x000, "H", ()),
        ("name", 0x002, "s", (30,)),
        ("basehp", 0x020, "h", ()),
        ("hplevel", 0x022, "h", ()),
        ("exp", 0x024, "h", ()),
        ("attmult", 0x026, "h", ()),
        ("maxenergy", 0x028, "h", ()),
        ("restored", 0x02A, "h", ()),
        ("abilities", 0x02C, "h", (10,)),
        ("magery", 0x040, "h", ()),
        ("magelvl", 0x042, "h", ()),
        ("weapon", 0x044, "h", ()),
        ("armour", 0x046, "h", ()),
        ("combat", 0x048, "h", ()),
        ("abilvals", 0x04A, "h", (10,)),
        ("unk_0x05E", 0x05E, "x", (6,)),
        ("titles", 0x064, "i", ()),
        ("modified", 0x068, "B", ()),
    ]),
    "WCCRACE": ("race", 126, [
        ("number", 0x000, "H", ()),
        ("name", 0x002, "s", (30,)),
        ("minint", 0x020, "h", ()),
        ("minwil", 0x022, "h", ()),
        ("minstr", 0x024, "h", ()),
        ("minhea", 0x026, "h", ()),
        ("minagi", 0x028, "h", ()),
        ("mincha", 0x02A, "h", ()),
        ("hpbonus", 0x02C, "h", ()),
        ("maxenergy", 0x02E, "h", ()),
        ("restored", 0x030, "h", ()),
        ("abilities", 0x032, "h", (10,)),
        ("basecp", 0x046, "h", ()),
        ("abilvals", 0x048, "h", (10,)),
        ("unk_0x05C", 0x05C, "x", (6,)),
        ("exp", 0x062, "h", ()),
        ("modified", 0x064, "B", ()),
        ("maxint", 0x066, "h", ()),
        ("maxwil", 0x068, "h", ()),
        ("maxstr", 0x06A, "h", ()),
        ("maxhea", 0x06C, "h", ()),
        ("maxagi", 0x06E, "h", ()),
        ("maxcha", 0x070, "h", ()),
    ]),
    "WCCSHOPS": ("shop", 500, [
        ("number", 0x000, "i", ()),
        ("name", 0x004, "s", (40,)),
        ("desc", 0x02C, "s", (3, 53)),
        ("type", 0x0CB, "h", ()),
        ("minlevel", 0x0CD, "H", ()),
        ("maxlevel", 0x0CF, "H", ()),
        ("markup", 0x0D1, "H", ()),
        ("ownable", 0x0D3, "B", ()),
        ("expandable", 0x0D4, "B", ()),
        ("trainclass", 0x0D5, "H", ()),
        ("items", 0x0D7, "i", (20,)),
        ("maxqty", 0x127, "H", (20,)),
        ("qty", 0x14F, "H", (20,)),
        ("regentime", 0x177, "H", (20,)),
        ("regennum", 0x19F, "h", (20,)),
        ("regenpct", 0x1C7, "B", (20,)),
        ("dirty", 0x1DB, "B", ()),
    ]),
    "WCCMSG": ("mudmsg", 253, [
        ("number", 0x000, "i", ()),
        ("msgs", 0x004, "s", (3, 82)),
        ("modified", 0x0FA, "B", ()),
    ]),
    "WCCTEXT": ("textblk", 2022, [
        ("part", 0x000, "h", ()),
        ("number", 0x00E, "i", ()),
        ("link", 0x012, "i", ()),
        ("text_raw", 0x016, "x", (2000,)),
    ]),
    "WCCACTS": ("action", 1010, [
        ("name", 0x000, "s", (30,)),
        ("texts", 0x01E, "s", (11, 80)),
        ("targets", 0x38E, "h", ()),
    ]),
    "WCCGANGS": ("gang", 253, [
        ("key", 0x000, "s", (20,)),
        ("name", 0x014, "s", (20,)),
        ("exp", 0x028, "I", ()),
        ("leader", 0x02C, "s", (30,)),
        ("created", 0x04A, "h", ()),
        ("members", 0x04E, "h", ()),
        ("flags", 0x050, "i", ()),
        ("modified", 0x054, "B", ()),
        ("overexp", 0x055, "I", ()),
        ("overflows", 0x059, "h", ()),
    ]),
    "WCCBANKS": ("bankbook", 72, [
        ("userid", 0x000, "s", (30,)),
        ("bank", 0x01E, "i", ()),
        ("balance", 0x022, "I", ()),
        ("loan", 0x026, "I", ()),
        ("dirty", 0x02A, "B", ()),
    ]),
    "WCCITOWN": ("itemown", 65, [
        ("id", 0x000, "i", ()),
        ("itemnum", 0x004, "i", ()),
        ("owntype", 0x008, "B", ()),
        ("mapnum", 0x009, "i", ()),
        ("roomnum", 0x00D, "i", ()),
        ("userid", 0x011, "s", (30,)),
        ("monnum", 0x02F, "i", ()),
    ]),
    "WCCUSERS": ("player", 1998, [
        ("userid", 0x000, "s", (30,)),
        ("firstname", 0x01E, "s", (11,)),
        ("lastname", 0x029, "s", (19,)),
        ("exp", 0x03C, "I", ()),
        ("spells", 0x040, "H", (10,)),
        ("spellvals", 0x054, "h", (10,)),
        ("spellrnds", 0x068, "h", (10,)),
        ("title", 0x07C, "s", (20,)),
        ("race", 0x090, "H", ()),
        ("class", 0x092, "H", ()),
        ("level", 0x094, "H", ()),
        ("basint", 0x096, "H", ()),
        ("baswil", 0x098, "H", ()),
        ("basstr", 0x09A, "H", ()),
        ("bashea", 0x09C, "H", ()),
        ("basagi", 0x09E, "H", ()),
        ("bascha", 0x0A0, "H", ()),
        ("intel", 0x0A2, "H", ()),
        ("wil", 0x0A4, "H", ()),
        ("str", 0x0A6, "H", ()),
        ("hea", 0x0A8, "H", ()),
        ("agi", 0x0AA, "H", ()),
        ("cha", 0x0AC, "H", ()),
        ("maxhp", 0x0AE, "h", ()),
        ("hp", 0x0B0, "h", ()),
        ("maxwgt", 0x0B2, "h", ()),
        ("wgt", 0x0B4, "H", ()),
        ("maxenergy", 0x0B8, "H", ()),
        ("energy", 0x0BA, "H", ()),
        ("poison", 0x0BE, "h", ()),
        ("maxmr", 0x0C0, "H", ()),
        ("mr", 0x0C2, "H", ()),
        ("map", 0x0C4, "i", ()),
        ("room", 0x0C8, "i", ()),
        ("light", 0x0CC, "h", ()),
        ("hunger", 0x0CE, "h", ()),
        ("thirst", 0x0D0, "h", ()),
        ("ninvslots", 0x0D5, "B", ()),
        ("items", 0x0D6, "i", (100,)),
        ("itemuses", 0x266, "h", (100,)),
        ("nkeyslots", 0x32E, "B", ()),
        ("keys", 0x32F, "i", (50,)),
        ("keyuses", 0x3F7, "h", (50,)),
        ("qflags2", 0x45B, "I", ()),
        ("hpbonus", 0x463, "i", ()),
        ("manabonus", 0x467, "i", ()),
        ("expbil", 0x46B, "I", ()),
        ("exprem", 0x46F, "I", ()),
        ("nspellslots", 0x473, "B", ()),
        ("spellbook", 0x474, "h", (100,)),
        ("evil", 0x53C, "h", ()),
        ("lawful", 0x53E, "b", ()),
        ("trailmaps", 0x547, "i", (20,)),
        ("trailrooms", 0x597, "i", (20,)),
        ("verbose", 0x5E7, "B", ()),
        ("bchannel", 0x5E8, "h", ()),
        ("fasttalk", 0x5EA, "B", ()),
        ("statline", 0x5EB, "B", ()),
        ("hidden", 0x5EC, "B", ()),
        ("perception", 0x5ED, "h", ()),
        ("stealth", 0x5EF, "h", ()),
        ("martial", 0x5F1, "h", ()),
        ("thievery", 0x5F3, "h", ()),
        ("maxmana", 0x5F5, "H", ()),
        ("mana", 0x5F7, "h", ()),
        ("spellcast", 0x5F9, "h", ()),
        ("traps", 0x5FB, "h", ()),
        ("disarm", 0x5FD, "h", ()),
        ("picklocks", 0x5FF, "h", ()),
        ("tracking", 0x601, "h", ()),
        ("runic", 0x603, "I", ()),
        ("platinum", 0x607, "I", ()),
        ("gold", 0x60B, "I", ()),
        ("silver", 0x60F, "I", ()),
        ("copper", 0x613, "I", ()),
        ("weapon", 0x617, "i", ()),
        ("worn", 0x61F, "i", (20,)),
        ("wornuses", 0x66F, "h", (20,)),
        ("autoengage", 0x697, "h", ()),
        ("lives", 0x699, "h", ()),
        ("aided", 0x69B, "B", ()),
        ("leader", 0x69C, "h", ()),
        ("rank", 0x69E, "B", ()),
        ("group", 0x69F, "h", (5,)),
        ("lighttime", 0x6A9, "h", ()),
        ("readied", 0x6AB, "b", ()),
        ("delay", 0x6AC, "B", ()),
        ("roomlight", 0x6AD, "h", ()),
        ("gang", 0x6B9, "B", (20,)),
        ("hairlen", 0x6CD, "B", ()),
        ("haircol", 0x6CE, "B", ()),
        ("eyecol", 0x6D0, "B", ()),
        ("paging", 0x6D2, "B", ()),
        ("cp", 0x6D3, "h", ()),
        ("suicpw", 0x6D5, "B", (9,)),
        ("attackers", 0x6DE, "i", ()),
        ("status", 0x6E2, "H", ()),
        ("movedir", 0x6E4, "h", ()),
        ("dragging", 0x6E6, "h", ()),
        ("totalcp", 0x6E8, "H", ()),
        ("immortal", 0x6EA, "B", ()),
        ("pagecnt", 0x6EB, "B", ()),
        ("pagemax", 0x6EC, "B", ()),
        ("settings", 0x6ED, "H", ()),
        ("gossmax", 0x6EF, "B", ()),
        ("auctmax", 0x6F0, "B", ()),
        ("gosscnt", 0x6F1, "B", ()),
        ("auctcnt", 0x6F2, "B", ()),
        ("sesstime", 0x6F3, "h", ()),
        ("encumpct", 0x6F5, "h", ()),
        ("avbonus", 0x6F7, "h", ()),
        ("acbonus", 0x6F9, "h", ()),
        ("dmgbonus", 0x6FB, "h", ()),
        ("mabase", 0x6FD, "B", ()),
        ("arenahp", 0x6FE, "h", ()),
        ("qflags", 0x705, "I", ()),
        ("version", 0x709, "h", ()),
        ("msglevel", 0x70B, "B", ()),
        ("pvpcnt", 0x70C, "B", ()),
        ("randomhp", 0x70D, "B", ()),
        ("abils", 0x722, "h", (30,)),
        ("abilvals", 0x75E, "h", (30,)),
        ("resttick", 0x79A, "B", ()),
        ("critbonus", 0x79B, "h", ()),
        ("drbonus", 0x79D, "h", ()),
        ("drpct", 0x79F, "h", ()),
        ("energyuse", 0x7A1, "h", ()),
        ("shield", 0x7A3, "h", ()),
        ("defmod", 0x7A5, "h", ()),
        ("lifemins", 0x7A7, "I", ()),
        ("moves", 0x7AB, "B", ()),
        ("lastdate", 0x7AC, "H", ()),
        ("status2", 0x7AE, "H", ()),
        ("scanpos", 0x7B0, "i", ()),
        ("palette", 0x7B4, "B", ()),
        ("deathdate", 0x7B5, "H", ()),
        ("flags", 0x7B7, "H", ()),
        ("sex", 0x7B9, "B", ()),
        ("healrate", 0x7BA, "h", ()),
        ("manarate", 0x7BC, "h", ()),
        ("racechg", 0x7BE, "I", ()),
        ("lastmon", 0x7C6, "i", ()),
        ("lastmoncnt", 0x7CA, "H", ()),
        ("pvptimer", 0x7CC, "B", ()),
        ("medtick", 0x7CD, "B", ()),
    ]),
}


def _from_record(cls, rec, attrs):
    """An instance of cls (Room, Monster, Shop, ...) filled from an exported record instead of
    raw bytes.  attrs maps attribute -> record field (or a callable on the record)."""
    o = cls.__new__(cls)
    for attr, f in attrs.items():
        v = f(rec) if callable(f) else rec[f]
        setattr(o, attr, tuple(v) if isinstance(v, list) and attr in _TUPLE_ATTRS else v)
    return o


_TUPLE_ATTRS = {"exits", "exittypes", "exitpar1", "exitpar2", "cash", "atkmins", "atkmaxs",
                "atkenergies"}
_ROOM_FIELDS = {"map": "mapnum", "num": "roomnum", "name": "name", "exits": "exits",
                "exittypes": "exittypes", "exitpar1": "exitpar1s", "exitpar2": "exitpar2s",
                "type": "type", "shop": "shopnum", "minlevel": "minlevel",
                "maxlevel": "maxlevel", "genmonster": "genmonster", "light": "light",
                "regendelay": "regendelay", "maxmonsters": "maxmonsters",
                "mongroup": "mongroup"}
_MONSTER_FIELDS = {"num": "number", "name": "name", "montype": "montype", "level": "level",
                   "genindex": "genindex", "dr": "dr", "ac": "ac", "magicres": "magicres",
                   "exp": "exp", "maxhp": "maxhp", "energy": "energy", "grouprole": "grouprole",
                   "alignment": "alignment", "cash": "cash", "atktypes": "atktypes",
                   "atkpcts": "atkpcts", "atkmins": "atkmins", "atkmaxs": "atkmaxs",
                   "atkenergies": "atkenergies"}


class World:
    def __init__(self, world_json=None):
        if world_json:
            # rooms, monsters, shops and messages from an export (enough for routing, spawn
            # pools and routecheck); items, spells, classes and races stay empty
            self.rooms = {(r.map, r.num): r for r in
                          (_from_record(Room, x, _ROOM_FIELDS)
                           for x in load_export(world_json, "WCCMP001"))}
            self.monsters = {m.num: m for m in
                             (_from_record(Monster, x, _MONSTER_FIELDS)
                              for x in load_export(world_json, "WCCKNMSR"))}
            self.shops = {}
            self.items = {}
            self.classes = self.races = self.spells = {}
            self.msgs = {x["number"]: x["msgs"] for x in load_export(world_json, "WCCMSG")}
            return
        self.rooms = {r.key: r for r in map(Room, _rows("WCCMP001"))}
        self.monsters = {m.num: m for m in map(Monster, _rows("WCCKNMSR"))}
        self.shops = {s.num: s for s in map(Shop, _rows("WCCSHOPS"))}
        self.items = {i.num: i for i in map(Item, _rows("WCCITEMS"))}
        self.classes = {_s(d[2:32]): struct.unpack_from("<H", d, 0)[0] for d in _rows("WCCCLASS")}
        # class number -> {ability: value} (e.g. 31 Bash, 51 Anti-Magic)
        self.class_abils = {struct.unpack_from("<H", d, 0)[0]:
                            {a: v for a, v in zip(struct.unpack_from("<10h", d, 0x2C),
                                                  struct.unpack_from("<10h", d, 0x4A)) if a}
                            for d in _rows("WCCCLASS")}
        self.races = {_s(d[2:32]): struct.unpack_from("<H", d, 0)[0] for d in _rows("WCCRACE")}
        # race name -> (mins, maxs) keyed str/int/wil/agi/hea/cha
        order = ("int", "wil", "str", "hea", "agi", "cha")  # record order of min/max fields
        self.race_ranges = {_s(d[2:32]): (dict(zip(order, struct.unpack_from("<6h", d, 0x20))),
                                          dict(zip(order, struct.unpack_from("<6h", d, 0x66))))
                            for d in _rows("WCCRACE")}
        self.spells = {sp.num: sp for sp in map(Spell, _rows("WCCSPELS"))}
        # class number -> (magery type, magery level)
        self.class_magery = {struct.unpack_from("<H", d, 0)[0]: struct.unpack_from("<hh", d, 0x40)
                             for d in _rows("WCCCLASS")}
        self.msgs = {}
        for d in _rows("WCCMSG"):
            n = struct.unpack_from("<i", d, 0)[0]
            self.msgs[n] = [_s(d[4 + 82 * i:4 + 82 * (i + 1)]) for i in range(3)]

    @staticmethod
    def live_shop(num):
        """{item: quantity in stock} for a shop, read fresh from the database."""
        c = sqlite3.connect(os.path.join(DATA, "WCCSHOPS.db"))
        for (d,) in c.execute("select data from data_t"):
            if struct.unpack_from("<i", d, 0)[0] == num:
                return Shop(d).qty
        return {}

    char_level = None   # set by the bot: Level exits are checked against it
    blocked = set()     # (room key, dir) the game refused us (learned at run time)

    def exit_ok(self, room, d):
        """Can the current character take exit d of room?  Level exits carry a level range
        (exitpar1 = minimum, exitpar2 = maximum, 0 = no limit): the Silvermere Docks -> Newhaven
        crossing is 0-3.  Exits the game has refused us are skipped too."""
        if (room.key, d) in self.blocked:
            return False
        if EXITTYPES[room.exittypes[d]] == "Level" and self.char_level is not None:
            lo, hi = room.exitpar1[d], room.exitpar2[d]
            if (lo and self.char_level < lo) or (hi and self.char_level > hi):
                return False
        return True

    def exit_command(self, room, d):
        """What to type to take exit d of room: the direction, or a Text exit's phrase."""
        if EXITTYPES[room.exittypes[d]] == "Text":
            m = self.msgs.get(room.exitpar1[d])
            if m and m[0]:
                return m[0]
        return DIRCMD[d]

    def find(self, spec):
        if ":" in spec and spec.replace(":", "").isdigit():
            m, r = map(int, spec.split(":"))
            return [self.rooms[(m, r)]] if (m, r) in self.rooms else []
        exact = [r for r in self.rooms.values() if r.name.lower() == spec.lower()]
        return exact or [r for r in self.rooms.values() if spec.lower() in r.name.lower()]

    danger = set()       # room keys to avoid walking through (set by the bot per character)
    DANGER_COST = 40     # one dangerous room "costs" as much as this many safe rooms

    def path(self, src, dst, walkable=WALKABLE, within=None):
        """Cheapest list of exit commands from room key src to dst, optionally only through the
        rooms in `within`.  Each step costs 1, plus DANGER_COST to enter a room in self.danger,
        so routes go around dangerous areas when there's any reasonable way round (Dijkstra)."""
        import heapq
        best = {src: 0}
        prev = {src: None}
        heap = [(0, src)]
        while heap:
            c, cur = heapq.heappop(heap)
            if cur == dst:
                break
            if c > best.get(cur, 1 << 30):
                continue
            room = self.rooms.get(cur)
            if not room:
                continue
            for d, nxt, t in room.links():
                if t not in walkable or nxt not in self.rooms or not self.exit_ok(room, d) \
                   or (within is not None and nxt not in within):
                    continue
                nc = c + 1 + (self.DANGER_COST if nxt in self.danger and nxt != dst else 0)
                if nc < best.get(nxt, 1 << 30):
                    best[nxt] = nc
                    prev[nxt] = (cur, d)
                    heapq.heappush(heap, (nc, nxt))
        if dst not in prev:
            return None
        steps = []
        k = dst
        while prev[k]:
            k, d = prev[k]
            steps.append((self.exit_command(self.rooms[k], d), self.rooms[k].exits[d]))
        return [c for c, _ in steps[::-1]]

    def danger_on_path(self, src, dst):
        """How many dangerous rooms the cheapest route from src to dst still crosses."""
        cur, n = src, 0
        for cmd, nxt in (self.route(src, dst) or []):
            if nxt in self.danger and nxt != dst:
                n += 1
        return n

    def route(self, src, dst, walkable=None, within=None):
        """Like path(), but as [(command, room key reached)] for following and checking."""
        walkable = walkable or WALKABLE
        cmds = self.path(src, dst, walkable, within)
        if cmds is None:
            return None
        out, cur = [], src
        for c in cmds:
            r = self.rooms[cur]
            nxt = next(k for d, k, t in r.links()
                       if t in walkable and self.exit_command(r, d) == c
                       and (within is None or k in within) and self.exit_ok(r, d))
            out.append((c, nxt))
            cur = nxt
        return out

    def distances(self, src, maxsteps, walkable=None):
        """{room key: steps} for every room reachable from src within maxsteps."""
        walkable = walkable or WALKABLE
        dist = {src: 0}
        q = collections.deque([src])
        while q:
            cur = q.popleft()
            if dist[cur] >= maxsteps or cur not in self.rooms:
                continue
            for d, nxt, t in self.rooms[cur].links():
                if t in walkable and nxt not in dist and nxt in self.rooms \
                   and self.exit_ok(self.rooms[cur], d):
                    dist[nxt] = dist[cur] + 1
                    q.append(nxt)
        return dist

    def patrol(self, start, stops, within):
        """A loop through the spawn rooms `stops` (nearest-neighbour order, then 2-opt), as
        [(command, room reached)] starting and ending at the first stop."""
        stops = [s for s in stops if s in within]
        if not stops:
            return []
        memo = {}

        def dist(a, b):
            if (a, b) not in memo:
                p = self.path(a, b, within=within)
                memo[(a, b)] = len(p) if p is not None else 10**6
            return memo[(a, b)]
        order = [min(stops, key=lambda s: dist(start, s))]
        left = set(stops) - {order[0]}
        while left:
            nxt = min(left, key=lambda s: dist(order[-1], s))
            order.append(nxt)
            left.remove(nxt)
        # 2-opt on the closed loop, a few passes
        n = len(order)
        for _ in range(3):
            improved = False
            for i in range(1, n - 1):
                for j in range(i + 1, n):
                    a, b = order[i - 1], order[i]
                    c, d = order[j], order[(j + 1) % n]
                    if dist(a, c) + dist(b, d) < dist(a, b) + dist(c, d):
                        order[i:j + 1] = reversed(order[i:j + 1])
                        improved = True
            if not improved:
                break
        steps = []
        for a, b in zip(order, order[1:] + order[:1]):
            leg = self.route(a, b, within=within) or []
            steps += leg
        return order, steps

    def spawn_pool(self, room):
        """Known monsters this room can generate (genmonster plus the group/genindex rule)."""
        key = (room.map, room.num)
        if not hasattr(self, "_pools"):
            self._pools = {}
        if key not in self._pools:
            self._pools[key] = self._spawn_pool(room)
        return self._pools[key]

    def _spawn_pool(self, room):
        pool = []
        if room.genmonster and room.genmonster in self.monsters:
            pool.append(self.monsters[room.genmonster])
        if room.maxmonsters and room.maxlevel:
            pool += [m for m in self.monsters.values()
                     if m.montype == room.mongroup
                     and (room.minlevel == 0 or m.genindex >= room.minlevel)
                     and m.genindex <= room.maxlevel]
        return pool

    def areas(self, mapnum=None):
        """Group spawning rooms into areas by room name: {name: [rooms]}."""
        out = collections.defaultdict(list)
        for r in self.rooms.values():
            if (mapnum is None or r.map == mapnum) and r.maxmonsters and (r.maxlevel or r.genmonster):
                out[r.name].append(r)
        return out

# ---------------------------------------------------------------------------------------------
# Full-record export (T-006): every field of every stock table as line-delimited JSON, decoded
# from the RECORDS tables above.  The Room/Monster/... classes stay as they are (the bot uses
# them); tests/test_world_export.py checks that they agree with the export.
# ---------------------------------------------------------------------------------------------

EXPORT_TABLES = ["WCCMP001", "NEWMP001", "WCCITEMS", "WCCKNMSR", "WCCACMSR", "WCCSPELS",
                 "WCCCLASS", "WCCRACE", "WCCSHOPS", "WCCMSG", "WCCTEXT", "WCCACTS", "WCCGANGS",
                 "WCCBANKS", "WCCITOWN", "WCCUSERS"]
RECORDS["NEWMP001"] = RECORDS["WCCMP001"]
TABLE_NOTES = {
    "WCCACMSR": "run-time active-monster state at the moment of export (not stock content)",
    "WCCUSERS": "rows are test characters, if any; never stock content (no real passwords are "
                "stored in the record; suicpw is game data)",
}
# the stock zip in the cache tools/differential/fetch-stock.sh fills (T-120)
DIST_ZIP = os.path.join(
    os.environ.get("SILVERMERE_STOCK_CACHE")
    or os.path.join(os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache"),
                    "silvermere", "stock"),
    "WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip")


def _file_ci(directory, name, exts):
    """Path of NAME with any of the extensions, upper or lower case (MBBSEmu varies), or None."""
    for e in exts:
        for n in (name + e, name + e.upper(), name + e.lower()):
            p = os.path.join(directory, n)
            if os.path.exists(p):
                return p
    return None


def decode_record(spec, d):
    """spec is RECORDS[table]; returns {"_raw_len": n, field: value, ...} in field order."""
    out = {"_raw_len": len(d)}
    for name, off, kind, dims in spec[2]:
        if kind == "x":
            out[name] = base64.b64encode(d[off:off + dims[0]]).decode("ascii")
        elif kind == "s":
            if len(dims) == 1:
                out[name] = _s(d[off:off + dims[0]])
            else:
                rows, width = dims
                out[name] = [_s(d[off + i * width:off + (i + 1) * width]) for i in range(rows)]
        elif not dims:
            out[name] = struct.unpack_from("<" + kind, d, off)[0]
        else:
            out[name] = list(struct.unpack_from("<%d%s" % (dims[0], kind), d, off))
    if spec[0] == "monster":
        # a monster record stores cash runic-first; CURRENCY codes are copper-first
        out["cash_copper_first"] = out["cash"][::-1]
    return out


def read_rows(directory, table):
    """[(data_t.id, data)] in id order, or None if the table's .db is not there."""
    p = _file_ci(directory, table, [".db"])
    if not p:
        return None
    c = sqlite3.connect(p)
    try:
        return list(c.execute("select id, data from data_t order by id"))
    finally:
        c.close()


def vir_header(path):
    """(page length, record count) from the Btrieve FCR of a .VIR file (wbtrv32 survey 2)."""
    with open(path, "rb") as f:
        h = f.read(0x40)
    page = struct.unpack_from("<H", h, 0x08)[0]
    hi, lo = struct.unpack_from("<HH", h, 0x1A)
    return page, (hi << 16) | lo


def _sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def export_world(data, out, tables=None, dist=DIST_ZIP, instance=None):
    os.makedirs(out, exist_ok=True)
    if instance is None:  # .../test/run/<instance>/WCCMMUD
        instance = os.path.basename(os.path.dirname(os.path.abspath(data)))
    dll = _file_ci(data, "WCCMMUD", [".dll"]) or _file_ci(os.path.dirname(os.path.abspath(data)),
                                                          "WCCMMUD", [".dll"])
    manifest = {
        "format": 1,
        "date": datetime.date.today().isoformat(),
        "post_update": True,  # WCCUPDAT.DAT was applied by the DLL's paced update at first start, not by wccmmutl
        "source": "MBBSEmu WCC*.db decoded by mmud_data.py export",
        "instance": instance,  # the staged MBBSEmu instance whose .db files were read
        "module_dll": os.path.basename(dll) if dll else None,
        "module_dll_sha256": _sha256(dll) if dll else None,  # the DLL that ran (WCCMMUD.DLL)
        "dist_zip": os.path.basename(dist),
        "dist_zip_sha256": _sha256(dist) if os.path.exists(dist) else None,
        "tables": {},
    }
    for table in tables or EXPORT_TABLES:
        rows = read_rows(data, table)
        if rows is None:
            if tables:
                sys.exit(f"{table}: no .db in {data}")
            continue
        spec = RECORDS[table]
        path = os.path.join(out, table + ".jsonl.gz")
        with open(path, "wb") as raw, gzip.GzipFile("", "wb", 9, raw, mtime=0) as gz:
            for rid, d in rows:
                if len(d) != spec[1]:
                    sys.exit(f"{table} id {rid}: {len(d)} bytes, expected {spec[1]}")
                rec = {"_id": rid}
                rec.update(decode_record(spec, d))
                gz.write((json.dumps(rec, separators=(",", ":")) + "\n").encode("ascii"))
        ent = {"db_sha256": _sha256(_file_ci(data, table, [".db"])), "struct": spec[0], "rows": len(rows), "record_length": spec[1],
               "file": table + ".jsonl.gz", "file_bytes": os.path.getsize(path),
               "file_sha256": _sha256(path)}
        vir = _file_ci(data, table, [".vir"])
        if vir:
            page, count = vir_header(vir)
            ent.update(vir_sha256=_sha256(vir), fcr_page_length=page, fcr_record_count=count)
        if table in TABLE_NOTES:
            ent["note"] = TABLE_NOTES[table]
        manifest["tables"][table] = ent
    with open(os.path.join(out, "MANIFEST.json"), "w") as f:
        json.dump(manifest, f, indent=1)
        f.write("\n")
    return manifest


def load_export(directory, table):
    """Rows of one exported table as dicts, in file (data_t.id) order."""
    with gzip.open(os.path.join(directory, table + ".jsonl.gz"), "rt", encoding="ascii") as f:
        return [json.loads(line) for line in f]


def fmt_room(w, r):
    ex = ", ".join(f"{DIRS[d]}->{k[0]}:{k[1]}" + ("" if t == "Normal" else f"[{t}]")
                   for d, k, t in r.links())
    s = f"{r.map}:{r.num} {r.name} ({ROOMTYPES[r.type] if 0 <= r.type < 8 else r.type})"
    if r.shop:
        sh = w.shops.get(r.shop)
        s += f" shop {r.shop}" + (f" '{sh.name}' {SHOPTYPES[sh.type]}" if sh else "")
    s += f"\n    exits: {ex}"
    pool = w.spawn_pool(r)
    if pool:
        s += f"\n    spawns (max {r.maxmonsters}): " + ", ".join(
            f"{m.name}(hp {m.maxhp}, exp {m.exp}, hit<={m.maxhit})" for m in pool[:12])
    return s


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("room").add_argument("spec")
    p = sub.add_parser("path"); p.add_argument("src"); p.add_argument("dst")
    sub.add_parser("monster").add_argument("name")
    sub.add_parser("shops").add_argument("type", nargs="?")
    p = sub.add_parser("hunt"); p.add_argument("--hp", type=int, required=True)
    p.add_argument("--map", type=int, default=1); p.add_argument("--from", dest="src")
    p.add_argument("--top", type=int, default=20)
    p.add_argument("--hpratio", type=float, default=1.0)
    p.add_argument("--maxsteps", type=int, default=120)
    p = sub.add_parser("export")
    p.add_argument("--data", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--tables", nargs="*")
    p.add_argument("--instance", help="instance name for the manifest (default: from --data)")
    a = ap.parse_args()
    if a.cmd == "export":
        m = export_world(a.data, a.out, a.tables, instance=a.instance)
        for t, e in m["tables"].items():
            print(f"{t}: {e['rows']} rows (FCR {e.get('fcr_record_count')}), {e['file_bytes']} bytes")
        return
    w = World()

    if a.cmd == "room":
        for r in w.find(a.spec)[:30]:
            print(fmt_room(w, r))
    elif a.cmd == "path":
        s, d = w.find(a.src), w.find(a.dst)
        if not s or not d:
            sys.exit("room not found")
        print(" ".join(w.path(s[0].key, d[0].key) or ["(no walkable path)"]))
    elif a.cmd == "monster":
        for m in w.monsters.values():
            if a.name.lower() in m.name.lower():
                print(f"{m.num} {m.name}: lvl {m.level} genidx {m.genindex} type {m.montype} "
                      f"hp {m.maxhp} exp {m.exp} ac {m.ac} dr {m.dr} hit<={m.maxhit} "
                      f"coins~{m.coin_value:.0f}c")
    elif a.cmd == "shops":
        where = collections.defaultdict(list)
        for r in w.rooms.values():
            if r.shop:
                where[r.shop].append(r)
        for s in w.shops.values():
            if a.type and SHOPTYPES[s.type].lower() != a.type.lower():
                continue
            rooms = ", ".join(f"{r.map}:{r.num} {r.name}" for r in where.get(s.num, [])[:3])
            extra = f" levels {s.minlevel}-{s.maxlevel} class {s.trainclass}" if s.type == 8 else ""
            print(f"{s.num} {s.name} [{SHOPTYPES[s.type]}]{extra} @ {rooms}")
    elif a.cmd == "hunt":
        start = w.find(a.src)[0].key if a.src else None
        rows = []
        for name, rooms in w.areas(a.map).items():
            pool = {m.num: m for r in rooms for m in w.spawn_pool(r)}.values()
            pool = [m for m in pool if m.exp > 0 and m.maxhp > 0]
            if not pool:
                continue
            worst_hit = max(m.maxhit for m in pool)
            hp = sum(m.maxhp for m in pool) / len(pool)
            exp = sum(m.exp for m in pool) / len(pool)
            # safe enough: no monster hits for more than a quarter of our HP, and none has more
            # HP than we do (fights stay short)
            if worst_hit > a.hp / 4 or max(m.maxhp for m in pool) > a.hp * a.hpratio:
                continue
            dist = 0
            if start:
                route = w.path(start, rooms[0].key)
                if route is None or len(route) > a.maxsteps:
                    continue  # unreachable on foot (keys, items, spells) or too far
                dist = len(route)
            rows.append((exp / hp, exp, hp, worst_hit, len(rooms), dist, name, rooms[0]))
        rows.sort(reverse=True)
        print(f"{'exp/hp':>6} {'exp':>6} {'hp':>5} {'hit':>4} {'rooms':>5} {'steps':>5}  area")
        for eh, exp, hp, hit, n, dist, name, r in rows[:a.top]:
            print(f"{eh:6.2f} {exp:6.0f} {hp:5.0f} {hit:4d} {n:5d} {dist:5d}  {name} ({r.map}:{r.num})")


if __name__ == "__main__":
    main()
