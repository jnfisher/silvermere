//! The `player` record (WCCUSERS) and the two genbb records that outlive a character.

use serde::Serialize;

use crate::enums::{MessageLevel, StatlineMode};
use crate::flags::{PlayerFlags, PlayerSettings, PlayerStatus, PlayerStatus2};
use crate::ids::{ClassNo, ItemNo, MapId, RaceNo, RoomId, SpellNo};
use crate::records::layout::{self, Layout};
use crate::records::{AbilitySlot, Coins};
use crate::sizes::{NABILITY, NSPELLCAST, NTRAIL, NWORN};

/// A character: the `player` record, minus the BBS-isms.
///
/// Dropped as session-only: `leader`, `group[5]`,
/// `dragging`, `scanpos`. `userid` is kept for lookups by user id but is not the
/// identity; the database key is [`crate::ids::CharacterId`]. The `unk_*` members are not
/// carried. Every other field keeps its record member name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Character {
    /// +0x000 BBS user-id (key 0).
    pub userid: String,
    /// +0x01E first name (key 1).
    pub firstname: String,
    /// +0x029 last name.
    pub lastname: String,
    /// +0x03C experience total (key 2).
    pub exp: u32,
    /// +0x040 spells cast on the character.
    pub spells: [SpellNo; NSPELLCAST],
    /// +0x054 value rolled for each.
    pub spellvals: [i16; NSPELLCAST],
    /// +0x068 rounds left for each.
    pub spellrnds: [i16; NSPELLCAST],
    /// +0x07C title.
    pub title: String,
    /// +0x090 race number.
    pub race: RaceNo,
    /// +0x092 class number.
    pub class: ClassNo,
    /// +0x094 level.
    pub level: u16,
    /// +0x096 base intellect.
    pub basint: u16,
    /// +0x098 base willpower.
    pub baswil: u16,
    /// +0x09A base strength.
    pub basstr: u16,
    /// +0x09C base health.
    pub bashea: u16,
    /// +0x09E base agility.
    pub basagi: u16,
    /// +0x0A0 base charm.
    pub bascha: u16,
    /// +0x0A2 current intellect.
    pub intel: u16,
    /// +0x0A4 current willpower.
    pub wil: u16,
    /// +0x0A6 current strength.
    pub str: u16,
    /// +0x0A8 current health.
    pub hea: u16,
    /// +0x0AA current agility.
    pub agi: u16,
    /// +0x0AC current charm.
    pub cha: u16,
    /// +0x0AE maximum hit points.
    pub maxhp: i16,
    /// +0x0B0 hit points.
    pub hp: i16,
    /// +0x0B2 carrying capacity.
    pub maxwgt: i16,
    /// +0x0B4 weight carried.
    pub wgt: u16,
    /// +0x0B8 energy per round.
    pub maxenergy: u16,
    /// +0x0BA energy left this round.
    pub energy: u16,
    /// +0x0BE poison level, >0 poisoned.
    pub poison: i16,
    /// +0x0C0 computed magic resistance.
    pub maxmr: u16,
    /// +0x0C2 magic resistance (shown).
    pub mr: u16,
    /// +0x0C4 map of the current room.
    pub map: MapId,
    /// +0x0C8 room number.
    pub room: RoomId,
    /// +0x0CC personal light level.
    pub light: i16,
    /// +0x0CE food level.
    pub hunger: i16,
    /// +0x0D0 drink level.
    pub thirst: i16,
    /// +0x0D5 inventory slots (100 at creation).
    pub ninvslots: u8,
    /// +0x0D6 inventory item numbers (`NINVITEMS`).
    pub items: Vec<ItemNo>,
    /// +0x266 uses per inventory slot (`NINVITEMS`).
    pub itemuses: Vec<i16>,
    /// +0x32E key slots (max 50).
    pub nkeyslots: u8,
    /// +0x32F key item numbers (`NKEYS`).
    pub keys: Vec<ItemNo>,
    /// +0x3F7 uses per key slot (`NKEYS`).
    pub keyuses: Vec<i16>,
    /// +0x45B "set" flags 33..64.
    pub qflags2: u32,
    /// +0x463 added to max hit points.
    pub hpbonus: i32,
    /// +0x467 added to max mana.
    pub manabonus: i32,
    /// +0x46B experience, billions. Whether `expbil`/`exprem` become one `u64` is T-023's call.
    pub expbil: u32,
    /// +0x46F experience below a billion.
    pub exprem: u32,
    /// +0x473 spellbook slots.
    pub nspellslots: u8,
    /// +0x474 known spell numbers (`NSPELLBOOK`).
    pub spellbook: Vec<SpellNo>,
    /// +0x53C evil points.
    pub evil: i16,
    /// +0x53E -10 = forced lawful.
    pub lawful: i8,
    /// +0x547 maps of the last rooms.
    pub trailmaps: [MapId; NTRAIL],
    /// +0x597 last rooms.
    pub trailrooms: [RoomId; NTRAIL],
    /// +0x5E7 1 verbose, 0 brief.
    pub verbose: u8,
    /// +0x5E8 broadcast channel.
    pub bchannel: i16,
    /// +0x5EA 1 fast, 0 slow talking.
    pub fasttalk: u8,
    /// +0x5EB status line mode.
    pub statline: StatlineMode,
    /// +0x5EC 1 = hidden.
    pub hidden: u8,
    /// +0x5ED perception.
    pub perception: i16,
    /// +0x5EF stealth.
    pub stealth: i16,
    /// +0x5F1 martial arts.
    pub martial: i16,
    /// +0x5F3 thievery.
    pub thievery: i16,
    /// +0x5F5 maximum mana / kai.
    pub maxmana: u16,
    /// +0x5F7 mana / kai.
    pub mana: i16,
    /// +0x5F9 spellcasting.
    pub spellcast: i16,
    /// +0x5FB traps.
    pub traps: i16,
    /// +0x5FD disarm traps.
    pub disarm: i16,
    /// +0x5FF picklocks.
    pub picklocks: i16,
    /// +0x601 tracking.
    pub tracking: i16,
    /// +0x603..+0x613 coins carried (`runic`, `platinum`, `gold`, `silver`, `copper`).
    pub coins: Coins,
    /// +0x617 wielded weapon item number.
    pub weapon: ItemNo,
    /// +0x61F worn item numbers.
    pub worn: [ItemNo; NWORN],
    /// +0x66F uses per worn item.
    pub wornuses: [i16; NWORN],
    /// +0x697 auto-engage on attack.
    pub autoengage: i16,
    /// +0x699 lives left.
    pub lives: i16,
    /// +0x69B aided while down.
    pub aided: u8,
    /// +0x69E group rank: 1 front, 2 back.
    pub rank: u8,
    /// +0x6A9 time left on personal light.
    pub lighttime: i16,
    /// +0x6AB inventory slot readied, -1.
    pub readied: i8,
    /// +0x6AC command delay ticks.
    pub delay: u8,
    /// +0x6AD "Alter Room Light" total.
    pub roomlight: i16,
    /// +0x6B9 gang name, "" = none.
    pub gang: String,
    /// +0x6CD hair length 0..5 ([`crate::names::HAIR_LENGTHS`]).
    pub hairlen: u8,
    /// +0x6CE hair colour 0..9 ([`crate::names::HAIR_COLOURS`]).
    pub haircol: u8,
    /// +0x6D0 eye colour 0..16 ([`crate::names::EYE_COLOURS`]).
    pub eyecol: u8,
    /// +0x6D2 1 = paging allowed.
    pub paging: u8,
    /// +0x6D3 unspent character points.
    pub cp: i16,
    /// +0x6D5 suicide password.
    pub suicpw: String,
    /// +0x6DE enemies engaging.
    pub attackers: i32,
    /// +0x6E2 status flags.
    pub status: PlayerStatus,
    /// +0x6E4 pending move direction, -1.
    pub movedir: i16,
    /// +0x6E6 channel being dragged, -1. Saved with the record, so it is -1 only once something
    /// has reset it (a new character starts at 0).
    pub dragging: i16,
    /// +0x6E8 character points ever earned.
    pub totalcp: u16,
    /// +0x6EA " -- Immortal !".
    pub immortal: u8,
    /// +0x6EB pages this period.
    pub pagecnt: u8,
    /// +0x6EC pages allowed per 3 seconds.
    pub pagemax: u8,
    /// +0x6ED option flags.
    pub settings: PlayerSettings,
    /// +0x6EF gossips allowed per 3 s.
    pub gossmax: u8,
    /// +0x6F0 auctions allowed per 3 s.
    pub auctmax: u8,
    /// +0x6F1 gossips this period.
    pub gosscnt: u8,
    /// +0x6F2 auctions this period.
    pub auctcnt: u8,
    /// +0x6F5 encumbrance percent.
    pub encumpct: i16,
    /// +0x6F7 best "Alter AV" value.
    pub avbonus: i16,
    /// +0x6F9 "Alter Defence Value" total.
    pub acbonus: i16,
    /// +0x6FB "Alter Damage" total.
    pub dmgbonus: i16,
    /// +0x6FD martial-arts base 1..75.
    pub mabase: u8,
    /// +0x6FE arena hit points.
    pub arenahp: i16,
    /// +0x705 "set" flags 1..32.
    pub qflags: u32,
    /// +0x709 record version (22 = 1.11p).
    pub version: i16,
    /// +0x70B message level (inverted: 0 high, 10 medium, 20 low).
    pub msglevel: MessageLevel,
    /// +0x70C player attacks counted.
    pub pvpcnt: u8,
    /// +0x70D random hit points (training).
    pub randomhp: u8,
    /// +0x722 ability numbers with +0x75E their values (two parallel arrays).
    pub abils: [AbilitySlot; NABILITY],
    /// +0x79A resting counter.
    pub resttick: u8,
    /// +0x79B "Alter Critical Hit Chance".
    pub critbonus: i16,
    /// +0x79D "Alter Damage Resistance".
    pub drbonus: i16,
    /// +0x79F "Alter DR by percent".
    pub drpct: i16,
    /// +0x7A1 "Alter Energy Usage".
    pub energyuse: i16,
    /// +0x7A3 "Protective Shield".
    pub shield: i16,
    /// +0x7A5 "Defence Modifier".
    pub defmod: i16,
    /// +0x6F3 half-minutes in the realm since the last midnight cleanup (one more per slow
    /// cycle; folded into `lifemins` by the cleanup). Stored, as in the original: it builds up
    /// across logins.
    pub sesstime: u32,
    /// +0x7A7 minutes played, earlier logins.
    pub lifemins: u32,
    /// +0x7AB moves this tick.
    pub moves: u8,
    /// +0x7AC date last on (a DOS date in the record), Unix seconds UTC; 0 = never.
    pub lastdate: i64,
    /// +0x7AE more status flags.
    pub status2: PlayerStatus2,
    /// +0x7B4 colour palette.
    pub palette: u8,
    /// +0x7B5 date of last death (a DOS date in the record), Unix seconds UTC; 0 = never.
    pub deathdate: i64,
    /// +0x7B7 option/state flags.
    pub flags: PlayerFlags,
    /// +0x7B9 'M' or 'F'.
    pub sex: u8,
    /// +0x7BA "Alter healing rate".
    pub healrate: i16,
    /// +0x7BC "Alter Mana Regeneration".
    pub manarate: i16,
    /// +0x7BE lifemins at race change.
    pub racechg: u32,
    /// +0x7C6 no-attack item not honoured.
    pub lastmon: i32,
    /// +0x7CA countdown for lastmon.
    pub lastmoncnt: u16,
    /// +0x7CC blows left knocked down.
    pub pvptimer: u8,
    /// +0x7CD meditation counter.
    pub medtick: u8,
}

impl Character {
    /// Byte layout of the `player` record.
    pub const LAYOUT: &'static Layout = &layout::PLAYER;
    /// The `player` record length: 1998 bytes.
    pub const RECORD_LEN: usize = 1998;
}

/// The settings kept across deaths: genbb `"WCC PERMANENT USER"`, the `permrec` record.
/// `statfmt` keeps the custom statline template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PermanentRecord {
    /// +0x00 user-id (key part 1).
    pub userid: String,
    /// +0x37 `player.deathdate`, Unix seconds UTC; 0 = never.
    pub deathdate: i64,
    /// +0x39 `player.palette`.
    pub palette: u8,
    /// +0x3A `player.msglevel`.
    pub msglevel: MessageLevel,
    /// +0x3B `player.settings`.
    pub settings: PlayerSettings,
    /// +0x3D `player.pagemax`.
    pub pagemax: u8,
    /// +0x3E `player.hairlen`.
    pub hairlen: u8,
    /// +0x3F `player.haircol`.
    pub haircol: u8,
    /// +0x41 `player.eyecol`.
    pub eyecol: u8,
    /// +0x43 `player.statline`.
    pub statline: StatlineMode,
    /// +0x44 `player.fasttalk`.
    pub fasttalk: u8,
    /// +0x45 `player.verbose`.
    pub verbose: u8,
    /// +0x47 `player.exp` (saved as 0).
    pub exp: u32,
    /// +0x4B `player.firstname`.
    pub firstname: String,
    /// +0x56 `player.lastname`.
    pub lastname: String,
    /// +0x69 `player.flags`.
    pub flags: PlayerFlags,
    /// +0x6B the custom statline template (held in the session while playing).
    pub statfmt: String,
    /// +0xA8 `player.gossmax`.
    pub gossmax: u8,
    /// +0xA9 `player.auctmax`.
    pub auctmax: u8,
    /// +0xAA the session's day byte, same day only.
    pub daybyte: u8,
    /// +0xAF kept experience, billions.
    pub expbil: u32,
    /// +0xB3 kept experience, remainder.
    pub exprem: u32,
}

impl PermanentRecord {
    /// Byte layout of the `permrec` record.
    pub const LAYOUT: &'static Layout = &layout::PERMREC;
    /// The `permrec` record length: 205 bytes.
    pub const RECORD_LEN: usize = 205;
}

/// Evil points kept over a death: genbb `"WCC MAJOR MUD EVIL"`, the `evilrec` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvilRecord {
    /// +0x00 user-id (key part 1).
    pub userid: String,
    /// +0x37 evil points.
    pub evil: i16,
}

impl EvilRecord {
    /// Byte layout of the `evilrec` record.
    pub const LAYOUT: &'static Layout = &layout::EVILREC;
    /// The `evilrec` record length: 57 bytes.
    pub const RECORD_LEN: usize = 57;
}
