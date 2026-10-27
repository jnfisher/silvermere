//! `GameConfig`: the WCCMMUD.MSG / WCCMMUD.INI options that survive (no I/O).
//!
//! Names are the original option names in snake_case. Defaults: the values the distributed
//! `WCCMMUD.MSG` ships for its options (John, 2026-10-14), the original's defaults for the
//! `WCCMMUD.INI` options, and `inactive_secs = 3600`, `characters_per_account`,
//! `open_signup` as John decided (`docs/config-defaults.md`).
//! Ranges are the `numopt` ranges the original declares for its options.
//! `docs/config-defaults.md` documents every option.

use serde::{Deserialize, Serialize};

/// `PUNHUP`: punishment for hanging up (a `tokopt` word option).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PunHup {
    /// `PUN_HIGH` (the default; also what an unknown word means).
    High,
    /// `PUN_MEDIUM`.
    Medium,
    /// `PUN_LOW`.
    Low,
    /// `PUN_NONE`.
    None,
}

/// The game options. Every field has a default, so `[game]` may be empty or absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameConfig {
    /// `INACTIVE`: seconds idle before a kick. 60..=32000.
    pub inactive_secs: u32,
    /// `DEATHHP`: hit points at which a player dies. -20000..=0. John: -200.
    pub death_hp: i32,
    /// Seed of the game RNG; absent (the default) seeds from the OS. The differential harness
    /// (T-019) sets it so two runs roll alike.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rng_seed: Option<u64>,
    /// `BLANK_LASTNAME_ALLOWED`.
    pub blank_lastname_allowed: bool,
    /// Characters per account (John: 1). 1..=255.
    pub characters_per_account: u32,
    /// Anyone may create an account with NEW (John: true).
    pub open_signup: bool,
    /// Add-on packs to load; `["all"]`.
    pub addons: Vec<String>,

    /// `SECCREATE`: seconds between monster creation.
    pub sec_create: u32,
    /// `SECOTH`: seconds between player saves. 1..=20000.
    pub sec_oth: u32,
    /// `SECSLOW`: slow cycle seconds.
    pub sec_slow: u32,
    /// `SECMED`: medium cycle seconds.
    pub sec_med: u32,
    /// `SECFAST`: fast cycle seconds.
    pub sec_fast: u32,
    /// `SECENERGY`: energy cycle seconds.
    pub sec_energy: u32,

    /// `STARTROOM` (map 1): where new characters start.
    pub start_room: u32,
    /// `REINCROOM` (map 1): where a dead player is reborn.
    pub reinc_room: u32,
    /// `EVILREINCROOM` (map 1): rebirth room for evil players.
    pub evil_reinc_room: u32,
    /// `JAILROOM` (map 1).
    pub jail_room: u32,

    /// `XTRALVL`: extra levels before training. 0..=16.
    pub xtra_lvl: u32,
    /// `REGENMINS`: minutes before monsters regenerate.
    pub regen_mins: u32,
    /// `GANGEXP`: experience (x1000) for a gang house. 0..=32767.
    pub gang_exp: u32,
    /// `TALKCHAR`: line prefix for talking.
    pub talk_char: char,
    /// `YELLCHAR`: line prefix for yelling.
    pub yell_char: char,
    /// `SMASHROUNDS`.
    pub smash_rounds: u32,
    /// `SMASHPENALTY`.
    pub smash_penalty: u32,
    /// `DEFCHATRATE`: pages, gossips and auctions allowed per slow cycle.
    pub def_chat_rate: u32,
    /// `MAXTOP`: entries in the top lists. 10..=3000.
    pub max_top: u32,
    /// `INVGLO1`..`INVGLO10`: disallowed globals (at most 10).
    pub inv_glo: Vec<String>,
    /// `PUNHUP`.
    pub pun_hup: PunHup,
    /// `EVILKEEP`: percentage of evil points kept. 0..=100.
    pub evil_keep: u32,
    /// `LVLDIFF`: largest level gap for PvP. -1..=100.
    pub lvl_diff: i32,
    /// `MINHUPHP`: least percentage lost on hangup. 0..=100.
    pub min_hup_hp: u32,
    /// `MAXHUPHP`: most percentage lost on hangup. 0..=100.
    pub max_hup_hp: u32,
    /// `MAXHUPIT`: most items lost on hangup. 0..=100.
    pub max_hup_items: u32,
    /// `KEEPEXP`.
    pub keep_exp: bool,
    /// `LIGHTEVY`: SYSOP LIGHTNING may hit any player.
    pub light_evy: bool,
    /// `GENDERGO`: gold to change gender. -1..=32000.
    pub gender_gold: i32,
    /// `ENABLAWF`: players may choose Lawful.
    pub enable_lawful: bool,
    /// `EPCYCLE`: minutes per evil-point forgiveness cycle. 15..=360.
    pub ep_cycle: u32,
    /// `EPAMOUNT`: evil points forgiven per cycle. 1..=100.
    pub ep_amount: u32,
    /// `MAXEPDAY`: most evil points forgiven a day. 4..=100.
    pub max_ep_day: u32,

    /// `USRCURNM`: name of the top currency, at most 28 characters.
    pub usr_cur_name: String,
    /// Platinum pieces per top coin.
    pub plat_per_runic: u32,
    /// Gold per platinum.
    pub gold_per_plat: u32,
    /// Silver per gold.
    pub silv_per_gold: u32,
    /// Copper per silver.
    pub copper_per_silv: u32,

    /// `TRACK_LIMITED`.
    pub track_limited: bool,
    /// `EXP_IN_TOPGANG`.
    pub exp_in_top_gang: bool,
    /// `LOG_SYSOP_COMMANDS` (default on).
    pub log_sysop_commands: bool,
    /// `RACE_CHANGE_DURATION`, minutes. 1..=720.
    pub race_change_duration: u32,
    /// `ENABLE_CLASS_CHANGE`.
    pub enable_class_change: bool,
    /// `TOURNAMENT_PLAY`.
    pub tournament_play: bool,
    /// `MAXIMUM_BANK_BALANCE` in copper; 0 means no limit.
    pub maximum_bank_balance: u64,
    /// `KID_GLOVES`: refund wrong payments.
    pub kid_gloves: bool,
    /// `SYS_LOG_DEATHS`: log player deaths, rerolls and suicides.
    pub sys_log_deaths: bool,
    /// `PROFCHEK`: filter profanity in say/yell/gossip-class lines.
    pub profanity_check: bool,
    /// `EXPINTEN`: show experience in the TOPTEN list.
    pub exp_in_topten: bool,
    /// `CODEC_OF_TWINKS`: developer code; 78 unlocks the sysop debug tools.
    pub codec_of_twinks: i32,
    /// `[game.cleanup]`: the daily cleanup (T-056).
    pub cleanup: CleanupConfig,
    /// `[game.buffers]`: the record cache sizes silvermere models (T-040).
    pub buffers: BuffersConfig,
}

/// `[game.buffers]`: the sizes of the original's record caches that silvermere models. Only
/// the active-monster cache is: the background passes walk it (T-086), so its size decides
/// which monsters move, regenerate and refill their energy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuffersConfig {
    /// `MONSBUF`: the active-monster cache holds `monsbuf + 1` slots (the `WCCMMUD.MSG` option
    /// `{Number of active monsters to buffer? 300}`, 10..=32767). At startup the original
    /// lowers it by 10 while it is above 25 and the cache does not fit in free memory (from
    /// 300: 20): under MBBSEmu it ends at 20 (21 slots), and the differential sets that value
    /// here (T-040).
    pub monsbuf: u32,
}

impl Default for BuffersConfig {
    fn default() -> Self {
        Self { monsbuf: 300 }
    }
}

/// `[game.cleanup]`: when the daily cleanup runs. The original ran it in the BBS's nightly
/// cleanup, which took the system down; silvermere runs it in the process, with the players
/// connected (T-056, John 2026-10-14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CleanupConfig {
    /// Local wall-clock time of the daily run, `"HH:MM"` (24-hour). Default `"03:00"`.
    pub at: String,
    /// IANA time zone `at` is read in. Default `"America/Detroit"` (John, 2026-10-15); a
    /// silvermere addition, the original used the host's clock.
    pub tz: String,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            at: "03:00".to_owned(),
            tz: "America/Detroit".to_owned(),
        }
    }
}

impl CleanupConfig {
    /// `at` as `(hour, minute)`, `None` when it is not a valid `"HH:MM"`.
    pub fn hour_minute(&self) -> Option<(u8, u8)> {
        hour_minute(&self.at)
    }
}

/// A `"HH:MM"` (24-hour) time of day as `(hour, minute)`, `None` when it is not one. The
/// daily cleanup's `at` and the scheduled backup's `backup.at` are read with it.
pub fn hour_minute(at: &str) -> Option<(u8, u8)> {
    let (h, m) = at.split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let h: u8 = h.parse().ok()?;
    let m: u8 = m.parse().ok()?;
    (h < 24 && m < 60).then_some((h, m))
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            inactive_secs: 3600,
            death_hp: -200,
            rng_seed: None,
            blank_lastname_allowed: false,
            characters_per_account: 1,
            open_signup: true,
            addons: vec!["all".to_owned()],
            sec_create: 5,
            sec_oth: 160,
            sec_slow: 30,
            sec_med: 3,
            sec_fast: 1,
            sec_energy: 5,
            start_room: 2140,
            reinc_room: 2189,
            evil_reinc_room: 142,
            jail_room: 376,
            xtra_lvl: 2,
            regen_mins: 5,
            gang_exp: 0,
            talk_char: '.',
            yell_char: '"',
            smash_rounds: 15,
            smash_penalty: 35,
            def_chat_rate: 1,
            max_top: 10,
            inv_glo: vec![";x".to_owned(), ";-".to_owned(), ";re".to_owned()],
            pun_hup: PunHup::None,
            evil_keep: 0,
            lvl_diff: 9,
            min_hup_hp: 0,
            max_hup_hp: 0,
            max_hup_items: 0,
            keep_exp: false,
            light_evy: true,
            gender_gold: 0,
            enable_lawful: true,
            ep_cycle: 60,
            ep_amount: 1,
            max_ep_day: 12,
            usr_cur_name: "runic coins".to_owned(),
            plat_per_runic: 100,
            gold_per_plat: 100,
            silv_per_gold: 10,
            copper_per_silv: 10,
            track_limited: false,
            exp_in_top_gang: false,
            log_sysop_commands: true,
            race_change_duration: 300,
            enable_class_change: false,
            tournament_play: false,
            maximum_bank_balance: 0,
            kid_gloves: false,
            sys_log_deaths: false,
            profanity_check: false,
            exp_in_topten: true,
            codec_of_twinks: 0,
            cleanup: CleanupConfig::default(),
            buffers: BuffersConfig::default(),
        }
    }
}

impl GameConfig {
    /// Checks every range; the error names the key and its range (the original
    /// refused out-of-range values).
    pub fn validate(&self) -> Result<(), String> {
        fn chk(key: &str, v: i64, lo: i64, hi: i64) -> Result<(), String> {
            if (lo..=hi).contains(&v) {
                Ok(())
            } else {
                Err(format!(
                    "game.{key} = {v} is out of range; allowed {lo}..={hi}"
                ))
            }
        }
        chk("inactive_secs", self.inactive_secs.into(), 60, 32000)?;
        chk("death_hp", self.death_hp.into(), -20000, 0)?;
        chk(
            "characters_per_account",
            self.characters_per_account.into(),
            1,
            255,
        )?;
        chk("sec_oth", self.sec_oth.into(), 1, 20000)?;
        chk("xtra_lvl", self.xtra_lvl.into(), 0, 16)?;
        chk("gang_exp", self.gang_exp.into(), 0, 32767)?;
        chk("max_top", self.max_top.into(), 10, 3000)?;
        chk("evil_keep", self.evil_keep.into(), 0, 100)?;
        chk("lvl_diff", self.lvl_diff.into(), -1, 100)?;
        chk("min_hup_hp", self.min_hup_hp.into(), 0, 100)?;
        chk("max_hup_hp", self.max_hup_hp.into(), 0, 100)?;
        chk("max_hup_items", self.max_hup_items.into(), 0, 100)?;
        chk("gender_gold", self.gender_gold.into(), -1, 32000)?;
        chk("ep_cycle", self.ep_cycle.into(), 15, 360)?;
        chk("ep_amount", self.ep_amount.into(), 1, 100)?;
        chk("max_ep_day", self.max_ep_day.into(), 4, 100)?;
        chk(
            "race_change_duration",
            self.race_change_duration.into(),
            1,
            720,
        )?;
        chk("inv_glo (entries)", self.inv_glo.len() as i64, 0, 10)?;
        chk("buffers.monsbuf", self.buffers.monsbuf.into(), 10, 32767)?;
        if self.cleanup.hour_minute().is_none() {
            return Err(format!(
                "game.cleanup.at = {:?} is not a time; allowed \"HH:MM\", 00:00..=23:59",
                self.cleanup.at
            ));
        }
        if let Err(e) = jiff::tz::TimeZone::get(&self.cleanup.tz) {
            return Err(format!(
                "game.cleanup.tz = {:?} is not a known IANA time zone ({e})",
                self.cleanup.tz
            ));
        }
        let n = self.usr_cur_name.chars().count();
        if n == 0 || n > 28 {
            return Err(format!(
                "game.usr_cur_name has {n} characters; allowed 1..=28"
            ));
        }
        Ok(())
    }
}
