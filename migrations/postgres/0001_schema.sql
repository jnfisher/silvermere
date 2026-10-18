-- silvermere schema v1, PostgreSQL dialect.
--
-- Normalised from the game's data-file records.
-- Content tables hold the world as shipped; state tables hold what play changes. The
-- two are one-to-one where the original kept both in one record (room_state,
-- shop_stock, monster_state). Column names keep the record's member names; a comment
-- names the member where the column had to be renamed.
--
-- Conventions: every table has an integer primary key; fixed-width DOS strings are
-- TEXT trimmed at the first NUL; flag words and enum codes are stored as the integer
-- the record holds; coins are copper first (the monster and item cash[5] arrays are
-- runic first in the record and reversed at import). Case-insensitive keys use a
-- lower() expression index here and COLLATE NOCASE on SQLite. Booleans are BOOLEAN
-- here and INTEGER 0/1 on SQLite; unsigned 32-bit values and 64-bit ids are BIGINT.
-- Array members become child tables with a slot column; empty slots (item 0, spell 0,
-- exit to room 0) are not stored.
--
-- The records' DOS packed dates and times (lastdate, deathdate, killdate/killtime,
-- gangs.created, active_monsters.created) are stored as Unix seconds UTC, 0 meaning never;
-- the importer converts them and monster killdate/killtime become one killed_at
-- (decided 2026-10-14, replacing the T-003 choice to keep the packed integers).
--
-- tools/schema-doc renders docs/schema.md from this file: keep one column per line
-- with its comment after `--`, and the table comment directly above CREATE TABLE.

-- ============================================================================
-- Accounts
-- ============================================================================

-- A login account. Replaces the BBS
-- user record; the character's kept userid column equals the username at import.
CREATE TABLE accounts (
    id            BIGSERIAL PRIMARY KEY,         -- account id
    username      TEXT NOT NULL,-- login name, unique case-insensitive
    password_hash TEXT NOT NULL,               -- argon2id PHC string, '' until set
    email         TEXT,                                      -- optional contact address
    created       TEXT NOT NULL,               -- RFC 3339 UTC
    last_login    TEXT,                                      -- RFC 3339 UTC, NULL before the first login
    flags         BIGINT NOT NULL DEFAULT 0                  -- bit 0 sex (0 M, 1 F); bits 1-3 no_gossip, no_auction, no_loan
);
CREATE UNIQUE INDEX accounts_username ON accounts (lower(username));

-- Roles granted to an account: player, sysop, gameop (the original SYSOPKEY/GAMOPKEY).
CREATE TABLE account_roles (
    account_id BIGINT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE, -- the account
    role       TEXT NOT NULL,                                 -- player, sysop, gameop, or an INI permission name (SYS_GOTO_KEY ...)
    PRIMARY KEY (account_id, role)
);

-- ============================================================================
-- Content: rooms
-- ============================================================================

-- Room record, WCCMP001: the static part. The live fields are in
-- room_state, room_floor_items and room_monsters.
CREATE TABLE rooms (
    id         BIGSERIAL PRIMARY KEY, -- surrogate key; child tables reference it
    map        BIGINT NOT NULL,                  -- mapnum, key 0 part 1
    room       BIGINT NOT NULL,                  -- roomnum, key 0 part 2
    name       TEXT NOT NULL,       -- room name
    desc0      TEXT NOT NULL,       -- desc[0], description line 1
    desc1      TEXT NOT NULL,       -- desc[1]
    desc2      TEXT NOT NULL,       -- desc[2]
    desc3      TEXT NOT NULL,       -- desc[3]
    desc4      TEXT NOT NULL,       -- desc[4]
    desc5      TEXT NOT NULL,       -- desc[5]
    desc6      TEXT NOT NULL,       -- desc[6], description line 7
    mapfile    TEXT NOT NULL,       -- ANSI map file name
    room_type  INTEGER NOT NULL,                  -- type, ROOM_* code
    shopnum    INTEGER NOT NULL,                  -- shop/bank/trainer number, 0 none
    minlevel   INTEGER NOT NULL,                  -- generation: minimum level
    maxlevel   INTEGER NOT NULL,                  -- generation: maximum level (arena: player maximum)
    genmonster INTEGER NOT NULL,                  -- generation monster number
    light      INTEGER NOT NULL,                  -- light level, negative is dark
    ganghouse  INTEGER NOT NULL,                  -- gang house number, GH_* code
    maxmonsters INTEGER NOT NULL,                 -- most monsters generated here
    mongroup   INTEGER NOT NULL,                  -- monster group
    flags      BIGINT NOT NULL,                   -- RF_* bits except RF_SPECALIVE, which is room_state.specalive
    deathroom  BIGINT NOT NULL,                  -- reincarnation room, 0 default
    huproom    BIGINT NOT NULL,                  -- room for a hang-up here
    cmdtext    INTEGER NOT NULL,                  -- WCCTEXT block: room command
    regendelay INTEGER NOT NULL,                  -- minutes between regenerations
    areamax    INTEGER NOT NULL,                  -- area monster maximum
    ctlroom    BIGINT NOT NULL,                  -- controlling room, 0 none
    specmon    INTEGER NOT NULL,                  -- specific monster (known number)
    protmsg    INTEGER NOT NULL,                  -- WCCTEXT block: protected reason
    picktrap   INTEGER NOT NULL,                  -- spell on a failed pick-lock
    roomspell  INTEGER NOT NULL                   -- occupant spell, medium cycle
);
CREATE UNIQUE INDEX rooms_map_room ON rooms (map, room);

-- exits[d], exittypes[d], exitpar1s..4s[d], one row per used slot.
CREATE TABLE room_exits (
    room_id   BIGINT NOT NULL REFERENCES rooms (id) ON DELETE CASCADE, -- the room
    slot      INTEGER NOT NULL,                                         -- 0..9, the DIR_* index
    direction INTEGER NOT NULL,                                         -- DIR_* code (equals slot)
    target    BIGINT NOT NULL,                                         -- exits[d]: room on the same map, 0 none
    exit_type INTEGER NOT NULL,                                         -- exittypes[d], EXIT_* code
    par1      BIGINT NOT NULL,                                         -- exitpar1s[d]
    par2      INTEGER NOT NULL,                                         -- exitpar2s[d]
    par3      INTEGER NOT NULL,                                         -- exitpar3s[d]
    par4      INTEGER NOT NULL,                                         -- exitpar4s[d]
    PRIMARY KEY (room_id, slot)
);

-- placed[NPLACED]: items placed in the room by the sysop.
CREATE TABLE room_placed_items (
    room_id BIGINT NOT NULL REFERENCES rooms (id) ON DELETE CASCADE, -- the room
    slot    INTEGER NOT NULL,                                         -- 0..9
    item    INTEGER NOT NULL,                                         -- item number
    PRIMARY KEY (room_id, slot)
);

-- ============================================================================
-- Content: items
-- ============================================================================

-- Item record, WCCITEMS. active is the one live field (copies of a
-- limited item in play) and is updated through items.set_active.
CREATE TABLE items (
    number       INTEGER PRIMARY KEY, -- item number, key 0
    level        INTEGER NOT NULL,    -- detect power to show hiddesc
    limit_copies INTEGER NOT NULL,    -- limit: copies allowed, 0 unlimited
    name         TEXT NOT NULL,       -- item name
    desc0        TEXT NOT NULL,       -- desc[0]
    desc1        TEXT NOT NULL,       -- desc[1]
    desc2        TEXT NOT NULL,       -- desc[2]
    desc3        TEXT NOT NULL,       -- desc[3]
    desc4        TEXT NOT NULL,       -- desc[4]
    desc5        TEXT NOT NULL,       -- desc[5]
    hiddesc0     TEXT NOT NULL,       -- hiddesc[0]
    hiddesc1     TEXT NOT NULL,       -- hiddesc[1]
    hiddesc2     TEXT NOT NULL,       -- hiddesc[2]
    active       INTEGER NOT NULL,    -- copies in play (limited item, live)
    weight       INTEGER NOT NULL,    -- encumbrance
    item_type    INTEGER NOT NULL,    -- type, ITEM_* code
    charges      INTEGER NOT NULL,    -- uses of a new copy, -1 none
    energy       INTEGER NOT NULL,    -- energy to use
    cost         INTEGER NOT NULL,    -- price in currency units
    mindmg       INTEGER NOT NULL,    -- weapon damage, minimum
    maxdmg       INTEGER NOT NULL,    -- weapon damage, maximum
    ac           INTEGER NOT NULL,    -- armour class
    wpntype      INTEGER NOT NULL,    -- WPN_* code
    armtype      INTEGER NOT NULL,    -- ARM_* code
    worn         INTEGER NOT NULL,    -- WORN_* code
    accuracy     INTEGER NOT NULL,    -- accuracy bonus
    dr           INTEGER NOT NULL,    -- damage resistance
    gettable     INTEGER NOT NULL,    -- may be picked up
    strreq       INTEGER NOT NULL,    -- strength required
    cash_copper   BIGINT NOT NULL,   -- cash[4]: containers, copper found on OPEN
    cash_silver   BIGINT NOT NULL,   -- cash[3]
    cash_gold     BIGINT NOT NULL,   -- cash[2]
    cash_platinum BIGINT NOT NULL,   -- cash[1]
    cash_runic    BIGINT NOT NULL,   -- cash[0]
    speed        INTEGER NOT NULL,    -- weapon speed
    hitmsg       INTEGER NOT NULL,    -- WCCMSG: hit verbs
    missmsg      INTEGER NOT NULL,    -- WCCMSG: miss verbs
    textnum      INTEGER NOT NULL,    -- WCCTEXT after the description
    destroymsg   INTEGER NOT NULL,    -- WCCMSG when charges run out
    nodrop       INTEGER NOT NULL,    -- may not be dropped or given
    currency     INTEGER NOT NULL,    -- CUR_* code of cost
    retain       INTEGER NOT NULL,    -- kept when charges run out
    robable      INTEGER NOT NULL,    -- may be stolen
    destroy      INTEGER NOT NULL     -- destroyed on death
);

-- abilities[NITEMABILS] with abilvals[].
CREATE TABLE item_abilities (
    item    INTEGER NOT NULL REFERENCES items (number) ON DELETE CASCADE, -- the item
    slot    INTEGER NOT NULL,                                             -- 0..19
    ability INTEGER NOT NULL,                                             -- ABL_* code
    value   INTEGER NOT NULL,                                             -- abilvals[slot]
    PRIMARY KEY (item, slot)
);

-- classes[10]: classes allowed, none stored = any.
CREATE TABLE item_classes (
    item  INTEGER NOT NULL REFERENCES items (number) ON DELETE CASCADE, -- the item
    slot  INTEGER NOT NULL,                                             -- 0..9
    class INTEGER NOT NULL,                                             -- class number
    PRIMARY KEY (item, slot)
);

-- races[10]: races allowed, none stored = any.
CREATE TABLE item_races (
    item INTEGER NOT NULL REFERENCES items (number) ON DELETE CASCADE, -- the item
    slot INTEGER NOT NULL,                                             -- 0..9
    race INTEGER NOT NULL,                                             -- race number
    PRIMARY KEY (item, slot)
);

-- negates[10]: spells negated while worn.
CREATE TABLE item_negates (
    item  INTEGER NOT NULL REFERENCES items (number) ON DELETE CASCADE, -- the item
    slot  INTEGER NOT NULL,                                             -- 0..9
    spell INTEGER NOT NULL,                                             -- spell number
    PRIMARY KEY (item, slot)
);

-- ============================================================================
-- Content: known monsters
-- ============================================================================

-- Known-monster record, WCCKNMSR. curalive and killed_at (killdate,
-- killtime) are in monster_state. cash[5] is runic first in the record.
CREATE TABLE monsters (
    number     INTEGER PRIMARY KEY, -- known-monster number, key 0
    name       TEXT NOT NULL,       -- name
    montype    INTEGER NOT NULL,    -- MTYPE_* code
    level      BIGINT NOT NULL,    -- level; experience awarded is exp * level
    genindex   INTEGER NOT NULL,    -- generation index
    wornitem   INTEGER NOT NULL,    -- item worn (abilities only)
    weapon     INTEGER NOT NULL,    -- weapon item
    dr         INTEGER NOT NULL,    -- damage resistance
    ac         INTEGER NOT NULL,    -- armour class
    group_num  INTEGER NOT NULL,    -- group: group number
    followpct  INTEGER NOT NULL,    -- percent chance to follow a user
    magicres   INTEGER NOT NULL,    -- magic resistance
    bsdefense  INTEGER NOT NULL,    -- defence against backstab
    exp        INTEGER NOT NULL,    -- experience
    maxhp      INTEGER NOT NULL,    -- hit points
    energy     INTEGER NOT NULL,    -- energy per round
    hpregen    INTEGER NOT NULL,    -- hit points per slow tick
    maxalive   INTEGER NOT NULL,    -- most alive at once, 0 no limit
    grouprole  INTEGER NOT NULL,    -- FOLLOW_* code
    maxfollow  INTEGER NOT NULL,    -- members a leader takes along
    undead     INTEGER NOT NULL,    -- affected by Kill Dead
    alignment  INTEGER NOT NULL,    -- ALIGN_* code
    regenhrs   INTEGER NOT NULL,    -- regeneration time, hours
    arrivemsg  INTEGER NOT NULL,    -- WCCMSG: arrival
    deathmsg   INTEGER NOT NULL,    -- WCCMSG: death
    cash_copper   BIGINT NOT NULL, -- cash[4]: most copper carried
    cash_silver   BIGINT NOT NULL, -- cash[3]
    cash_gold     BIGINT NOT NULL, -- cash[2]
    cash_platinum BIGINT NOT NULL, -- cash[1]
    cash_runic    BIGINT NOT NULL, -- cash[0]
    talktext   INTEGER NOT NULL,    -- WCCTEXT block for ASK
    charmlvl   INTEGER NOT NULL,    -- user level needed to charm
    namelist   INTEGER NOT NULL,    -- random-name list
    randtext   INTEGER NOT NULL,    -- WCCTEXT block: random event
    charmres   INTEGER NOT NULL,    -- resistance to enslaving
    deathspell INTEGER NOT NULL,    -- spell cast when killed
    createspell INTEGER NOT NULL,   -- spell cast when generated
    desc0      TEXT NOT NULL,       -- desc[0]
    desc1      TEXT NOT NULL,       -- desc[1]
    desc2      TEXT NOT NULL,       -- desc[2]
    desc3      TEXT NOT NULL,       -- desc[3]
    gender     INTEGER NOT NULL     -- GENDER_* code
);

-- abilities[NRECABILS] with abilvals[].
CREATE TABLE monster_abilities (
    monster INTEGER NOT NULL REFERENCES monsters (number) ON DELETE CASCADE, -- the monster
    slot    INTEGER NOT NULL,                                                -- 0..9
    ability INTEGER NOT NULL,                                                -- ABL_* code
    value   INTEGER NOT NULL,                                                -- abilvals[slot]
    PRIMARY KEY (monster, slot)
);

-- The five attack slots: atktypes, atkaccs, atkpcts, atkmins, atkmaxs, hitmsgs, defmsgs,
-- missmsgs, atkenergy, hitspells at one index.
CREATE TABLE monster_attacks (
    monster  INTEGER NOT NULL REFERENCES monsters (number) ON DELETE CASCADE, -- the monster
    slot     INTEGER NOT NULL,                                                -- 0..4
    kind     INTEGER NOT NULL,                                                -- atktypes[slot], MATK_* code
    accuracy INTEGER NOT NULL,                                                -- accuracy; spell number for a spell attack
    pct      INTEGER NOT NULL,                                                -- cumulative percent chance
    min      INTEGER NOT NULL,                                                -- minimum damage; cast percent for a spell attack
    max      INTEGER NOT NULL,                                                -- maximum damage; cast level for a spell attack
    hitmsg   INTEGER NOT NULL,                                                -- WCCMSG: hit
    defmsg   INTEGER NOT NULL,                                                -- WCCMSG: deflect or dodge
    missmsg  INTEGER NOT NULL,                                                -- WCCMSG: miss
    energy   INTEGER NOT NULL,                                                -- energy of the attack
    hitspell INTEGER NOT NULL,                                                -- spell cast on a hit
    PRIMARY KEY (monster, slot)
);

-- items[NMONITEMS] with itemuses[] and itempcts[].
CREATE TABLE monster_items (
    monster INTEGER NOT NULL REFERENCES monsters (number) ON DELETE CASCADE, -- the monster
    slot    INTEGER NOT NULL,                                                -- 0..9
    item    INTEGER NOT NULL,                                                -- item carried
    uses    INTEGER NOT NULL,                                                -- uses of it
    pct     INTEGER NOT NULL,                                                -- percent chance it is carried
    PRIMARY KEY (monster, slot)
);

-- spells[NMONSPELLS] with spellpcts[] and spelllvls[]: cast after combat rounds.
CREATE TABLE monster_spells (
    monster INTEGER NOT NULL REFERENCES monsters (number) ON DELETE CASCADE, -- the monster
    slot    INTEGER NOT NULL,                                                -- 0..4
    spell   INTEGER NOT NULL,                                                -- spell number
    pct     INTEGER NOT NULL,                                                -- cumulative percent chance
    level   INTEGER NOT NULL,                                                -- cast level
    PRIMARY KEY (monster, slot)
);

-- ============================================================================
-- Content: spells, classes, races
-- ============================================================================

-- Spell record, WCCSPELS.
CREATE TABLE spells (
    number     INTEGER PRIMARY KEY, -- spell number, key 0
    name       TEXT NOT NULL,       -- spell name
    desc0      TEXT NOT NULL,       -- descs[0]
    desc1      TEXT NOT NULL,       -- descs[1]
    resist     INTEGER NOT NULL,    -- Resist Message
    levelcap   INTEGER NOT NULL,    -- caster level cap, 0 none
    flags      INTEGER NOT NULL,    -- SPF_* bits
    energy     INTEGER NOT NULL,    -- combat energy used
    level      INTEGER NOT NULL,    -- minimum level to cast
    minval     INTEGER NOT NULL,    -- minimum value
    maxval     INTEGER NOT NULL,    -- maximum value
    spell_type INTEGER NOT NULL,    -- type, SPT_* code
    resmode    INTEGER NOT NULL,    -- SRES_* code
    diff       INTEGER NOT NULL,    -- difficulty, 200 or more never fails
    durlvl     INTEGER NOT NULL,    -- duration bonus per level
    target     INTEGER NOT NULL,    -- TGT_* code
    duration   INTEGER NOT NULL,    -- base duration, rounds
    element    INTEGER NOT NULL,    -- ELEM_* code
    resstat    INTEGER NOT NULL,    -- RSTAT_* code
    magery     INTEGER NOT NULL,    -- MAGERY_* code
    msgnum     INTEGER NOT NULL,    -- WCCMSG cast texts
    mana       INTEGER NOT NULL,    -- mana or kai cost
    maxinc     INTEGER NOT NULL,    -- maxval += maxinc * level / maxlvls
    maxlvls    INTEGER NOT NULL,    -- see maxinc
    magelvl    INTEGER NOT NULL,    -- magery level required
    mininc     INTEGER NOT NULL,    -- minval += mininc * level / minlvls
    minlvls    INTEGER NOT NULL,    -- see mininc
    durinc     INTEGER NOT NULL,    -- duration += level / durlvls * durinc
    durlvls    INTEGER NOT NULL,    -- see durinc
    shortnm    TEXT NOT NULL        -- command name, e.g. mmis
);

-- abilities[NRECABILS] with abilvals[].
CREATE TABLE spell_abilities (
    spell   INTEGER NOT NULL REFERENCES spells (number) ON DELETE CASCADE, -- the spell
    slot    INTEGER NOT NULL,                                              -- 0..9
    ability INTEGER NOT NULL,                                              -- ABL_* code
    value   INTEGER NOT NULL,                                              -- abilvals[slot]
    PRIMARY KEY (spell, slot)
);

-- Class record, WCCCLASS.
CREATE TABLE classes (
    number    INTEGER PRIMARY KEY, -- class number, key 0
    name      TEXT NOT NULL,       -- class name
    basehp    INTEGER NOT NULL,    -- max HP += level * basehp
    hplevel   INTEGER NOT NULL,    -- random HP per level
    exp       INTEGER NOT NULL,    -- experience percentage
    attmult   INTEGER NOT NULL,    -- Attack Mult (report only)
    maxenergy INTEGER NOT NULL,    -- report only
    restored  INTEGER NOT NULL,    -- report only
    magery    INTEGER NOT NULL,    -- MAGERY_* code
    magelvl   INTEGER NOT NULL,    -- magery level
    weapon    INTEGER NOT NULL,    -- WCLS_* code, max weapon type
    armour    INTEGER NOT NULL,    -- ARM_* code, max armour type
    combat    INTEGER NOT NULL,    -- combat value
    titles    INTEGER NOT NULL     -- WCCTEXT block: level titles
);

-- abilities[NRECABILS] with abilvals[].
CREATE TABLE class_abilities (
    class   INTEGER NOT NULL REFERENCES classes (number) ON DELETE CASCADE, -- the class
    slot    INTEGER NOT NULL,                                               -- 0..9
    ability INTEGER NOT NULL,                                               -- ABL_* code
    value   INTEGER NOT NULL,                                               -- abilvals[slot]
    PRIMARY KEY (class, slot)
);

-- Race record, WCCRACE.
CREATE TABLE races (
    number    INTEGER PRIMARY KEY, -- race number, key 0
    name      TEXT NOT NULL,       -- race name
    minint    INTEGER NOT NULL,    -- minimum intellect
    minwil    INTEGER NOT NULL,    -- minimum willpower
    minstr    INTEGER NOT NULL,    -- minimum strength
    minhea    INTEGER NOT NULL,    -- minimum health
    minagi    INTEGER NOT NULL,    -- minimum agility
    mincha    INTEGER NOT NULL,    -- minimum charm
    hpbonus   INTEGER NOT NULL,    -- max HP += level * hpbonus
    maxenergy INTEGER NOT NULL,    -- report only
    restored  INTEGER NOT NULL,    -- report only
    basecp    INTEGER NOT NULL,    -- starting character points
    exp       INTEGER NOT NULL,    -- experience percentage
    maxint    INTEGER NOT NULL,    -- maximum intellect
    maxwil    INTEGER NOT NULL,    -- maximum willpower
    maxstr    INTEGER NOT NULL,    -- maximum strength
    maxhea    INTEGER NOT NULL,    -- maximum health
    maxagi    INTEGER NOT NULL,    -- maximum agility
    maxcha    INTEGER NOT NULL     -- maximum charm
);

-- abilities[NRECABILS] with abilvals[].
CREATE TABLE race_abilities (
    race    INTEGER NOT NULL REFERENCES races (number) ON DELETE CASCADE, -- the race
    slot    INTEGER NOT NULL,                                             -- 0..9
    ability INTEGER NOT NULL,                                             -- ABL_* code
    value   INTEGER NOT NULL,                                             -- abilvals[slot]
    PRIMARY KEY (race, slot)
);

-- ============================================================================
-- Content: shops, messages, text blocks, actions, help
-- ============================================================================

-- Shop record, WCCSHOPS. The stock definition is shop_items, the
-- live quantities shop_stock.
CREATE TABLE shops (
    number     INTEGER PRIMARY KEY, -- shop number, key 0
    name       TEXT NOT NULL,       -- shop name
    desc0      TEXT NOT NULL,       -- desc[0]
    desc1      TEXT NOT NULL,       -- desc[1]
    desc2      TEXT NOT NULL,       -- desc[2]
    shop_type  INTEGER NOT NULL,    -- type, SHOP_* code
    minlevel   INTEGER NOT NULL,    -- training: lowest level
    maxlevel   INTEGER NOT NULL,    -- training: highest level
    markup     INTEGER NOT NULL,    -- percent over item cost
    ownable    INTEGER NOT NULL,    -- sysop dump only
    expandable INTEGER NOT NULL,    -- sysop dump only
    trainclass INTEGER NOT NULL     -- training: class, 0 any
);

-- items[NSHOPITEMS] with maxqty[], regentime[], regennum[], regenpct[]. In a gang shop
-- the original reuses maxqty, regentime and regennum; the raw
-- numbers are kept.
CREATE TABLE shop_items (
    shop      INTEGER NOT NULL REFERENCES shops (number) ON DELETE CASCADE, -- the shop
    slot      INTEGER NOT NULL,                                             -- 0..19
    item      INTEGER NOT NULL,                                             -- item number sold
    maxqty    INTEGER NOT NULL,                                             -- restock ceiling
    regentime INTEGER NOT NULL,                                             -- minutes between restocks
    regennum  INTEGER NOT NULL,                                             -- copies per restock
    regenpct  INTEGER NOT NULL,                                             -- restock success percent
    PRIMARY KEY (shop, slot)
);

-- Message record, WCCMSG: a three-part message.
CREATE TABLE messages (
    number INTEGER PRIMARY KEY, -- message number, key 0
    text0  TEXT NOT NULL,       -- msgs[0], to the actor
    text1  TEXT NOT NULL,       -- msgs[1], to the target
    text2  TEXT NOT NULL        -- msgs[2], to the room
);

-- Text-block record, WCCTEXT: one part of a text block. text is stored
-- decoded (the record adds 0x20 to every byte; the importer subtracts it). A block is
-- parts 0, 1, 2.. of one number read in order;
-- link is another block number for callers that chain texts.
CREATE TABLE text_blocks (
    id     BIGSERIAL PRIMARY KEY, -- surrogate key
    part   INTEGER NOT NULL,                  -- continuation number, 0 first (key 0 part 1)
    number INTEGER NOT NULL,                  -- text block number (key 0 part 2)
    link   INTEGER NOT NULL,                  -- linked text block number, 0 none
    text   TEXT NOT NULL                      -- the text, decoded
);
CREATE UNIQUE INDEX text_blocks_part_number ON text_blocks (part, number);
CREATE INDEX text_blocks_number ON text_blocks (number, part);

-- Action record, WCCACTS: a social verb.
CREATE TABLE actions (
    id      BIGSERIAL PRIMARY KEY, -- surrogate key
    name    TEXT NOT NULL,      -- verb, key 0, unique case-insensitive
    targets INTEGER NOT NULL                   -- AT_* bits
);
CREATE UNIQUE INDEX actions_name ON actions (lower(name));

-- texts[NACTTEXTS]: 0/1 single, 2-4 user, 5/6 monster, 7/8 inventory, 9/10 floor.
CREATE TABLE action_texts (
    action_id BIGINT NOT NULL REFERENCES actions (id) ON DELETE CASCADE, -- the action
    slot      INTEGER NOT NULL,                                           -- 0..10
    text      TEXT NOT NULL,                                -- texts[slot]
    PRIMARY KEY (action_id, slot)
);

-- Help topics from WCCMMHLP.MSG (filled by T-011).
CREATE TABLE help_topics (
    id    BIGSERIAL PRIMARY KEY, -- surrogate key
    words TEXT NOT NULL,       -- the topic words, space separated
    text  TEXT NOT NULL                      -- the help text
);
CREATE INDEX help_topics_words ON help_topics (words);

-- ============================================================================
-- State: rooms
-- ============================================================================

-- The live part of the room record, one-to-one with rooms; a fresh import copies it from
-- the content (the .VIR is the virgin state).
CREATE TABLE room_state (
    room_id     BIGINT PRIMARY KEY REFERENCES rooms (id) ON DELETE CASCADE, -- the room
    copper      BIGINT NOT NULL,                                            -- coins lying here
    silver      BIGINT NOT NULL,                                            -- coins lying here
    gold        BIGINT NOT NULL,                                            -- coins lying here
    platinum    BIGINT NOT NULL,                                            -- coins lying here
    runic       BIGINT NOT NULL,                                            -- coins lying here
    hidcopper   BIGINT NOT NULL,                                            -- hidden coins
    hidsilver   BIGINT NOT NULL,                                            -- hidden coins
    hidgold     BIGINT NOT NULL,                                            -- hidden coins
    hidplatinum BIGINT NOT NULL,                                            -- hidden coins
    hidrunic    BIGINT NOT NULL,                                            -- hidden coins
    owner       TEXT NOT NULL,                                 -- owner's user-id (RF_OWNED)
    specalive   BOOLEAN NOT NULL,                                            -- RF_SPECALIVE: the specific monster is alive
    foundmask   BIGINT NOT NULL,                                            -- hidden slots found
    lastkill    INTEGER NOT NULL,                                            -- minute of last kill
    areacount   INTEGER NOT NULL,                                            -- area monsters now
    searchers   INTEGER NOT NULL,                                            -- players who found hidden items
    nummonsters INTEGER NOT NULL                                             -- monsters generated here now
);

-- items[NROOMITEMS]/itemuses/itemcounts (hidden 0) and hiditems[NHIDITEMS]/hiditemuses/
-- hiditemcounts (hidden 1).
CREATE TABLE room_floor_items (
    room_id BIGINT NOT NULL REFERENCES rooms (id) ON DELETE CASCADE, -- the room
    hidden  BOOLEAN NOT NULL,                                         -- true for a hidden slot
    slot    INTEGER NOT NULL,                                         -- 0..16 visible, 0..14 hidden
    item    INTEGER NOT NULL,                                         -- item number
    uses    INTEGER NOT NULL,                                         -- uses left, -1 none
    count   INTEGER NOT NULL,                                         -- extra copies in this slot
    PRIMARY KEY (room_id, hidden, slot)
);

-- monsters[NROOMMONS]: active monsters standing in the room.
CREATE TABLE room_monsters (
    room_id        BIGINT NOT NULL REFERENCES rooms (id) ON DELETE CASCADE, -- the room
    slot           INTEGER NOT NULL,                                         -- 0..14
    active_monster INTEGER NOT NULL,                                         -- active monster number
    PRIMARY KEY (room_id, slot)
);

-- ============================================================================
-- State: monsters and shops
-- ============================================================================

-- Active-monster record, WCCACMSR: a live monster. cash[5] is runic first in
-- the record.
CREATE TABLE active_monsters (
    number     INTEGER PRIMARY KEY, -- active number, key 0
    knownnum   INTEGER NOT NULL,    -- known-monster number
    level      BIGINT NOT NULL,    -- copy of monster.level
    map        BIGINT NOT NULL,    -- map of the current room
    room       BIGINT NOT NULL,    -- current room
    poison     INTEGER NOT NULL,    -- poison per slow tick
    energy     INTEGER NOT NULL,    -- energy left this round
    hp         INTEGER NOT NULL,    -- hit points
    following  TEXT NOT NULL,       -- name of the user followed
    hunting    INTEGER NOT NULL,    -- active monster it tracks
    name       TEXT NOT NULL,       -- name
    wornitem   INTEGER NOT NULL,    -- item worn
    weapon     INTEGER NOT NULL,    -- weapon item
    cash_copper   BIGINT NOT NULL, -- cash[4]: copper carried
    cash_silver   BIGINT NOT NULL, -- cash[3]
    cash_gold     BIGINT NOT NULL, -- cash[2]
    cash_platinum BIGINT NOT NULL, -- cash[1]
    cash_runic    BIGINT NOT NULL, -- cash[0]
    maxhp      INTEGER NOT NULL,    -- maximum hit points
    alignment  INTEGER NOT NULL,    -- ALIGN_* code
    followpct  INTEGER NOT NULL,    -- percent chance to follow
    dr         INTEGER NOT NULL,    -- damage resistance
    ac         INTEGER NOT NULL,    -- armour class
    exp        INTEGER NOT NULL,    -- experience
    maxenergy  INTEGER NOT NULL,    -- energy per round
    friendly   INTEGER NOT NULL,    -- Friend of the followed user
    genroom    BIGINT NOT NULL,    -- room where generated
    inactive   INTEGER NOT NULL,    -- rounds inactive
    flags      INTEGER NOT NULL,    -- AMF_* bits
    montype    INTEGER NOT NULL,    -- MTYPE_* code
    target     INTEGER NOT NULL,    -- user being fought, -1 none
    hpregen    INTEGER NOT NULL,    -- hit points per slow tick
    lastdir    INTEGER NOT NULL,    -- direction of last move, -1
    lastupd    INTEGER NOT NULL,    -- slow-update cycle last applied
    grouprole  INTEGER NOT NULL,    -- FOLLOW_* code
    downrounds INTEGER NOT NULL,    -- rounds left knocked down
    created    BIGINT NOT NULL      -- when generated (now()), Unix seconds UTC
);
CREATE INDEX active_monsters_room ON active_monsters (map, room);

-- trail[NAMTRAIL]: rooms passed, slot 0 latest.
CREATE TABLE active_monster_trail (
    active_monster INTEGER NOT NULL REFERENCES active_monsters (number) ON DELETE CASCADE, -- the monster
    slot           INTEGER NOT NULL,                                                       -- 0..9
    room           BIGINT NOT NULL,                                                       -- room number
    PRIMARY KEY (active_monster, slot)
);

-- hunters[10]: active monsters tracking it.
CREATE TABLE active_monster_hunters (
    active_monster INTEGER NOT NULL REFERENCES active_monsters (number) ON DELETE CASCADE, -- the monster
    slot           INTEGER NOT NULL,                                                       -- 0..9
    hunter         INTEGER NOT NULL,                                                       -- active monster number
    PRIMARY KEY (active_monster, slot)
);

-- items[NMONITEMS] with itemuses[]: items carried.
CREATE TABLE active_monster_items (
    active_monster INTEGER NOT NULL REFERENCES active_monsters (number) ON DELETE CASCADE, -- the monster
    slot           INTEGER NOT NULL,                                                       -- 0..9
    item           INTEGER NOT NULL,                                                       -- item number
    uses           INTEGER NOT NULL,                                                       -- uses left
    PRIMARY KEY (active_monster, slot)
);

-- spells[NMONSPELLS] with spellvals[] and spelldurs[]: spells in effect.
CREATE TABLE active_monster_spells (
    active_monster INTEGER NOT NULL REFERENCES active_monsters (number) ON DELETE CASCADE, -- the monster
    slot           INTEGER NOT NULL,                                                       -- 0..4
    spell          INTEGER NOT NULL,                                                       -- spell number
    value          INTEGER NOT NULL,                                                       -- its value
    rounds         INTEGER NOT NULL,                                                       -- rounds left
    PRIMARY KEY (active_monster, slot)
);

-- The live part of the known-monster record, one-to-one with monsters (the kill time).
CREATE TABLE monster_state (
    monster_id INTEGER PRIMARY KEY REFERENCES monsters (number) ON DELETE CASCADE, -- the known monster
    curalive   INTEGER NOT NULL,                                                   -- alive now
    killed_at  BIGINT NOT NULL                                                     -- killdate, killtime: last killed, Unix seconds UTC, 0 never
);

-- qty[NSHOPITEMS]: live stock of each shop slot, one-to-one with shop_items.
CREATE TABLE shop_stock (
    shop_id INTEGER NOT NULL REFERENCES shops (number) ON DELETE CASCADE, -- the shop
    slot    INTEGER NOT NULL,                                             -- 0..19
    qty     INTEGER NOT NULL,                                             -- quantity in stock
    PRIMARY KEY (shop_id, slot)
);

-- ============================================================================
-- State: characters
-- ============================================================================

-- Player record, WCCUSERS, without the session-only members (leader,
-- group, scanpos) and the unk_* spans. One character per account
-- (decided 2026-10-14). userid is kept for lookups by user-ID; the identity is id.
CREATE TABLE characters (
    id          BIGSERIAL PRIMARY KEY,                          -- character id
    account_id  BIGINT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE, -- owning account, one character each
    userid      TEXT NOT NULL,                 -- BBS user-id, key 0, unique case-insensitive
    firstname   TEXT NOT NULL,                 -- first name, key 1, unique case-insensitive
    lastname    TEXT NOT NULL,                                -- last name
    exp         BIGINT NOT NULL,                                           -- experience total, key 2
    title       TEXT NOT NULL,                                -- title
    race        INTEGER NOT NULL,                                           -- race number
    class       INTEGER NOT NULL,                                           -- class number
    level       INTEGER NOT NULL,                                           -- level
    basint      INTEGER NOT NULL,                                           -- base intellect
    baswil      INTEGER NOT NULL,                                           -- base willpower
    basstr      INTEGER NOT NULL,                                           -- base strength
    bashea      INTEGER NOT NULL,                                           -- base health
    basagi      INTEGER NOT NULL,                                           -- base agility
    bascha      INTEGER NOT NULL,                                           -- base charm
    intel       INTEGER NOT NULL,                                           -- current intellect
    wil         INTEGER NOT NULL,                                           -- current willpower
    str         INTEGER NOT NULL,                                           -- current strength
    hea         INTEGER NOT NULL,                                           -- current health
    agi         INTEGER NOT NULL,                                           -- current agility
    cha         INTEGER NOT NULL,                                           -- current charm
    maxhp       INTEGER NOT NULL,                                           -- maximum hit points
    hp          INTEGER NOT NULL,                                           -- hit points
    maxwgt      INTEGER NOT NULL,                                           -- carrying capacity
    wgt         INTEGER NOT NULL,                                           -- weight carried
    maxenergy   INTEGER NOT NULL,                                           -- energy per round
    energy      INTEGER NOT NULL,                                           -- energy left this round
    poison      INTEGER NOT NULL,                                           -- poison level, positive is poisoned
    maxmr       INTEGER NOT NULL,                                           -- computed magic resistance
    mr          INTEGER NOT NULL,                                           -- magic resistance shown
    map         BIGINT NOT NULL,                                           -- map of the current room
    room        BIGINT NOT NULL,                                           -- room number
    light       INTEGER NOT NULL,                                           -- personal light level
    hunger      INTEGER NOT NULL,                                           -- food level
    thirst      INTEGER NOT NULL,                                           -- drink level
    ninvslots   INTEGER NOT NULL,                                           -- inventory slots (100 at creation)
    nkeyslots   INTEGER NOT NULL,                                           -- key slots (max 50)
    qflags2     BIGINT NOT NULL,                                           -- set flags 33..64
    hpbonus     INTEGER NOT NULL,                                           -- added to max hit points
    manabonus   INTEGER NOT NULL,                                           -- added to max mana
    expbil      BIGINT NOT NULL,                                           -- experience, billions
    exprem      BIGINT NOT NULL,                                           -- experience below a billion
    nspellslots INTEGER NOT NULL,                                           -- spellbook slots
    evil        INTEGER NOT NULL,                                           -- evil points
    lawful      INTEGER NOT NULL,                                           -- -10 forced lawful
    verbose_mode INTEGER NOT NULL,                                          -- verbose: 1 verbose, 0 brief
    bchannel    INTEGER NOT NULL,                                           -- broadcast channel
    fasttalk    INTEGER NOT NULL,                                           -- 1 fast, 0 slow talking
    statline    INTEGER NOT NULL,                                           -- SL_* code
    hidden      INTEGER NOT NULL,                                           -- 1 hidden
    perception  INTEGER NOT NULL,                                           -- perception
    stealth     INTEGER NOT NULL,                                           -- stealth
    martial     INTEGER NOT NULL,                                           -- martial arts
    thievery    INTEGER NOT NULL,                                           -- thievery
    maxmana     INTEGER NOT NULL,                                           -- maximum mana or kai
    mana        INTEGER NOT NULL,                                           -- mana or kai
    spellcast   INTEGER NOT NULL,                                           -- spellcasting
    traps       INTEGER NOT NULL,                                           -- traps
    disarm      INTEGER NOT NULL,                                           -- disarm traps
    picklocks   INTEGER NOT NULL,                                           -- picklocks
    tracking    INTEGER NOT NULL,                                           -- tracking
    copper      BIGINT NOT NULL,                                           -- coins carried
    silver      BIGINT NOT NULL,                                           -- coins carried
    gold        BIGINT NOT NULL,                                           -- coins carried
    platinum    BIGINT NOT NULL,                                           -- coins carried
    runic       BIGINT NOT NULL,                                           -- coins carried
    weapon      INTEGER NOT NULL,                                           -- wielded weapon item number
    autoengage  INTEGER NOT NULL,                                           -- auto-engage on attack
    lives       INTEGER NOT NULL,                                           -- lives left
    aided       INTEGER NOT NULL,                                           -- aided while down
    rank        INTEGER NOT NULL,                                           -- group rank: 1 front, 2 back
    lighttime   INTEGER NOT NULL,                                           -- time left on personal light
    readied     INTEGER NOT NULL,                                           -- inventory slot readied, -1
    delay       INTEGER NOT NULL,                                           -- command delay ticks
    roomlight   INTEGER NOT NULL,                                           -- Alter Room Light total
    gang        TEXT NOT NULL,                                -- gang name, '' none
    hairlen     INTEGER NOT NULL,                                           -- hair length 0..5
    haircol     INTEGER NOT NULL,                                           -- hair colour 0..9
    eyecol      INTEGER NOT NULL,                                           -- eye colour 0..16
    paging      INTEGER NOT NULL,                                           -- 1 paging allowed
    cp          INTEGER NOT NULL,                                           -- unspent character points
    suicpw      TEXT NOT NULL,                                -- suicide password
    attackers   INTEGER NOT NULL,                                           -- enemies engaging
    status      INTEGER NOT NULL,                                           -- ST_* bits
    movedir     INTEGER NOT NULL,                                           -- pending move direction, -1
    dragging    INTEGER NOT NULL DEFAULT 0,                                 -- channel being dragged, -1; 0 until something resets it
    totalcp     INTEGER NOT NULL,                                           -- character points ever earned
    immortal    INTEGER NOT NULL,                                           -- Immortal mark
    pagecnt     INTEGER NOT NULL,                                           -- pages this period
    pagemax     INTEGER NOT NULL,                                           -- pages allowed per 3 seconds
    settings    INTEGER NOT NULL,                                           -- PS_* bits
    gossmax     INTEGER NOT NULL,                                           -- gossips allowed per 3 s
    auctmax     INTEGER NOT NULL,                                           -- auctions allowed per 3 s
    gosscnt     INTEGER NOT NULL,                                           -- gossips this period
    auctcnt     INTEGER NOT NULL,                                           -- auctions this period
    encumpct    INTEGER NOT NULL,                                           -- encumbrance percent
    avbonus     INTEGER NOT NULL,                                           -- best Alter AV value
    acbonus     INTEGER NOT NULL,                                           -- Alter Defence Value total
    dmgbonus    INTEGER NOT NULL,                                           -- Alter Damage total
    mabase      INTEGER NOT NULL,                                           -- martial-arts base 1..75
    arenahp     INTEGER NOT NULL,                                           -- arena hit points
    qflags      BIGINT NOT NULL,                                           -- set flags 1..32
    version     INTEGER NOT NULL,                                           -- record version (22 = 1.11p)
    msglevel    INTEGER NOT NULL,                                           -- MLV_* code
    pvpcnt      INTEGER NOT NULL,                                           -- player attacks counted
    randomhp    INTEGER NOT NULL,                                           -- random hit points (training)
    resttick    INTEGER NOT NULL,                                           -- resting counter
    critbonus   INTEGER NOT NULL,                                           -- Alter Critical Hit Chance
    drbonus     INTEGER NOT NULL,                                           -- Alter Damage Resistance
    drpct       INTEGER NOT NULL,                                           -- Alter DR by percent
    energyuse   INTEGER NOT NULL,                                           -- Alter Energy Usage
    shield      INTEGER NOT NULL,                                           -- Protective Shield
    defmod      INTEGER NOT NULL,                                           -- Defence Modifier
    sesstime    BIGINT NOT NULL DEFAULT 0,                                  -- half-minutes in the realm since the last cleanup
    lifemins    BIGINT NOT NULL,                                           -- minutes played, earlier logins
    moves       INTEGER NOT NULL,                                           -- moves this tick
    lastdate    BIGINT NOT NULL,                                            -- date last on, Unix seconds UTC, 0 never
    status2     INTEGER NOT NULL,                                           -- ST2_* bits
    palette     INTEGER NOT NULL,                                           -- colour palette
    deathdate   BIGINT NOT NULL,                                            -- date of last death, Unix seconds UTC, 0 never
    flags       INTEGER NOT NULL,                                           -- PF_* bits
    sex         INTEGER NOT NULL,                                           -- 'M' or 'F' as a byte
    healrate    INTEGER NOT NULL,                                           -- Alter healing rate
    manarate    INTEGER NOT NULL,                                           -- Alter Mana Regeneration
    racechg     BIGINT NOT NULL,                                           -- lifemins at race change
    lastmon     INTEGER NOT NULL,                                           -- no-attack item not honoured
    lastmoncnt  INTEGER NOT NULL,                                           -- countdown for lastmon
    pvptimer    INTEGER NOT NULL,                                           -- blows left knocked down
    medtick     INTEGER NOT NULL                                            -- meditation counter
);
CREATE UNIQUE INDEX characters_account ON characters (account_id);
CREATE UNIQUE INDEX characters_userid ON characters (lower(userid));
CREATE UNIQUE INDEX characters_firstname ON characters (lower(firstname));
CREATE INDEX characters_exp ON characters (exp DESC);

-- spells[NSPELLCAST] with spellvals[] and spellrnds[]: spells cast on the character.
CREATE TABLE character_spells_cast (
    character_id BIGINT NOT NULL REFERENCES characters (id) ON DELETE CASCADE, -- the character
    slot         INTEGER NOT NULL,                                              -- 0..9
    spell        INTEGER NOT NULL,                                              -- spell number
    value        INTEGER NOT NULL,                                              -- value rolled
    rounds       INTEGER NOT NULL,                                              -- rounds left
    PRIMARY KEY (character_id, slot)
);

-- items[NINVITEMS] with itemuses[]: the inventory.
CREATE TABLE character_inventory (
    character_id BIGINT NOT NULL REFERENCES characters (id) ON DELETE CASCADE, -- the character
    slot         INTEGER NOT NULL,                                              -- 0..99
    item         INTEGER NOT NULL,                                              -- item number
    uses         INTEGER NOT NULL,                                              -- uses left
    PRIMARY KEY (character_id, slot)
);

-- keys[NKEYS] with keyuses[]: the key ring.
CREATE TABLE character_keys (
    character_id BIGINT NOT NULL REFERENCES characters (id) ON DELETE CASCADE, -- the character
    slot         INTEGER NOT NULL,                                              -- 0..49
    item         INTEGER NOT NULL,                                              -- key item number
    uses         INTEGER NOT NULL,                                              -- uses left
    PRIMARY KEY (character_id, slot)
);

-- spellbook[NSPELLBOOK]: known spells.
CREATE TABLE character_spellbook (
    character_id BIGINT NOT NULL REFERENCES characters (id) ON DELETE CASCADE, -- the character
    slot         INTEGER NOT NULL,                                              -- 0..99
    spell        INTEGER NOT NULL,                                              -- spell number
    PRIMARY KEY (character_id, slot)
);

-- worn[NWORN] with wornuses[]: worn items.
CREATE TABLE character_worn (
    character_id BIGINT NOT NULL REFERENCES characters (id) ON DELETE CASCADE, -- the character
    slot         INTEGER NOT NULL,                                              -- 0..19
    item         INTEGER NOT NULL,                                              -- item number
    uses         INTEGER NOT NULL,                                              -- uses left
    PRIMARY KEY (character_id, slot)
);

-- trailmaps[NTRAIL] with trailrooms[]: the last rooms (track).
CREATE TABLE character_trail (
    character_id BIGINT NOT NULL REFERENCES characters (id) ON DELETE CASCADE, -- the character
    slot         INTEGER NOT NULL,                                              -- 0..19
    map          BIGINT NOT NULL,                                              -- map number
    room         BIGINT NOT NULL,                                              -- room number
    PRIMARY KEY (character_id, slot)
);

-- abils[NABILITY] with abilvals[]: the character's abilities.
CREATE TABLE character_abilities (
    character_id BIGINT NOT NULL REFERENCES characters (id) ON DELETE CASCADE, -- the character
    slot         INTEGER NOT NULL,                                              -- 0..29
    ability      INTEGER NOT NULL,                                              -- ABL_* code
    value        INTEGER NOT NULL,                                              -- abilvals[slot]
    PRIMARY KEY (character_id, slot)
);

-- Permanent-info record, genbb "WCC PERMANENT USER": settings kept across
-- deaths and rerolls, one per account.
CREATE TABLE character_profiles (
    account_id BIGINT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE, -- the account
    userid     TEXT NOT NULL,                                    -- user-id (key part 1), kept
    deathdate  BIGINT NOT NULL,                                                -- player.deathdate, Unix seconds UTC, 0 never
    palette    INTEGER NOT NULL,                                               -- player.palette
    msglevel   INTEGER NOT NULL,                                               -- player.msglevel
    settings   INTEGER NOT NULL,                                               -- player.settings
    pagemax    INTEGER NOT NULL,                                               -- player.pagemax
    hairlen    INTEGER NOT NULL,                                               -- player.hairlen
    haircol    INTEGER NOT NULL,                                               -- player.haircol
    eyecol     INTEGER NOT NULL,                                               -- player.eyecol
    statline   INTEGER NOT NULL,                                               -- player.statline
    fasttalk   INTEGER NOT NULL,                                               -- player.fasttalk
    verbose_mode INTEGER NOT NULL,                                             -- verbose: player.verbose
    exp        BIGINT NOT NULL,                                               -- player.exp (saved as 0)
    firstname  TEXT NOT NULL,                                    -- player.firstname
    lastname   TEXT NOT NULL,                                    -- player.lastname
    flags      INTEGER NOT NULL,                                               -- player.flags
    statfmt    TEXT NOT NULL,                                    -- custom statline template
    gossmax    INTEGER NOT NULL,                                               -- player.gossmax
    auctmax    INTEGER NOT NULL,                                               -- player.auctmax
    daybyte    INTEGER NOT NULL,                                               -- the day of the counts, same day only
    expbil     BIGINT NOT NULL,                                               -- kept experience, billions
    exprem     BIGINT NOT NULL                                                -- kept experience, remainder
);

-- Evil-points record, genbb "WCC MAJOR MUD EVIL": evil points kept over a
-- death, one per account.
CREATE TABLE evil_points (
    account_id BIGINT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE, -- the account
    userid     TEXT NOT NULL,                                    -- user-id (key part 1), kept
    evil       INTEGER NOT NULL                                                -- evil points
);

-- Bank-book record, WCCBANKS: key 0 is (userid, bank).
CREATE TABLE bank_books (
    id         BIGSERIAL PRIMARY KEY,                          -- surrogate key
    account_id BIGINT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE, -- the owner
    userid     TEXT NOT NULL,                                 -- owner's user-id, kept
    bank       INTEGER NOT NULL,                                            -- the bank's shop number
    balance    BIGINT NOT NULL,                                            -- money on deposit
    loan       BIGINT NOT NULL                                             -- money owed
);
CREATE UNIQUE INDEX bank_books_account_bank ON bank_books (account_id, bank);

-- Gang record, WCCGANGS.
CREATE TABLE gangs (
    id        BIGSERIAL PRIMARY KEY, -- surrogate key
    key       TEXT NOT NULL,      -- name in upper case, key 0, unique case-insensitive
    name      TEXT NOT NULL,       -- name as entered
    exp       BIGINT NOT NULL,                  -- gang experience, key 1
    leader    TEXT NOT NULL,       -- leader's character name
    created   BIGINT NOT NULL,                   -- date founded, Unix seconds UTC
    members   INTEGER NOT NULL,                  -- number of members
    flags     BIGINT NOT NULL,                  -- GF_* bits
    overexp   BIGINT NOT NULL,                  -- experience past the cap
    overflows INTEGER NOT NULL                   -- extra wraps of overexp
);
CREATE UNIQUE INDEX gangs_key ON gangs (lower(key));
CREATE INDEX gangs_exp ON gangs (exp DESC);

-- Item-ownership record, WCCITOWN: where one copy of a limited item is.
CREATE TABLE item_ownership (
    id      INTEGER PRIMARY KEY, -- record id, key 0
    itemnum INTEGER NOT NULL,    -- item number, key 1 part 1
    owntype INTEGER NOT NULL,    -- OWN_* code, key 1 part 2
    mapnum  BIGINT NOT NULL,    -- OWN_ROOM: map
    roomnum BIGINT NOT NULL,    -- OWN_ROOM: room
    userid  TEXT NOT NULL,       -- OWN_USER: user-id
    monnum  INTEGER NOT NULL     -- OWN_MONSTER: active monster
);
CREATE INDEX item_ownership_item ON item_ownership (itemnum, owntype);

-- ============================================================================
-- Events
-- ============================================================================

-- Append-only game events; the
-- shocst call sites write here as well as to tracing. Pruned to a retention count.
CREATE TABLE events (
    id        BIGSERIAL PRIMARY KEY, -- event id, ascending in time
    at        TEXT NOT NULL,       -- RFC 3339 UTC
    kind      TEXT NOT NULL,       -- event kind, e.g. login, death, level
    session   INTEGER,                           -- session id, NULL for world events
    character BIGINT,                           -- character id, NULL when none
    map       BIGINT,                           -- map of the room concerned, NULL when none
    room      BIGINT,                           -- room concerned, NULL when none
    payload   TEXT NOT NULL                      -- JSON object with the event's details
);
CREATE INDEX events_kind ON events (kind, id);
