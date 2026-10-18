//! The command words: `parse_command` as a table.
//!
//! In the original a word is a command as soon as the letters typed so far identify one,
//! case not counting, and no command once a letter does not continue a known word. Every
//! accepted abbreviation of a word is therefore a run of prefixes, from the shortest the
//! original accepts to the full word, and one longer or one wrong letter gives `CMD_NONE`.
//! Each run is one [`CommandWord`]: the longest word of the run and the length of its
//! shortest member. A command whose accepted forms are not one run has one entry per run
//! (`N`/`NORTH`, `Q`/`QUIT`, `PU[NCH]`/`PUR[G]`). The runs never overlap, so table order
//! does not matter; the entries are grouped by first letter for reading.
//!
//! The oracle is `tests/fixtures/command_words.json`: every reachable letter sequence with
//! its code, plus one longer and one wrong-letter sibling; `tests/parse_command.rs` feeds
//! every entry through [`super::parse::parse_command`].
//!
//! Original quirks the table keeps (sic):
//! - `KILLBLOW`: the accepted spelling has three Ls, so `KIL`..`KILLLBLO` are
//!   KILLBLOW and `KILLBLOW` typed in full is no command.
//! - `VERB` and `VERS` with anything after them are VERBOSE and VERSION ([`open`]
//!   entries), as are the punctuation prefixes `'`, `-`, `>`, `/`. `?` must stand alone.
//! - `NORTH` and `QUIT` take only `N`/`Q` or the full word; `QU`, `QUI`, `QUIE` are QUIET.
//!   `PUR` and `PURG` are PUNCH; PURGE only in full.
//! - `BORROW` is the one word no `(word, min)` pair expresses: `BORROW` plus two characters
//!   is still BORROW and plus three is none. `parse::parse_command` has that arm by hand.
//!   (`BORROW` plus one character gives no fixed answer in the original; here it is BORROW,
//!   as the fixture generator reads it.)
//! - Synonyms that are other words' commands: `AA`, `ALLOUT`, `FORCE` are BASH; `AU` is
//!   ATTACK; `BS` BACKSTAB; `BG`, `GB` BROADGANG; `BALANCE` BANKBOOK; `ORDER` BUY; `WHO`
//!   SCAN; `REA` READY but `READ` READ.
//!
//! [`open`]: CommandWord::open

use silvermere_types::enums::CommandCode as Cmd;

/// One run of accepted abbreviations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandWord {
    /// The longest form, upper case.
    pub word: &'static str,
    /// The shortest accepted length: `word[..min]` is the shortest abbreviation.
    pub min: u8,
    pub code: Cmd,
    /// Any text after `word` is accepted too (`VERB`, `VERS`, `'`, `-`, `>`, `/`).
    pub open: bool,
}

const fn w(word: &'static str, min: u8, code: Cmd) -> CommandWord {
    CommandWord {
        word,
        min,
        code,
        open: false,
    }
}

const fn open(word: &'static str, min: u8, code: Cmd) -> CommandWord {
    CommandWord {
        word,
        min,
        code,
        open: true,
    }
}

/// The table. Each entry is one run of prefixes; see the module doc.
pub const TABLE: &[CommandWord] = &[
    // leading characters
    open("'", 1, Cmd::Broadcast),
    open("-", 1, Cmd::Broadcast),
    w("?", 1, Cmd::Help),
    open(">", 1, Cmd::DirectMsg),
    open("/", 1, Cmd::Whisper),
    // A
    w("ATTACK", 1, Cmd::Attack),
    w("AU", 2, Cmd::Attack),
    w("AA", 2, Cmd::Bash),
    w("ALLOUT", 2, Cmd::Bash),
    w("ACTION", 2, Cmd::Action),
    w("AID", 2, Cmd::Aid),
    w("APPRAISE", 2, Cmd::Appraise),
    w("ARM", 2, Cmd::Arm),
    w("ASK", 2, Cmd::Ask),
    w("AUCTION", 3, Cmd::Auction),
    w("AUTOCOMBAT", 3, Cmd::Autocombat),
    // B
    w("BACKRANK", 5, Cmd::BackRank),
    w("BACKSTAB", 5, Cmd::Backstab),
    w("BS", 2, Cmd::Backstab),
    w("BALANCE", 3, Cmd::BankBook),
    w("BANKBOOK", 3, Cmd::BankBook),
    w("BASH", 3, Cmd::Bash),
    w("BG", 2, Cmd::Broadgang),
    w("BORROW", 2, Cmd::Borrow),
    w("BREAK", 3, Cmd::Break),
    w("BRIEF", 3, Cmd::Brief),
    w("BROADCAST", 2, Cmd::Broadcast),
    w("BROADGANG", 6, Cmd::Broadgang),
    w("BUY", 2, Cmd::Buy),
    // C
    w("CAST", 1, Cmd::Cast),
    w("CLOSE", 2, Cmd::Close),
    w("CREATE", 2, Cmd::Create),
    // D
    w("D", 1, Cmd::Down),
    w("DOWN", 3, Cmd::Down),
    w("DEFEND", 3, Cmd::Defend),
    w("DEMOTE", 3, Cmd::Demote),
    w("DEPOSIT", 3, Cmd::Deposit),
    w("DISARM", 2, Cmd::Disarm),
    w("DISBAND", 4, Cmd::Disband),
    w("DISMISS", 4, Cmd::Dismiss),
    w("DONATE", 3, Cmd::Donate),
    w("DRAG", 3, Cmd::Drag),
    w("DRINK", 3, Cmd::Drink),
    w("DROP", 3, Cmd::Drop),
    // E
    w("E", 1, Cmd::East),
    w("EAST", 3, Cmd::East),
    w("EAT", 3, Cmd::Eat),
    w("EQUIP", 2, Cmd::Equip),
    w("EXITS", 3, Cmd::Exits),
    w("EXPERIENCE", 3, Cmd::Experience),
    // F
    w("FEEDBACK", 2, Cmd::Feedback),
    w("FIRE", 2, Cmd::Fire),
    w("FOLLOW", 2, Cmd::Follow),
    w("FORCE", 4, Cmd::Bash),
    w("FORGET", 5, Cmd::Forget),
    w("FORGIVE", 4, Cmd::Forgive),
    w("FRONTRANK", 2, Cmd::FrontRank),
    // G
    w("GB", 2, Cmd::Broadgang),
    w("GET", 1, Cmd::Get),
    w("GIVE", 2, Cmd::Give),
    w("GLOBAL", 3, Cmd::Global),
    w("GOSSIP", 3, Cmd::Gossip),
    w("GREET", 2, Cmd::Greet),
    w("GUARD", 2, Cmd::Guard),
    // H
    w("HEALTH", 2, Cmd::Health),
    w("HELP", 3, Cmd::Help),
    w("HIDE", 3, Cmd::Hide),
    w("HUNT", 2, Cmd::Hunt),
    // I
    w("I", 1, Cmd::Inventory),
    w("INVENTORY", 4, Cmd::Inventory),
    w("IDENTITY", 2, Cmd::Identity),
    w("INVITE", 4, Cmd::Invite),
    w("INVOKE", 4, Cmd::Invoke),
    // J
    w("JOIN", 2, Cmd::Join),
    w("JUMPKICK", 2, Cmd::Jumpkick),
    // K
    w("KEYS", 2, Cmd::Keys),
    w("KICK", 3, Cmd::Kick),
    w("KILLLBLO", 3, Cmd::Killblow),
    // L
    w("LEAVE", 2, Cmd::Leave),
    w("LIGHT", 3, Cmd::Light),
    w("LIST", 3, Cmd::List),
    w("LOCK", 3, Cmd::Lock),
    w("LOOK", 1, Cmd::Look),
    // M
    w("MAP", 2, Cmd::Map),
    w("MARKUP", 3, Cmd::MarkUp),
    w("MEDITATE", 3, Cmd::Meditate),
    w("MESMERIZE", 2, Cmd::Mesmerize),
    w("MIDRANK", 2, Cmd::Midrank),
    // N
    w("N", 1, Cmd::North),
    w("NORTH", 5, Cmd::North),
    w("NE", 2, Cmd::NorthEast),
    w("NORTHEAST", 6, Cmd::NorthEast),
    w("NW", 2, Cmd::NorthWest),
    w("NORTHWEST", 6, Cmd::NorthWest),
    // O
    w("OPEN", 2, Cmd::Open),
    w("ORDER", 3, Cmd::Buy),
    // P
    w("PAGE", 3, Cmd::Page),
    w("PARTY", 3, Cmd::Party),
    w("PICKLOCK", 2, Cmd::PickLock),
    w("PLAY", 2, Cmd::Play),
    w("POWERS", 2, Cmd::Powers),
    w("PROFILE", 2, Cmd::Profile),
    w("PROMOTE", 4, Cmd::Promote),
    w("PUNCH", 2, Cmd::Punch),
    w("PURG", 3, Cmd::Punch),
    w("PURGE", 5, Cmd::Purge),
    // Q
    w("Q", 1, Cmd::Quit),
    w("QUIT", 4, Cmd::Quit),
    w("QUIET", 2, Cmd::Quiet),
    // R
    w("REA", 3, Cmd::Ready),
    w("READ", 4, Cmd::Read),
    w("READY", 5, Cmd::Ready),
    w("REMOVE", 3, Cmd::Remove),
    w("REPAY", 3, Cmd::Repay),
    w("REROLL", 6, Cmd::Reroll),
    w("RESEARCH", 4, Cmd::Research),
    w("REST", 4, Cmd::Rest),
    w("ROB", 2, Cmd::Rob),
    // S
    w("S", 1, Cmd::South),
    w("SOUTH", 5, Cmd::South),
    w("SE", 2, Cmd::SouthEast),
    w("SOUTHEAST", 6, Cmd::SouthEast),
    w("SW", 2, Cmd::SouthWest),
    w("SOUTHWEST", 6, Cmd::SouthWest),
    w("SAVE", 2, Cmd::Save),
    w("SCAN", 2, Cmd::Scan),
    w("SEARCH", 3, Cmd::Search),
    w("SELL", 3, Cmd::Sell),
    w("SET", 3, Cmd::Set),
    w("SHARE", 3, Cmd::Share),
    w("SHOOT", 3, Cmd::Shoot),
    w("SMASH", 3, Cmd::Smash),
    w("SNEAK", 2, Cmd::Sneak),
    w("SOUND", 4, Cmd::Sound),
    w("SPELLS", 2, Cmd::Spells),
    w("STASH", 4, Cmd::Stash),
    w("STATLINE", 5, Cmd::StatLine),
    w("STATUS", 2, Cmd::Status),
    w("STOCK", 3, Cmd::Stock),
    w("SUICIDE", 7, Cmd::Suicide),
    w("SYSOP", 2, Cmd::Sysop),
    // T
    w("TAKE", 3, Cmd::Take),
    w("TALK", 3, Cmd::Talk),
    w("TAME", 3, Cmd::Tame),
    w("TEXT", 2, Cmd::Text),
    w("THROW", 2, Cmd::Throw),
    w("TOPTEN", 2, Cmd::Topten),
    w("TRACK", 4, Cmd::Track),
    w("TRAIN", 4, Cmd::Train),
    // U
    w("UP", 1, Cmd::Up),
    w("UNINVITE", 3, Cmd::Uninvite),
    w("UNREADY", 3, Cmd::Unready),
    w("UNSTOCK", 3, Cmd::Unstock),
    w("USE", 2, Cmd::Use),
    // V
    open("VERB", 4, Cmd::Verbose),
    open("VERS", 4, Cmd::Version),
    // W
    w("W", 1, Cmd::West),
    w("WEST", 4, Cmd::West),
    w("WEALTH", 4, Cmd::Wealth),
    w("WEAR", 3, Cmd::Wear),
    w("WHISPER", 3, Cmd::Whisper),
    w("WHO", 3, Cmd::Scan),
    w("WIELD", 3, Cmd::Wield),
    w("WITHDRAW", 3, Cmd::Withdraw),
];

/// The codes no typed word reaches. Empty: every `CMD_` code but `CMD_NONE` has a word
/// (the fixture's `codes_reachable` lists all 138, `CMD_NONE` included).
pub const CODES_WITHOUT_WORD: &[Cmd] = &[];
