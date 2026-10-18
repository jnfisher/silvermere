//! The room display: [`display_room`], [`long_room_desc`] and [`desc_from_file`].
//!
//! Every part is a typed [`Msg`] appended to the viewer's
//! [`OutputBatch`](crate::OutputBatch) in the order below, the order of the original's
//! screen, and the batch goes out once at the end of the event:
//!
//! 1. `ANSI_CLRLINE`, and `DBG: <room> Map: <map>` for a sysop with debug on;
//! 2. `can_see`'s refusal (`You are blind.`, `The room is ... - you can't see anything`)
//!    and nothing else, when the viewer cannot see;
//! 3. the title in light cyan, or the first line of a `FILE DESCRIPTION` file;
//! 4. the colliseum warning while the arena is not death-free;
//! 5. with `long`, the description paragraph (and a shop's, and a bank's rates);
//! 6. `You notice ... here.`;
//! 7. `Also here: ...`, players then monsters;
//! 8. `Obvious exits: ...`;
//! 9. `The room is <light>` below light 0.
//!
//! The title, description, items, `Also here:` and exits lines wrap at 79 columns
//! (`Msg::wrap`).

use std::collections::HashMap;
use std::path::Path;

use silvermere_text::colour::{ColRow, Colour, Palette};
use silvermere_text::{Msg, ansi, cp437};
use silvermere_types::enums::{Alignment, DebugLevel, RoomType, ShopType};
use silvermere_types::flags::{ActiveMonsterFlags, PlayerStatus};
use silvermere_types::ids::{ActiveMonsterId, MapId, RoomId, RoomKey, SessionId};
use silvermere_types::records::Room;

use super::exits::{DirNames, exits_msg};
use super::light::can_see;
use super::room_items::items_msg;
use crate::world::World;

/// Evil points from which a character is an Outlaw.
pub const EVIL_OUTLAW: i16 = 40;

/// The arguments of the room display that matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoomMode {
    /// Add the description paragraph (LOOK, verbose players).
    pub long: bool,
    /// Leave out the movement line, which never printed anything (see
    /// [`super::movement_noise`]); kept so a caller still says which display it wants. The light
    /// line is shown either way.
    pub skip_move: bool,
}

impl RoomMode {
    /// The short display.
    pub const SHORT: RoomMode = RoomMode {
        long: false,
        skip_move: false,
    };
    /// The display with the description.
    pub const LONG: RoomMode = RoomMode {
        long: true,
        skip_move: false,
    };

    /// The display a player's verbose setting asks for: long when it is set.
    pub const fn verbose(verbose: u8) -> RoomMode {
        RoomMode {
            long: verbose != 0,
            skip_move: false,
        }
    }
}

/// The text files `FILE DESCRIPTION` rooms name (the gang houses' `WCCnnnnn.HSE`), loaded
/// once from `data/assets/` and kept in memory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assets {
    files: HashMap<String, Vec<u8>>,
}

impl Assets {
    /// Every regular file in `dir`, by upper-case file name.
    pub fn load_dir(dir: &Path) -> std::io::Result<Assets> {
        let mut assets = Assets::default();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let name = entry.file_name().to_string_lossy().into_owned();
                assets.insert(&name, std::fs::read(entry.path())?);
            }
        }
        Ok(assets)
    }

    /// Adds or replaces a file.
    pub fn insert(&mut self, name: &str, bytes: Vec<u8>) {
        self.files.insert(name.to_ascii_uppercase(), bytes);
    }

    /// A file by name, any case (DOS file names).
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.files
            .get(&name.trim().to_ascii_uppercase())
            .map(Vec::as_slice)
    }
}

/// The module globals the display reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayGlobals {
    /// `arenasafe` (default 1): colliseum rooms are death-free; the SYSOP ARENA command
    /// switches it (T-055).
    pub arena_safe: bool,
    /// The live direction names.
    pub dirnames: DirNames,
    /// The `FILE DESCRIPTION` files.
    pub assets: Assets,
}

impl Default for DisplayGlobals {
    fn default() -> Self {
        Self {
            arena_safe: true,
            dirnames: DirNames::default(),
            assets: Assets::default(),
        }
    }
}

/// The room display for the room `id` stands in.
pub fn display_room(world: &mut World, id: SessionId, mode: RoomMode) {
    let Some(s) = world.session(id) else {
        return;
    };
    let Some(plr) = s.player() else {
        return;
    };
    let key = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let debug = s.flags.debug != DebugLevel::Off;
    let palette = Palette::repaired(plr.palette);
    // A room that cannot be loaded sends the player to the reincarnation room of map 1 and
    // shows nothing this time. T-026: the exits as they stand now (doors opened, hidden
    // exits found)
    let Some(room) = crate::movement::exits::live_room(world, key) else {
        tracing::error!(
            ?key,
            session = id.0,
            "invalid room; player sent to reinc room"
        );
        let reinc = RoomId(world.config.reinc_room);
        if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            p.map = MapId(1);
            p.room = reinc;
        }
        return;
    };
    let push = |world: &mut World, msg: Msg| {
        if let Some(s) = world.session_mut(id) {
            s.output.push(msg);
        }
    };
    push(world, Msg::Clrline);
    if debug {
        push(
            world,
            Msg::RoomDebug {
                room: room.key.room.0,
                map: room.key.map.0,
            },
        );
    }
    let Some((light, lightdesc)) = can_see(world, id) else {
        return;
    };
    if let Some((file, prefix)) = file_description(&room.desc) {
        // The file's first line only (`once`), then `clear_output`, after the light cyan.
        if let Some(lines) = desc_from_file(world, file, prefix, true) {
            match lines.into_iter().next() {
                Some(name) => push(world, Msg::RoomTitle { name }),
                None => {
                    if let Some(s) = world.session_mut(id) {
                        s.output.push_raw(ansi::ANSI_LTCYAN);
                    }
                }
            }
        }
    } else {
        push(
            world,
            Msg::RoomTitle {
                name: room.name.clone(),
            },
        );
    }
    if !world.display.arena_safe && room.room_type == RoomType::Colliseum {
        push(world, Msg::ArenaWarning);
    }
    if mode.long {
        long_room_desc(world, id, &room);
    }
    if let Some(msg) = items_msg(world, key) {
        push(world, msg);
    }
    if let Some(who) = also_here(world, id, key, palette) {
        push(world, Msg::RoomAlsoHere { who });
    }
    let exits = exits_msg(&room, &world.display.dirnames, &world.messages);
    push(world, exits);
    // The light line below 0, with or without `skip_move`.
    if light < 0 {
        push(
            world,
            Msg::RoomLight {
                light: lightdesc.text().into(),
            },
        );
    }
}

/// A first description line of `FILE DESCRIPTION` (case not counting) with a non-empty
/// second line: the file name and the line prefix.
fn file_description(desc: &[String]) -> Option<(&str, &str)> {
    let first = desc.first()?;
    let file = desc.get(1)?;
    (first.trim_end().eq_ignore_ascii_case("FILE DESCRIPTION") && !file.is_empty())
        .then(|| (file.as_str(), desc.get(2).map_or("", String::as_str)))
}

/// The stock distribution ships none of 84 of the 131 `FILE DESCRIPTION` names, so a
/// player standing in such a room would log a line per keystroke: warn once per file name
/// per process (T-025 review, silvermere#30).
fn warn_missing_file_once(file: &str) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let first = SEEN
        .get_or_init(Mutex::default)
        .lock()
        .is_ok_and(|mut seen| seen.insert(file.to_owned()));
    if first {
        tracing::warn!(file, "can't display description file (not in the assets)");
    }
}

/// The lines of `file` that start with `prefix`, without it, up to the first line after
/// them that does not, or only the first with `once`; at most 500 lines are read. `None`
/// (and a log line) when the file is missing.
///
/// The BBS kernel's text-file scanner is not part of the game, so its behaviour is
/// MBBSEmu's (the differential oracle; `Majorbbs.cs` `tfsrdl`, `tfspfx`) *(inferred)*:
/// `tfsrdl` drops the line's trailing white space; `tfspfx` compares the prefix
/// case-sensitively and is true only when something but white space follows it, `tfspst`
/// then pointing past the white space after the prefix. Every caller passes a prefix (the
/// third description line), and every stock gang house's is empty, so a file shows up to
/// its first blank line, each line without its leading and trailing blanks (T-107's m4 run:
/// the original drops the trailing blanks of `WCC85715.HSE`). No page mode is carried.
pub fn desc_from_file(world: &World, file: &str, prefix: &str, once: bool) -> Option<Vec<String>> {
    let Some(bytes) = world.display.assets.get(file) else {
        warn_missing_file_once(file);
        return None;
    };
    let mut out = Vec::new();
    let mut found = false;
    for line in bytes.split(|&b| b == b'\n').take(500) {
        let text = cp437::decode(line);
        // tfsrdl: the line without its trailing white space (the CR included); ASCII blanks
        // only, so a CP437 byte such as 0xFF (U+00A0) stays
        let text = text.trim_end_matches(is_blank);
        let rest = text
            .strip_prefix(prefix)
            .map(|r| r.trim_matches(is_blank))
            .filter(|r| !r.is_empty());
        if let Some(rest) = rest {
            found = true;
            out.push(rest.to_owned());
            if once {
                break;
            }
        } else if found {
            break;
        }
    }
    Some(out)
}

/// The white space the text-file scanner trims: space, tab and CR.
fn is_blank(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r')
}

/// The long room description into the viewer's batch.
pub fn long_room_desc(world: &mut World, id: SessionId, room: &Room) {
    let mut msgs = Vec::new();
    if let Some((file, prefix)) = file_description(&room.desc) {
        msgs.push(Msg::Clrline);
        msgs.push(Msg::AnsiNormal);
        if let Some(lines) = desc_from_file(world, file, prefix, false) {
            msgs.extend(lines.into_iter().map(|line| Msg::RoomFileLine { line }));
        }
    } else {
        msgs.push(Msg::RoomLongDesc {
            text: paragraph(&room.desc),
        });
    }
    if room.room_type == RoomType::Shop {
        match world.shops.get(&room.shopnum) {
            Some(shp) => {
                if let Some((file, prefix)) = file_description(&shp.desc) {
                    if let Some(lines) = desc_from_file(world, file, prefix, false) {
                        msgs.extend(lines.into_iter().map(|line| Msg::RoomFileLine { line }));
                    }
                } else if shp.desc.iter().any(|l| !l.is_empty()) {
                    msgs.push(Msg::RoomShopDesc {
                        text: paragraph(&shp.desc),
                    });
                }
                if shp.shop_type == ShopType::Bank {
                    let c = &world.config;
                    msgs.push(Msg::ConversionRates {
                        plat_per_runic: c.plat_per_runic,
                        gold_per_plat: c.gold_per_plat,
                        silv_per_gold: c.silv_per_gold,
                        copper_per_silv: c.copper_per_silv,
                        runic: c.usr_cur_name.clone(),
                    });
                }
            }
            // The room shows as a normal one: the shop text is skipped and the fault
            // logged.
            None => {
                tracing::warn!(room = ?room.key, shop = room.shopnum.0, "shop room without its shop")
            }
        }
    }
    if let Some(s) = world.session_mut(id) {
        for m in msgs {
            s.output.push(m);
        }
    }
}

/// The description lines as one paragraph: each non-empty line depadded and followed by one
/// space.
fn paragraph(lines: &[String]) -> String {
    let mut text = String::new();
    for l in lines.iter().filter(|l| !l.is_empty()) {
        text.push_str(l.trim_end_matches(' '));
        text.push(' ');
    }
    text
}

/// The colour of `row` in `palette` as text, for building a coloured argument.
fn colour(row: ColRow, palette: Palette) -> &'static str {
    std::str::from_utf8(Colour::new(row, palette).prefix()).unwrap_or("")
}

fn ansi_str(a: &'static [u8]) -> &'static str {
    std::str::from_utf8(a).unwrap_or("")
}

/// The names after `Also here: ` with their colours, `None` when nobody and nothing is
/// here. The players in the game come first, in channel order; then the room's monsters in
/// slot order.
fn also_here(world: &mut World, id: SessionId, key: RoomKey, palette: Palette) -> Option<String> {
    let viewer = world.session(id)?.player()?;
    let see_hidden = viewer.status.contains(PlayerStatus::SEEHIDDEN);
    let viewer_evil = viewer.evil;
    let viewer_name = viewer.firstname.clone();
    let names = colour(ColRow::HereNames, palette);
    let mut who = String::new();
    let mut cnt = 0;
    for other in world.session_ids() {
        if other == id || !world.is_in_game(other) {
            continue;
        }
        let Some(v) = world.session(other).and_then(|s| s.player()) else {
            continue;
        };
        if v.room != key.room || v.map != key.map || (v.hidden != 0 && !see_hidden) {
            continue;
        }
        if cnt == 0 {
            who.push_str(names);
        } else {
            who.push_str(", ");
        }
        who.push_str(&v.firstname);
        // A timer either way or an Outlaw, unless robbed unseen
        if crate::evil::shows_evil_star(world, id, other, v.evil) {
            who.push('*');
        }
        if v.hidden != 0 {
            who.push_str("(Hidden)");
        }
        cnt += 1;
    }
    let Some(state) = world.room_state.get(&key) else {
        return (cnt > 0).then_some(who);
    };
    let slots = state.monsters;
    for (i, &monnum) in slots.iter().enumerate() {
        if monnum.0 == 0 {
            continue;
        }
        let Some(mon) = active_monster(world, monnum) else {
            // Gone; logged and dropped from the room's list
            tracing::warn!(monster = monnum.0, room = ?key, "mongone");
            clear_slot(world, key, i);
            continue;
        };
        if mon.room != key.room || mon.map != key.map {
            // Listed here, recorded elsewhere
            let own = RoomKey {
                map: mon.map,
                room: mon.room,
            };
            let listed_there = world
                .room_state
                .get(&own)
                .is_some_and(|s| s.monsters.contains(&monnum));
            if listed_there {
                tracing::warn!(monster = monnum.0, room = ?key, own = ?own, "monbadroom");
                clear_slot(world, key, i);
            } else if let Some(m) = world
                .active_monsters
                .values_mut()
                .find(|m| m.number == monnum)
            {
                m.room = key.room;
                m.map = key.map;
            }
            continue;
        }
        // The label opens the line when no player did (it is the template's); later names
        // follow a comma in COL_HERENAMES.
        cnt += 1;
        if cnt > 1 {
            who.push_str(names);
            who.push_str(", ");
        }
        who.push_str(monster_colour(mon.alignment, viewer_evil, palette));
        who.push_str(&mon.name);
        if mon.flags.contains(ActiveMonsterFlags::CHARMED)
            && mon.following.eq_ignore_ascii_case(&viewer_name)
        {
            who.push_str(" (Charmed)");
        }
        who.push_str(ansi_str(ansi::ANSI_RESET));
    }
    (cnt > 0).then_some(who)
}

/// The colour of a monster's name for this viewer: good and neutral in cyan; lawful good in
/// grey for a viewer under [`EVIL_OUTLAW`]; lawful evil in grey for a viewer at or over it;
/// everything else in `COL_HERENAMES`.
fn monster_colour(alignment: Alignment, viewer_evil: i16, palette: Palette) -> &'static str {
    let names = colour(ColRow::HereNames, palette);
    let grey = ansi_str(ansi::ANSI_GREY);
    match alignment {
        Alignment::Good | Alignment::Neutral => ansi_str(ansi::ANSI_CYAN),
        Alignment::LawfulGood => {
            if viewer_evil < EVIL_OUTLAW {
                grey
            } else {
                names
            }
        }
        // The lawful evil test falls into the evil cases' branch
        Alignment::LawfulEvil => {
            if viewer_evil < EVIL_OUTLAW {
                names
            } else {
                grey
            }
        }
        Alignment::Evil | Alignment::ChaoticEvil | Alignment::NeutralEvil => names,
        // Any other alignment
        Alignment::Unknown(_) => names,
    }
}

fn active_monster(
    world: &World,
    number: ActiveMonsterId,
) -> Option<silvermere_types::records::ActiveMonster> {
    // Looking the monster up (T-086) brings it into the cache
    world.active_monsters.by_number(number).cloned()
}

fn clear_slot(world: &mut World, key: RoomKey, i: usize) {
    if let Some(s) = world.room_state.get_mut(&key) {
        s.monsters[i] = ActiveMonsterId(0);
    }
}
