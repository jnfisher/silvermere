//! Every stock spell through the rules: each of the 1,379 spell records of the downloaded
//! distribution converts to its effects with no unknown ability, rolls inside its range
//! and casts on a player without a panic.

use crate::stock_records;

use std::collections::BTreeMap;

use serde_json::{Value, json};
use silvermere_rules::SeededRng;
use silvermere_rules::spells::apply::{
    Cast, CastPath, CastState, CasterStats, Target, UserTarget, apply_spell,
};
use silvermere_rules::spells::{SpellEffects, amount_range, roll_amount};
use silvermere_types::enums::{
    Ability, Element, Magery, MessageLevel, ResistMode, ResistStat, SpellTarget, SpellType,
    StatlineMode,
};
use silvermere_types::flags::{
    PlayerFlags, PlayerSettings, PlayerStatus, PlayerStatus2, SpellFlags,
};
use silvermere_types::ids::{ItemNo, SpellNo};
use silvermere_types::records::{Character, Coins, Item, Spell};
use silvermere_types::{NABILITY, NRECABILS, NSPELLCAST, NTRAIL, NWORN};

const NONE: (Ability, i16) = (Ability::None, 0);

/// The stock spell records, one object each: the record's members as `--json` writes them,
/// with `abilities` as `[number, value]` pairs in slot order.
fn stock_spell_rows() -> Vec<Value> {
    stock_records::table("WCCSPELS")
        .iter()
        .map(|r| {
            let mut row = r.clone();
            let pairs: Vec<Value> = stock_records::ints(r, "abilities")
                .into_iter()
                .zip(stock_records::ints(r, "abilvals"))
                .map(|(a, v)| json!([a, v]))
                .collect();
            row.insert("abilities".into(), Value::from(pairs));
            Value::Object(row)
        })
        .collect()
}

fn slots<const N: usize>(given: &[(Ability, i16)]) -> [(Ability, i16); N] {
    let mut out = [NONE; N];
    out[..given.len()].copy_from_slice(given);
    out
}

fn character() -> Character {
    Character {
        userid: String::new(),
        firstname: String::new(),
        lastname: String::new(),
        exp: 0,
        spells: [SpellNo(0); NSPELLCAST],
        spellvals: [0; NSPELLCAST],
        spellrnds: [0; NSPELLCAST],
        title: String::new(),
        race: 0.into(),
        class: 0.into(),
        level: 0,
        basint: 0,
        baswil: 0,
        basstr: 0,
        bashea: 0,
        basagi: 0,
        bascha: 0,
        intel: 0,
        wil: 0,
        str: 0,
        hea: 0,
        agi: 0,
        cha: 0,
        maxhp: 0,
        hp: 0,
        maxwgt: 0,
        wgt: 0,
        maxenergy: 0,
        energy: 0,
        poison: 0,
        maxmr: 0,
        mr: 0,
        map: 0.into(),
        room: 0.into(),
        light: 0,
        hunger: 0,
        thirst: 0,
        ninvslots: 0,
        items: Vec::new(),
        itemuses: Vec::new(),
        nkeyslots: 0,
        keys: Vec::new(),
        keyuses: Vec::new(),
        qflags2: 0,
        hpbonus: 0,
        manabonus: 0,
        expbil: 0,
        exprem: 0,
        nspellslots: 0,
        spellbook: Vec::new(),
        evil: 0,
        lawful: 0,
        trailmaps: [0.into(); NTRAIL],
        trailrooms: [0.into(); NTRAIL],
        verbose: 0,
        bchannel: 0,
        fasttalk: 0,
        statline: StatlineMode::from_code(0),
        hidden: 0,
        perception: 0,
        stealth: 0,
        martial: 0,
        thievery: 0,
        maxmana: 0,
        mana: 0,
        spellcast: 0,
        traps: 0,
        disarm: 0,
        picklocks: 0,
        tracking: 0,
        coins: Coins::default(),
        weapon: ItemNo(0),
        worn: [ItemNo(0); NWORN],
        wornuses: [0; NWORN],
        autoengage: 0,
        lives: 0,
        aided: 0,
        rank: 0,
        lighttime: 0,
        readied: 0,
        delay: 0,
        roomlight: 0,
        gang: String::new(),
        hairlen: 0,
        haircol: 0,
        eyecol: 0,
        paging: 0,
        cp: 0,
        suicpw: String::new(),
        attackers: 0,
        status: PlayerStatus::empty(),
        movedir: 0,
        dragging: 0,
        totalcp: 0,
        immortal: 0,
        pagecnt: 0,
        pagemax: 0,
        settings: PlayerSettings::empty(),
        gossmax: 0,
        auctmax: 0,
        gosscnt: 0,
        auctcnt: 0,
        encumpct: 0,
        avbonus: 0,
        acbonus: 0,
        dmgbonus: 0,
        mabase: 0,
        arenahp: 0,
        qflags: 0,
        version: 0,
        msglevel: MessageLevel::from_code(0),
        pvpcnt: 0,
        randomhp: 0,
        abils: [NONE; NABILITY],
        resttick: 0,
        critbonus: 0,
        drbonus: 0,
        drpct: 0,
        energyuse: 0,
        shield: 0,
        defmod: 0,
        sesstime: 0,
        lifemins: 0,
        moves: 0,
        lastdate: 0,
        status2: PlayerStatus2::empty(),
        palette: 0,
        deathdate: 0,
        flags: PlayerFlags::empty(),
        sex: 0,
        healrate: 0,
        manarate: 0,
        racechg: 0,
        lastmon: 0,
        lastmoncnt: 0,
        pvptimer: 0,
        medtick: 0,
    }
}

fn spell(number: i16, abilities: &[(Ability, i16)]) -> Spell {
    Spell {
        number: SpellNo(number),
        name: String::new(),
        descs: [String::new(), String::new()],
        resist: 0,
        levelcap: 0,
        flags: SpellFlags::empty(),
        abilities: slots(abilities),
        energy: 0,
        level: 0,
        minval: 0,
        maxval: 0,
        spell_type: SpellType::from_code(0),
        resmode: ResistMode::from_code(0),
        diff: 0,
        durlvl: 0,
        target: SpellTarget::from_code(0),
        duration: 0,
        element: Element::from_code(0),
        resstat: ResistStat::from_code(0),
        magery: Magery::from_code(0),
        msgnum: 0.into(),
        mana: 0,
        maxinc: 0,
        maxlvls: 0,
        magelvl: 0,
        mininc: 0,
        minlvls: 0,
        durinc: 0,
        durlvls: 0,
        shortnm: String::new(),
    }
}

fn caster_at<'a>(level: u16) -> CasterStats<'a> {
    CasterStats {
        firstname: "Thornwyn",
        level,
        spelldamage_pct: 0,
        spellduration_pct: 0,
        spellcast: 50,
    }
}

fn no_spells(_: SpellNo) -> Option<&'static Spell> {
    None
}

fn no_items(_: ItemNo) -> Option<&'static Item> {
    None
}

fn user<'a>(character: &'a mut Character) -> UserTarget<'a, 'static> {
    UserTarget {
        character,
        is_caster: false,
        anti_magic: false,
        poison_immune: false,
        element_resist_pct: 0,
        deathhp: -200,
        arena_safe: false,
        spells: &no_spells,
        items: &no_items,
    }
}

fn player(hp: i16, maxhp: i16) -> Character {
    let mut c = character();
    c.hp = hp;
    c.maxhp = maxhp;
    c.maxmana = 40;
    c.mana = 10;
    c.maxenergy = 1000;
    c.energy = 500;
    c.level = 10;
    c
}

fn single<'a>(spell: &'a Spell, caster: &'a CasterStats<'a>, power: i32) -> Cast<'a> {
    Cast {
        spell,
        caster: Some(caster),
        path: CastPath::Single,
        power,
    }
}

/// A stock spell from one of [`stock_spell_rows`].
fn spell_from_json(v: &serde_json::Value) -> Spell {
    let i = |k: &str| v[k].as_i64().unwrap_or_else(|| panic!("{k}"));
    let mut abilities = [NONE; NRECABILS];
    for (slot, pair) in v["abilities"].as_array().unwrap().iter().enumerate() {
        let a = pair[0].as_u64().unwrap() as u16;
        let val = pair[1].as_i64().unwrap() as i16;
        abilities[slot] = (Ability::from_code(a), val);
    }
    let mut s = spell(i("number") as i16, &[]);
    s.abilities = abilities;
    s.name = v["name"].as_str().unwrap().to_owned();
    s.levelcap = i("levelcap") as i16;
    s.energy = i("energy") as u16;
    s.level = i("level") as u16;
    s.minval = i("minval") as i16;
    s.maxval = i("maxval") as i16;
    s.spell_type = SpellType::from_code(i("type") as u16);
    s.resmode = ResistMode::from_code(i("resmode") as u16);
    s.diff = i("diff") as i16;
    s.durlvl = i("durlvl") as i16;
    s.target = SpellTarget::from_code(i("target") as u16);
    s.duration = i("duration") as i16;
    s.element = Element::from_code(i("element") as u16);
    s.resstat = ResistStat::from_code(i("resstat") as u16);
    s.magery = Magery::from_code(i("magery") as u16);
    s.mana = i("mana") as i16;
    s.maxinc = i("maxinc") as u8;
    s.maxlvls = i("maxlvls") as u8;
    s.magelvl = i("magelvl") as i16;
    s.mininc = i("mininc") as u8;
    s.minlvls = i("minlvls") as u8;
    s.durinc = i("durinc") as u8;
    s.durlvls = i("durlvls") as u8;
    s
}

fn stock_spells() -> Vec<Spell> {
    stock_spell_rows().iter().map(spell_from_json).collect()
}

// Every spell in the stock data converts without an Unknown variant; the histogram of
// variants is printed.
#[test]
fn every_stock_spell_converts() {
    let spells = stock_spells();
    assert_eq!(spells.len(), 1379);
    let mut histogram: BTreeMap<&'static str, usize> = BTreeMap::new();
    for s in &spells {
        let e = SpellEffects::from_record(s);
        assert!(
            !e.has_unknown(),
            "spell {} {:?} has an unknown ability",
            s.number.0,
            s.name
        );
        for slot in e.iter() {
            *histogram.entry(slot.effect.kind_name()).or_default() += 1;
        }
    }
    println!("SpellEffect histogram over {} stock spells:", spells.len());
    for (name, n) in &histogram {
        println!("  {name:<24} {n}");
    }
    assert!(histogram["Damage{armour}"] > 100);
    assert!(histogram["Marker"] > 1000);
    assert!(histogram.contains_key("Lasting"));
    // Every stock spell rolls inside its range at every level and never panics on a cast.
    let caster = caster_at(30);
    let mut rng = SeededRng::new(7);
    for s in &spells {
        let range = amount_range(s, 30);
        let r = roll_amount(s, 30, &mut rng);
        assert!(r >= range.min && r <= range.max, "{} {range:?} {r}", s.name);
        let mut c = player(20, 30);
        let mut t = Target::User(user(&mut c));
        let mut state = CastState::new();
        apply_spell(
            &SpellEffects::from_record(s),
            &single(s, &caster, r),
            &mut t,
            &mut state,
            &mut rng,
        );
    }
}
