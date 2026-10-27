//! Flag sets: one `bitflags!` type per family of stored flag bits, bit values verbatim.
//!
//! Constant names are the catalogue names without the family prefix (`PF_NOGOSSIP` is
//! [`PlayerFlags::NOGOSSIP`]). Bits with no known meaning are kept by `from_bits_retain`, so a
//! record's flag word round-trips. `tests/enum_codes.rs` checks every value against the committed
//! facts.

use bitflags::bitflags;

bitflags! {
    /// Room flags (a 32-bit word in the room record).
    ///
    /// `RF_` bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct RoomFlags: u32 {
        /// `RF_PROTECTED`: protected room
        const PROTECTED = 0x0001;
        /// `RF_PATROL`: patrollable
        const PATROL = 0x0002;
        /// `RF_OWNED`: owned room: owner can CREATE (inf)
        const OWNED = 0x0004;
        /// `RF_SPECALIVE`: specific monster is alive
        const SPECALIVE = 0x0008;
        /// `RF_CLEANUP`: delete items at cleanup
        const CLEANUP = 0x0010;
        /// `RF_ENTRYSPELL`: also cast roomspell on entering
        const ENTRYSPELL = 0x0020;
        /// `RF_GANGHOUSE`: gang house
        const GANGHOUSE = 0x0040;
    }
}

bitflags! {
    /// Gang flags (a 32-bit word in the gang record).
    ///
    /// `GF_` bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct GangFlags: u32 {
        /// `GF_DISBANDED`: disbanded
        const DISBANDED = 0x0001;
        /// `GF_DELETE`: delete at cleanup
        const DELETE = 0x0002;
        /// `GF_HIDDEN`: hidden from the top gangs list
        const HIDDEN = 0x0004;
        /// `GF_OVERFLOW`: experience has overflowed
        const OVERFLOW = 0x0008;
    }
}

bitflags! {
    /// What a social action can target.
    ///
    /// `AT_` bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct ActionTargets: u16 {
        /// `AT_SINGLE`: no target
        const SINGLE = 0x0001;
        /// `AT_USER`: another user
        const USER = 0x0002;
        /// `AT_MONSTER`: a monster
        const MONSTER = 0x0004;
        /// `AT_INVENTORY`: an inventory item
        const INVENTORY = 0x0008;
        /// `AT_FLOOR`: an item on the floor
        const FLOOR = 0x0010;
        /// `AT_KEEP`: not replaced by the database update
        const KEEP = 0x0040;
    }
}

bitflags! {
    /// Active monster flags.
    ///
    /// `AMF_` bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct ActiveMonsterFlags: u16 {
        /// `AMF_CHARMED`: charmed
        const CHARMED = 0x0001;
        /// `AMF_FOUGHT`: fought this round
        const FOUGHT = 0x0002;
        /// `AMF_TARGETED`: counted as a target
        const TARGETED = 0x0004;
        /// `AMF_DOWN`: knocked down
        const DOWN = 0x0008;
        /// `AMF_ATKPEN`: attack penalty
        const ATKPEN = 0x0200;
    }
}

bitflags! {
    /// Per-session state flags.
    ///
    /// `USF_` bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct SessionFlags: u32 {
        /// `USF_TOPTEN`: TOPTEN list paging
        const TOPTEN = 0x0001;
        /// `USF_GANGTOP`: gang TOPTEN list (unused in 1.11p)
        const GANGTOP = 0x0002;
        /// `USF_GANGLIST`: gang member list paging
        const GANGLIST = 0x0004;
        /// `USF_REPORT`: reported-user list paging
        const REPORT = 0x0008;
        /// `USF_CHARMODE`: character-mode input (inf)
        const CHARMODE = 0x0010;
        /// `USF_INACTWARN`: inactivity warning given (inf)
        const INACTWARN = 0x0020;
        /// `USF_SYSOPLIST`: sysop user display in progress (inf)
        const SYSOPLIST = 0x0040;
    }
}

bitflags! {
    /// Player status flags: the state of the moment.
    ///
    /// `ST_` bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct PlayerStatus: u16 {
        /// `ST_SUICIDE`: SUICIDE confirmed; the character is deleted
        const SUICIDE = 0x0001;
        /// `ST_DRAGGING`: dragging the player in dragging
        const DRAGGING = 0x0002;
        /// `ST_SNEAKING`: sneaking
        const SNEAKING = 0x0004;
        /// `ST_EXITING`: leaving the game (X) after a wait
        const EXITING = 0x0008;
        /// `ST_SEARCHED`: found hidden items in this room (counted in the room's searcher count)
        const SEARCHED = 0x0010;
        /// `ST_HUNGUP`: hung up while playing; punished at the next logon
        const HUNGUP = 0x0020;
        /// `ST_JUSTARRIVED`: entered the room this round; a sneaker is not seen or attacked
        const JUSTARRIVED = 0x0040;
        /// `ST_CONFUSED`: ability Confusion
        const CONFUSED = 0x0080;
        /// `ST_MUTE`: ability Mute: cannot speak
        const MUTE = 0x0100;
        /// `ST_TARGETGONE`: player fought in autocombat has left the room
        const TARGETGONE = 0x0200;
        /// `ST_RESTING`: resting (REST)
        const RESTING = 0x0400;
        /// `ST_PARALYSED`: ability Paralyze
        const PARALYSED = 0x0800;
        /// `ST_BLIND`: ability Blind User
        const BLIND = 0x1000;
        /// `ST_SEEHIDDEN`: ability See Hidden
        const SEEHIDDEN = 0x2000;
        /// `ST_SPOTTED`: seen or fought this round: hiding and sneaking are harder
        const SPOTTED = 0x8000;
        /// `ST_FROMABILS`: the bits set from abilities, cleared before re-applying them
        /// Mask, not a bit: the bits set from abilities, cleared before re-applying them.
        const FROMABILS = 0x3980;
    }
}

bitflags! {
    /// More player status flags.
    ///
    /// `ST2_` bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct PlayerStatus2: u16 {
        /// `ST2_NAMECHECK`: name check of a new character in progress
        const NAMECHECK = 0x0001;
        /// `ST2_SLOWED`: ability Slowness
        const SLOWED = 0x0002;
        /// `ST2_QUICKENED`: ability Quickness
        const QUICKENED = 0x0004;
        /// `ST2_ATTACKED`: attacked this round (becomes ST_SPOTTED at the next round)
        const ATTACKED = 0x0008;
        /// `ST2_AREATARGET`: counted as a target of the area spell being cast
        const AREATARGET = 0x0010;
        /// `ST2_HELD`: ability Hold person
        const HELD = 0x0020;
        /// `ST2_STATRESET`: stats reset on loading an old record; not yet in the realm
        const STATRESET = 0x0040;
        /// `ST2_AFRAID`: ability Flee: too afraid to fight
        const AFRAID = 0x0080;
        /// `ST2_MEDITATING`: meditating (MEDITATE)
        const MEDITATING = 0x0100;
        /// `ST2_KNOCKEDDOWN`: smashed to the ground (stunned)
        const KNOCKEDDOWN = 0x0200;
        /// `ST2_EXPMARKED`: set by SYSOP EXPERIENCE after a restructure; never tested
        const EXPMARKED = 0x2000;
        /// `ST2_FROMABILS`: the bits set from abilities, cleared before re-applying them
        /// Mask, not a bit: the bits set from abilities, cleared before re-applying them.
        const FROMABILS = 0x00A6;
    }
}

bitflags! {
    /// Player settings and account state.
    ///
    /// `PS_` bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct PlayerSettings: u16 {
        /// `PS_BLINDFOLLOW`: follow mode Blind
        const BLINDFOLLOW = 0x0001;
        /// `PS_ATKINTERRUPT`: attack interrupts on
        const ATKINTERRUPT = 0x0002;
        /// `PS_CANCAST`: may still cast this round (set each round, cleared by a cast)
        const CANCAST = 0x0004;
        /// `PS_NORECEIVE`: receiving items disabled
        const NORECEIVE = 0x0008;
        /// `PS_EVILWARN`: warn before an evil act
        const EVILWARN = 0x0010;
        /// `PS_NOTELEPATH`: telepaths (whispers) not allowed
        const NOTELEPATH = 0x0020;
        /// `PS_EVILADDED`: evil points were charged; never tested (meaning inferred)
        const EVILADDED = 0x0040;
        /// `PS_OLDBLOCKENT`: old place of the block-entrance option, moved to PF_NOENTRYMSG
        const OLDBLOCKENT = 0x0080;
        /// `PS_BRIEFTALK`: talking responses Brief
        const BRIEFTALK = 0x0100;
        /// `PS_NOACTIONS`: social actions turned off
        const NOACTIONS = 0x0200;
        /// `PS_KEEPEXP`: keep some exp on death
        const KEEPEXP = 0x0400;
        /// `PS_TECHNICAL`: message style Technical
        const TECHNICAL = 0x0800;
        /// `PS_VALIDATED`: name checked; record may be saved
        const VALIDATED = 0x2000;
        /// `PS_NOTOPTEN`: left out of TOPTEN (sysop)
        const NOTOPTEN = 0x4000;
        /// `PS_PAYING`: account had the save key when entering (paying account)
        const PAYING = 0x8000;
    }
}

bitflags! {
    /// Player options and pending notices.
    ///
    /// `PF_` bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct PlayerFlags: u16 {
        /// `PF_NOENTRYMSG`: block entrance message
        const NOENTRYMSG = 0x0001;
        /// `PF_NOGOSSIP`: gossip off
        const NOGOSSIP = 0x0002;
        /// `PF_NOAUCTION`: auction off
        const NOAUCTION = 0x0004;
        /// `PF_GANGONLINE`: view gang members: online only
        const GANGONLINE = 0x0008;
        /// `PF_SPLITSCREEN`: ANSI split-screen mode; never seen set (inferred)
        /// Deprecated: never seen set in play and not given any meaning here; the bit is
        /// kept only so flag words round-trip.
        const SPLITSCREEN = 0x0010;
        /// `PF_CLASSSTEALTH`: stealth from Class Stealth
        const CLASSSTEALTH = 0x0020;
        /// `PF_RACESTEALTH`: stealth from Race Stealth
        const RACESTEALTH = 0x0040;
        /// `PF_RACECHANGED`: one-time race/class change used
        const RACECHANGED = 0x0080;
        /// `PF_LIEUTENANT`: gang lieutenant
        const LIEUTENANT = 0x0100;
        /// `PF_PROMOTED`: promoted while offline (notice)
        const PROMOTED = 0x0200;
        /// `PF_DEMOTED`: demoted while offline (notice)
        const DEMOTED = 0x0400;
        /// `PF_HOUSECLOSED`: gang house closed (notice)
        const HOUSECLOSED = 0x0800;
        /// `PF_HOUSEITEMS`: gang house items taken (notice)
        const HOUSEITEMS = 0x1000;
        /// `PF_NEWEXP`: experience is on the 1.11 system
        const NEWEXP = 0x2000;
        /// `PF_SOLDDEED`: sold to a deed shop; may not buy a deed again
        const SOLDDEED = 0x4000;
    }
}

bitflags! {
    /// Flags of a Hidden exit, held in the exit's first parameter (a 32-bit word).
    ///
    /// `EXF_` bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct ExitFlags: u32 {
        /// `EXF_PASSABLE`: passable
        const PASSABLE = 0x0001;
        /// `EXF_NEEDSEARCH`: must be found by searching
        const NEEDSEARCH = 0x0002;
        /// `EXF_VISSEARCHED`: found by a search, now visible; hidden again by the background
        const VISSEARCHED = 0x0004;
        /// `EXF_VISBLOCKED`: visible but blocked
        const VISBLOCKED = 0x0008;
        /// `EXF_BLOCKED1`: blocked step 1; steps 1..n are set together when the exit is blocked again, n = the absolute value of the exit's second parameter
        const BLOCKED1 = 0x0010;
        /// `EXF_BLOCKED2`: blocked step 2
        const BLOCKED2 = 0x0020;
        /// `EXF_BLOCKED3`: blocked step 3
        const BLOCKED3 = 0x0040;
        /// `EXF_BLOCKED4`: blocked step 4
        const BLOCKED4 = 0x0080;
        /// `EXF_BLOCKED5`: blocked step 5
        const BLOCKED5 = 0x0100;
        /// `EXF_BLOCKED6`: blocked step 6
        const BLOCKED6 = 0x0200;
        /// `EXF_BLOCKED7`: blocked step 7
        const BLOCKED7 = 0x0400;
        /// `EXF_BLOCKED8`: blocked step 8
        const BLOCKED8 = 0x0800;
        /// `EXF_BLOCKED9`: blocked step 9
        const BLOCKED9 = 0x1000;
        /// `EXF_BLOCKED10`: blocked step 10
        const BLOCKED10 = 0x2000;
    }
}

bitflags! {
    /// Spell flags.
    ///
    /// `SPF_` bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct SpellFlags: u8 {
        /// `SPF_ALTMSG`: alternate messaging
        const ALTMSG = 0x01;
    }
}
