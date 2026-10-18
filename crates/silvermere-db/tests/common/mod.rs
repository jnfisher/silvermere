//! Shared by the integration tests: the test database and fully populated records.
//!
//! Every field of every record gets a distinct value from a running counter, so a column
//! wired to the wrong field, or a slot dropped on the way through a child table, fails
//! the `==` after the round trip.

#![allow(dead_code)]

use jiff::Timestamp;
use silvermere_db::repo::Account;
use silvermere_db::{Db, TestDb};
use silvermere_types::enums::*;
use silvermere_types::flags::*;
use silvermere_types::ids::*;
use silvermere_types::records::*;
use silvermere_types::sizes::*;

/// The backend `SILVERMERE_DATABASE_URL` names, migrated and empty.
pub async fn db() -> TestDb {
    // Worker logs (including the kind and code of a dropped write) go to the test output,
    // which libtest shows for a failing test.
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("silvermere_db=debug")
        .try_init();
    TestDb::from_env().await.expect("test database")
}

/// Distinct values for every field.
pub struct Seq(pub u32);

impl Seq {
    pub fn new(seed: u32) -> Self {
        Self(seed * 1000)
    }
    pub fn next(&mut self) -> u32 {
        self.0 += 1;
        self.0
    }
    pub fn u8(&mut self) -> u8 {
        (self.next() % 200) as u8 + 1
    }
    pub fn i8(&mut self) -> i8 {
        (self.next() % 100) as i8 - 50
    }
    pub fn u16(&mut self) -> u16 {
        (self.next() % 60000) as u16 + 1
    }
    pub fn i16(&mut self) -> i16 {
        ((self.next() % 60000) as i32 - 30000) as i16
    }
    pub fn u32(&mut self) -> u32 {
        self.next().wrapping_mul(2_654_435_761) | 1
    }
    /// Unix seconds past `i32::MAX` (2038), so a 32-bit column would not hold them.
    pub fn secs(&mut self) -> i64 {
        4_000_000_000 + i64::from(self.next())
    }
    pub fn i32(&mut self) -> i32 {
        self.next().wrapping_mul(2_654_435_761) as i32
    }
    pub fn nz_i32(&mut self) -> i32 {
        (self.next() % 2_000_000) as i32 + 1
    }
    pub fn nz_u32(&mut self) -> u32 {
        self.next() % 2_000_000 + 1
    }
    pub fn nz_i16(&mut self) -> i16 {
        (self.next() % 30000) as i16 + 1
    }
    pub fn text(&mut self, what: &str) -> String {
        format!("{what}-{}", self.next())
    }
    pub fn coins(&mut self) -> Coins {
        Coins {
            copper: self.u32(),
            silver: self.u32(),
            gold: self.u32(),
            platinum: self.u32(),
            runic: self.u32(),
        }
    }
    pub fn pick<T: Copy>(&mut self, all: &[T]) -> T {
        all[(self.next() as usize) % all.len()]
    }
    pub fn ability(&mut self) -> AbilitySlot {
        let n = self.next() as usize;
        (Ability::ALL[1 + n % (Ability::ALL.len() - 1)], self.i16())
    }
    pub fn abilities<const N: usize>(&mut self) -> [AbilitySlot; N] {
        std::array::from_fn(|_| self.ability())
    }
    pub fn texts<const N: usize>(&mut self, what: &str) -> [String; N] {
        std::array::from_fn(|_| self.text(what))
    }
}

pub fn room(seq: &mut Seq, map: u32, number: u32) -> Room {
    let mut s = Seq::new(seq.next());
    Room {
        key: RoomKey::new(map, number),
        name: s.text("room"),
        desc: s.texts("desc"),
        mapfile: s.text("map"),
        exits: std::array::from_fn(|_| Exit {
            target: RoomId(s.nz_u32()),
            kind: s.pick(ExitType::ALL),
            par1: s.u32(),
            par2: s.i16(),
            par3: s.i32(),
            par4: s.i32(),
        }),
        monsters: std::array::from_fn(|_| ActiveMonsterId(s.nz_i32())),
        room_type: s.pick(RoomType::ALL),
        shopnum: ShopNo(s.nz_i32()),
        owner: s.text("owner"),
        minlevel: s.i16(),
        maxlevel: s.i16(),
        genmonster: MonsterNo(s.nz_i32()),
        light: s.i16(),
        ganghouse: s.pick(GangHouse::ALL),
        items: std::array::from_fn(|_| RoomItem {
            item: ItemNo(s.nz_i32()),
            uses: s.i16(),
            count: s.i16(),
        }),
        hiditems: std::array::from_fn(|_| RoomItem {
            item: ItemNo(s.nz_i32()),
            uses: s.i16(),
            count: s.i16(),
        }),
        coins: s.coins(),
        hidcoins: s.coins(),
        maxmonsters: s.u8(),
        mongroup: s.i16(),
        lastkill: s.u16(),
        flags: RoomFlags::from_bits_retain(s.u32()),
        foundmask: s.u32(),
        deathroom: RoomId(s.nz_u32()),
        huproom: RoomId(s.nz_u32()),
        cmdtext: TextBlockNo(s.nz_i32()),
        regendelay: s.i16(),
        areamax: s.i16(),
        areacount: s.i16(),
        ctlroom: RoomId(s.nz_u32()),
        specmon: MonsterNo(s.nz_i32()),
        placed: std::array::from_fn(|_| ItemNo(s.nz_i32())),
        protmsg: TextBlockNo(s.nz_i32()),
        picktrap: s.i16(),
        roomspell: s.i32(),
        searchers: s.u8(),
        nummonsters: s.u8(),
    }
}

pub fn item(seq: &mut Seq, number: i32) -> Item {
    let mut s = Seq::new(seq.next());
    Item {
        number: ItemNo(number),
        level: s.u16(),
        limit: s.u16(),
        name: s.text("item"),
        desc: s.texts("desc"),
        hiddesc: s.texts("hid"),
        active: s.u16(),
        weight: s.i16(),
        item_type: s.pick(ItemType::ALL),
        abilities: s.abilities(),
        charges: s.i16(),
        energy: s.i16(),
        cost: s.u16(),
        classes: std::array::from_fn(|_| ClassNo(s.u16())),
        mindmg: s.i16(),
        maxdmg: s.i16(),
        ac: s.i16(),
        races: std::array::from_fn(|_| RaceNo(s.u16())),
        negates: std::array::from_fn(|_| SpellNo(s.nz_i16())),
        wpntype: s.pick(WeaponType::ALL),
        armtype: s.pick(ArmourType::ALL),
        worn: s.pick(WornLocation::ALL),
        accuracy: s.i16(),
        dr: s.i16(),
        gettable: s.i16(),
        strreq: s.i16(),
        cash: s.coins(),
        speed: s.i16(),
        hitmsg: MessageNo(s.i32()),
        missmsg: MessageNo(s.i32()),
        textnum: TextBlockNo(s.i32()),
        destroymsg: MessageNo(s.i32()),
        nodrop: s.u8(),
        currency: s.pick(Currency::ALL),
        retain: s.u8(),
        robable: s.u8(),
        destroy: s.u8(),
    }
}

pub fn monster(seq: &mut Seq, number: i32) -> KnownMonster {
    let mut s = Seq::new(seq.next());
    KnownMonster {
        number: MonsterNo(number),
        name: s.text("monster"),
        montype: s.pick(MonsterType::ALL),
        level: s.u32(),
        genindex: s.i16(),
        wornitem: ItemNo(s.i32()),
        weapon: ItemNo(s.i32()),
        dr: s.i16(),
        ac: s.i16(),
        group: s.i16(),
        followpct: s.i16(),
        magicres: s.i16(),
        bsdefense: s.u8(),
        exp: s.i32(),
        maxhp: s.i16(),
        energy: s.u16(),
        hpregen: s.i16(),
        abilities: s.abilities(),
        maxalive: s.u16(),
        curalive: s.u16(),
        grouprole: s.pick(GroupRole::ALL),
        maxfollow: s.u8(),
        undead: s.u8(),
        alignment: s.pick(Alignment::ALL),
        regenhrs: s.u8(),
        killed_at: s.secs(),
        arrivemsg: MessageNo(s.i32()),
        deathmsg: MessageNo(s.i32()),
        items: std::array::from_fn(|_| MonsterItem {
            item: ItemNo(s.nz_i32()),
            uses: s.i16(),
            pct: s.u8(),
        }),
        cash: s.coins(),
        talktext: TextBlockNo(s.i32()),
        charmlvl: s.u16(),
        namelist: s.i32(),
        attacks: std::array::from_fn(|_| MonsterAttack {
            kind: s.pick(MonsterAttackType::ALL),
            accuracy: s.i16(),
            pct: s.u8(),
            min: s.i16(),
            max: s.i16(),
            hitmsg: MessageNo(s.i32()),
            defmsg: MessageNo(s.i32()),
            missmsg: MessageNo(s.i32()),
            energy: s.i16(),
            hitspell: SpellNo(s.i16()),
        }),
        randtext: TextBlockNo(s.i32()),
        charmres: s.i16(),
        deathspell: SpellNo(s.i16()),
        createspell: SpellNo(s.i16()),
        spells: std::array::from_fn(|_| MonsterSpell {
            spell: SpellNo(s.nz_i16()),
            pct: s.u8(),
            level: s.u8(),
        }),
        desc: s.texts("desc"),
        gender: s.pick(Gender::ALL),
    }
}

pub fn active_monster(seq: &mut Seq, number: i32) -> ActiveMonster {
    let mut s = Seq::new(seq.next());
    ActiveMonster {
        number: ActiveMonsterId(number),
        knownnum: MonsterNo(s.i32()),
        level: s.u32(),
        map: MapId(s.u32()),
        room: RoomId(s.u32()),
        poison: s.i16(),
        energy: s.u16(),
        hp: s.i16(),
        following: s.text("following"),
        trail: std::array::from_fn(|_| RoomId(s.nz_u32())),
        hunters: std::array::from_fn(|_| ActiveMonsterId(s.nz_i32())),
        hunting: ActiveMonsterId(s.i32()),
        name: s.text("actmon"),
        wornitem: ItemNo(s.i32()),
        weapon: ItemNo(s.i32()),
        items: std::array::from_fn(|_| (ItemNo(s.nz_i32()), s.i16())),
        cash: s.coins(),
        maxhp: s.i16(),
        alignment: s.pick(Alignment::ALL),
        followpct: s.i16(),
        dr: s.i16(),
        ac: s.i16(),
        exp: s.i32(),
        maxenergy: s.i16(),
        friendly: s.u8(),
        genroom: RoomId(s.u32()),
        inactive: s.u8(),
        flags: ActiveMonsterFlags::from_bits_retain(s.u16()),
        montype: s.pick(MonsterType::ALL),
        target: s.i16(),
        hpregen: s.i16(),
        lastdir: s.i16(),
        lastupd: s.i32(),
        grouprole: s.pick(GroupRole::ALL),
        spells: std::array::from_fn(|_| ActiveSpell {
            spell: SpellNo(s.nz_i16()),
            value: s.i16(),
            rounds: s.i16(),
        }),
        downrounds: s.i16(),
        created: s.secs(),
    }
}

pub fn spell(seq: &mut Seq, number: i16) -> Spell {
    let mut s = Seq::new(seq.next());
    Spell {
        number: SpellNo(number),
        name: s.text("spell"),
        descs: s.texts("desc"),
        resist: s.i32(),
        levelcap: s.i16(),
        flags: SpellFlags::from_bits_retain(s.u8()),
        abilities: s.abilities(),
        energy: s.u16(),
        level: s.u16(),
        minval: s.i16(),
        maxval: s.i16(),
        spell_type: s.pick(SpellType::ALL),
        resmode: s.pick(ResistMode::ALL),
        diff: s.i16(),
        durlvl: s.i16(),
        target: s.pick(SpellTarget::ALL),
        duration: s.i16(),
        element: s.pick(Element::ALL),
        resstat: s.pick(ResistStat::ALL),
        magery: s.pick(Magery::ALL),
        msgnum: MessageNo(s.i32()),
        mana: s.i16(),
        maxinc: s.u8(),
        maxlvls: s.u8(),
        magelvl: s.i16(),
        mininc: s.u8(),
        minlvls: s.u8(),
        durinc: s.u8(),
        durlvls: s.u8(),
        shortnm: s.text("sh"),
    }
}

pub fn class(seq: &mut Seq, number: u16) -> Class {
    let mut s = Seq::new(seq.next());
    Class {
        number: ClassNo(number),
        name: s.text("class"),
        basehp: s.i16(),
        hplevel: s.i16(),
        exp: s.i16(),
        attmult: s.i16(),
        maxenergy: s.i16(),
        restored: s.i16(),
        abilities: s.abilities(),
        magery: s.pick(Magery::ALL),
        magelvl: s.i16(),
        weapon: s.pick(WeaponClass::ALL),
        armour: s.pick(ArmourType::ALL),
        combat: s.i16(),
        titles: TextBlockNo(s.i32()),
    }
}

pub fn race(seq: &mut Seq, number: u16) -> Race {
    let mut s = Seq::new(seq.next());
    Race {
        number: RaceNo(number),
        name: s.text("race"),
        minint: s.i16(),
        minwil: s.i16(),
        minstr: s.i16(),
        minhea: s.i16(),
        minagi: s.i16(),
        mincha: s.i16(),
        hpbonus: s.i16(),
        maxenergy: s.i16(),
        restored: s.i16(),
        abilities: s.abilities(),
        basecp: s.i16(),
        exp: s.i16(),
        maxint: s.i16(),
        maxwil: s.i16(),
        maxstr: s.i16(),
        maxhea: s.i16(),
        maxagi: s.i16(),
        maxcha: s.i16(),
    }
}

pub fn shop(seq: &mut Seq, number: i32) -> Shop {
    let mut s = Seq::new(seq.next());
    Shop {
        number: ShopNo(number),
        name: s.text("shop"),
        desc: s.texts("desc"),
        shop_type: s.pick(ShopType::ALL),
        minlevel: s.u16(),
        maxlevel: s.u16(),
        markup: s.u16(),
        ownable: s.u8(),
        expandable: s.u8(),
        trainclass: ClassNo(s.u16()),
        stock: std::array::from_fn(|_| ShopSlot {
            item: ItemNo(s.nz_i32()),
            maxqty: s.u16(),
            qty: s.u16(),
            regentime: s.u16(),
            regennum: s.i16(),
            regenpct: s.u8(),
        }),
    }
}

pub fn message(seq: &mut Seq, number: i32) -> Message {
    let mut s = Seq::new(seq.next());
    Message {
        number: MessageNo(number),
        msgs: s.texts("msg"),
    }
}

pub fn text_block(seq: &mut Seq, part: i16, number: i32) -> TextBlock {
    let mut s = Seq::new(seq.next());
    TextBlock {
        part,
        number: TextBlockNo(number),
        link: TextBlockNo(s.i32()),
        text: format!(
            "{} ÷ ░ CP437 text {}\r\n",
            s.text("text"),
            "x".repeat(TEXTSIZE - 60)
        ),
    }
}

pub fn action(seq: &mut Seq, name: &str) -> Action {
    let mut s = Seq::new(seq.next());
    Action {
        name: name.to_string(),
        texts: s.texts("act"),
        targets: ActionTargets::from_bits_retain(s.u16()),
    }
}

pub fn gang(seq: &mut Seq, name: &str) -> Gang {
    let mut s = Seq::new(seq.next());
    Gang {
        key: name.to_uppercase(),
        name: name.to_string(),
        exp: s.u32(),
        leader: s.text("leader"),
        created: s.secs(),
        members: s.i16(),
        flags: GangFlags::from_bits_retain(s.u32()),
        overexp: s.u32(),
        overflows: s.i16(),
    }
}

pub fn bank_book(seq: &mut Seq, userid: &str, bank: i32) -> BankBook {
    let mut s = Seq::new(seq.next());
    BankBook {
        userid: userid.to_string(),
        bank: ShopNo(bank),
        balance: s.u32(),
        loan: s.u32(),
    }
}

pub fn item_ownership(seq: &mut Seq, id: i32) -> ItemOwnership {
    let mut s = Seq::new(seq.next());
    ItemOwnership {
        id,
        itemnum: ItemNo(s.nz_i32()),
        owntype: s.pick(OwnerKind::ALL),
        mapnum: MapId(s.u32()),
        roomnum: RoomId(s.u32()),
        userid: s.text("owner"),
        monnum: ActiveMonsterId(s.i32()),
    }
}

pub fn permanent_record(seq: &mut Seq, userid: &str) -> PermanentRecord {
    let mut s = Seq::new(seq.next());
    PermanentRecord {
        userid: userid.to_string(),
        deathdate: s.secs(),
        palette: s.u8(),
        msglevel: s.pick(MessageLevel::ALL),
        settings: PlayerSettings::from_bits_retain(s.u16()),
        pagemax: s.u8(),
        hairlen: s.u8(),
        haircol: s.u8(),
        eyecol: s.u8(),
        statline: s.pick(StatlineMode::ALL),
        fasttalk: s.u8(),
        verbose: s.u8(),
        exp: s.u32(),
        firstname: s.text("first"),
        lastname: s.text("last"),
        flags: PlayerFlags::from_bits_retain(s.u16()),
        statfmt: s.text("statfmt"),
        gossmax: s.u8(),
        auctmax: s.u8(),
        daybyte: s.u8(),
        expbil: s.u32(),
        exprem: s.u32(),
    }
}

pub fn evil_record(seq: &mut Seq, userid: &str) -> EvilRecord {
    let mut s = Seq::new(seq.next());
    EvilRecord {
        userid: userid.to_string(),
        evil: s.i16(),
    }
}

pub fn character(seq: &mut Seq, userid: &str, firstname: &str) -> Character {
    let mut s = Seq::new(seq.next());
    Character {
        userid: userid.to_string(),
        firstname: firstname.to_string(),
        lastname: s.text("last"),
        exp: s.u32(),
        spells: std::array::from_fn(|_| SpellNo(s.nz_i16())),
        spellvals: std::array::from_fn(|_| s.i16()),
        spellrnds: std::array::from_fn(|_| s.i16()),
        title: s.text("title"),
        race: RaceNo(s.u16()),
        class: ClassNo(s.u16()),
        level: s.u16(),
        basint: s.u16(),
        baswil: s.u16(),
        basstr: s.u16(),
        bashea: s.u16(),
        basagi: s.u16(),
        bascha: s.u16(),
        intel: s.u16(),
        wil: s.u16(),
        str: s.u16(),
        hea: s.u16(),
        agi: s.u16(),
        cha: s.u16(),
        maxhp: s.i16(),
        hp: s.i16(),
        maxwgt: s.i16(),
        wgt: s.u16(),
        maxenergy: s.u16(),
        energy: s.u16(),
        poison: s.i16(),
        maxmr: s.u16(),
        mr: s.u16(),
        map: MapId(s.u32()),
        room: RoomId(s.u32()),
        light: s.i16(),
        hunger: s.i16(),
        thirst: s.i16(),
        ninvslots: s.u8(),
        items: (0..NINVITEMS).map(|_| ItemNo(s.nz_i32())).collect(),
        itemuses: (0..NINVITEMS).map(|_| s.i16()).collect(),
        nkeyslots: s.u8(),
        keys: (0..NKEYS).map(|_| ItemNo(s.nz_i32())).collect(),
        keyuses: (0..NKEYS).map(|_| s.i16()).collect(),
        qflags2: s.u32(),
        hpbonus: s.i32(),
        manabonus: s.i32(),
        expbil: s.u32(),
        exprem: s.u32(),
        nspellslots: s.u8(),
        spellbook: (0..NSPELLBOOK).map(|_| SpellNo(s.nz_i16())).collect(),
        evil: s.i16(),
        lawful: s.i8(),
        trailmaps: std::array::from_fn(|_| MapId(s.nz_u32())),
        trailrooms: std::array::from_fn(|_| RoomId(s.nz_u32())),
        verbose: s.u8(),
        bchannel: s.i16(),
        fasttalk: s.u8(),
        statline: s.pick(StatlineMode::ALL),
        hidden: s.u8(),
        perception: s.i16(),
        stealth: s.i16(),
        martial: s.i16(),
        thievery: s.i16(),
        maxmana: s.u16(),
        mana: s.i16(),
        spellcast: s.i16(),
        traps: s.i16(),
        disarm: s.i16(),
        picklocks: s.i16(),
        tracking: s.i16(),
        coins: s.coins(),
        weapon: ItemNo(s.i32()),
        worn: std::array::from_fn(|_| ItemNo(s.nz_i32())),
        wornuses: std::array::from_fn(|_| s.i16()),
        autoengage: s.i16(),
        lives: s.i16(),
        aided: s.u8(),
        rank: s.u8(),
        lighttime: s.i16(),
        readied: s.i8(),
        delay: s.u8(),
        roomlight: s.i16(),
        gang: s.text("gang"),
        hairlen: s.u8(),
        haircol: s.u8(),
        eyecol: s.u8(),
        paging: s.u8(),
        cp: s.i16(),
        suicpw: s.text("pw"),
        attackers: s.i32(),
        status: PlayerStatus::from_bits_retain(s.u16()),
        movedir: s.i16(),
        dragging: 0,
        totalcp: s.u16(),
        immortal: s.u8(),
        pagecnt: s.u8(),
        pagemax: s.u8(),
        settings: PlayerSettings::from_bits_retain(s.u16()),
        gossmax: s.u8(),
        auctmax: s.u8(),
        gosscnt: s.u8(),
        auctcnt: s.u8(),
        encumpct: s.i16(),
        avbonus: s.i16(),
        acbonus: s.i16(),
        dmgbonus: s.i16(),
        mabase: s.u8(),
        arenahp: s.i16(),
        qflags: s.u32(),
        version: 22,
        msglevel: s.pick(MessageLevel::ALL),
        pvpcnt: s.u8(),
        randomhp: s.u8(),
        abils: s.abilities(),
        resttick: s.u8(),
        critbonus: s.i16(),
        drbonus: s.i16(),
        drpct: s.i16(),
        energyuse: s.i16(),
        shield: s.i16(),
        defmod: s.i16(),
        sesstime: s.u32(),
        lifemins: s.u32(),
        moves: s.u8(),
        lastdate: s.secs(),
        status2: PlayerStatus2::from_bits_retain(s.u16()),
        palette: s.u8(),
        deathdate: s.secs(),
        flags: PlayerFlags::from_bits_retain(s.u16()),
        sex: b'F',
        healrate: s.i16(),
        manarate: s.i16(),
        racechg: s.u32(),
        lastmon: s.i32(),
        lastmoncnt: s.u16(),
        pvptimer: s.u8(),
        medtick: s.u8(),
    }
}

/// A fixed instant so tests never read the wall clock.
pub fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(1_760_400_000 + seconds).expect("timestamp")
}

pub fn account(seq: &mut Seq, username: &str) -> Account {
    let mut s = Seq::new(seq.next());
    Account {
        username: username.to_string(),
        password_hash: s.text("$argon2id$hash"),
        email: Some(s.text("mail")),
        created: at(s.next() as i64),
        last_login: Some(at(s.next() as i64)),
        flags: s.u32(),
        roles: vec!["player".to_string(), "sysop".to_string()],
    }
}

/// Creates an account named `username` and returns its id.
pub async fn new_account(db: &Db, seq: &mut Seq, username: &str) -> AccountId {
    db.accounts()
        .insert(&account(seq, username))
        .await
        .expect("insert account")
}
