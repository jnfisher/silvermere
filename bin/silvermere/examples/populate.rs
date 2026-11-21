//! `populate`: a population of characters for demos and stress tests (a development tool;
//! nothing here ships with the server).
//!
//! ```text
//! cargo run --release -p silvermere --example populate -- seed --database-url sqlite://world.db
//! cargo run --release -p silvermere --example populate -- run --telnet 127.0.0.1:2323 --minutes 10
//! ```
//!
//! `seed` runs the server in this process on the database and makes the characters the way
//! players do: an account each, the creation dialogue over telnet, then, through the engine,
//! the levels a trainer would give (the game's own gains), the spells the class may learn at
//! that level, the best armour and weapon the class may use (worn with the game's own WEAR
//! and WIELD), and a start in an area whose monsters suit the level. It writes
//! `population.json`: the characters, their passwords and the rooms of their areas.
//!
//! `run` plays them against any server (this process holds none): every character connects
//! from an address of its own, walks its area, fights what it meets, rests when hurt and
//! talks now and then, and the run reports how the server kept up.
//!
//! The levels come in four brackets: 24 characters at 1-5, 20 at 6-15, 12 at 16-30 and 8
//! at 31-50 (for 64; other counts keep the proportions). Names and choices come from
//! `--seed`, so a seed makes the same population.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use silvermere::config::Config;
use silvermere_auth::Role;
use silvermere_auth::password::Cost;
use silvermere_engine::{EngineInput, SeededRng, SessionId, SystemClock, World};
use silvermere_testkit::TelnetClient;
use silvermere_testkit::client::Wait;
use silvermere_types::enums::{Direction, ItemType, WornLocation};
use silvermere_types::ids::{ItemNo, MapId, RoomId, RoomKey};

const PROMPT: &str = r"\[HP=-?\d+[^\]\n]*\]:";
const DIRS: [&str; 10] = ["n", "s", "e", "w", "ne", "nw", "se", "sw", "u", "d"];

#[derive(Parser)]
#[command(about = "A population of characters for demos and stress tests")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Make the characters in a database (the server runs in this process meanwhile).
    Seed {
        /// The database: a copy of an imported world (sqlite://... or postgres://...).
        #[arg(long)]
        database_url: String,
        /// How many characters.
        #[arg(long, default_value_t = 64)]
        count: usize,
        /// The seed of the names and choices.
        #[arg(long, default_value_t = 7)]
        seed: u64,
        /// Where the population is written.
        #[arg(long, default_value = "population.json")]
        out: PathBuf,
        /// How many characters are made at once.
        #[arg(long, default_value_t = 8)]
        parallel: usize,
    },
    /// Play the characters against a running server.
    Run {
        /// The server's telnet address.
        #[arg(long, default_value = "127.0.0.1:2323")]
        telnet: SocketAddr,
        /// What `seed` wrote.
        #[arg(long, default_value = "population.json")]
        population: PathBuf,
        /// How long to play; 0 plays until stopped.
        #[arg(long, default_value_t = 10)]
        minutes: u64,
        /// The first local address the characters connect from (each takes the next).
        /// Linux treats every 127.x.y.z as this computer; elsewhere use 127.0.0.1, `same`.
        #[arg(long, default_value = "127.0.1.1")]
        from: String,
        /// Mean seconds between a character's commands.
        #[arg(long, default_value_t = 2.0)]
        think: f64,
        /// The server's process id, to report its CPU and memory (Linux).
        #[arg(long)]
        server_pid: Option<u32>,
        /// Where the report is written.
        #[arg(long, default_value = "report.json")]
        report: PathBuf,
        /// Only the first N characters.
        #[arg(long)]
        only: Option<usize>,
    },
}

// ---- a small seeded random source ---------------------------------------------------------

/// xorshift64*: the same choices for the same seed on every platform.
#[derive(Clone)]
struct Dice(u64);

impl Dice {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// 0..n
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    fn range(&mut self, lo: u16, hi: u16) -> u16 {
        lo + self.below(usize::from(hi - lo) + 1) as u16
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

// ---- the population file ------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Population {
    seed: u64,
    characters: Vec<Member>,
    /// Each area's rooms: `"map:room"` -> its name and its exits (`"n"` -> `"map:room"`).
    areas: Vec<BTreeMap<String, AreaRoom>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Member {
    name: String,
    password: String,
    class: String,
    race: String,
    level: u16,
    /// Index in `areas`.
    area: usize,
    /// Where the character starts.
    start: String,
    /// What it wears and wields, by name.
    gear: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct AreaRoom {
    name: String,
    exits: BTreeMap<String, String>,
}

fn key_text(k: RoomKey) -> String {
    format!("{}:{}", k.map.0, k.room.0)
}

// ---- seed ---------------------------------------------------------------------------------

/// The level brackets and their share of 64.
const BRACKETS: [(u16, u16, usize); 4] = [(1, 5, 24), (6, 15, 20), (16, 30, 12), (31, 50, 8)];

const SYLLABLES: [&str; 40] = [
    "al", "bar", "cor", "dra", "el", "fen", "gar", "hal", "is", "jor", "kel", "lor", "mar", "nor",
    "or", "pel", "quin", "ran", "sil", "tor", "ul", "var", "wen", "xan", "yor", "zel", "an", "bri",
    "cas", "dun", "eri", "fal", "gil", "hen", "ith", "kai", "lin", "mir", "nim", "ros",
];
const SURNAMES: [&str; 16] = [
    "Ashford",
    "Blackwood",
    "Coldwater",
    "Dunmore",
    "Emberfall",
    "Frostvale",
    "Greymane",
    "Hollowell",
    "Ironside",
    "Marsh",
    "Oakheart",
    "Ravencrest",
    "Stonebrook",
    "Thorne",
    "Wilder",
    "Wyndham",
];

fn name(d: &mut Dice, taken: &mut BTreeSet<String>) -> String {
    loop {
        let parts = 2 + d.below(2);
        let mut n: String = (0..parts).map(|_| *d.pick(&SYLLABLES)).collect();
        n.truncate(10);
        let mut c = n.chars();
        let n = match c.next() {
            Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
            None => continue,
        };
        if n.len() >= 4 && taken.insert(n.to_ascii_lowercase()) {
            return n;
        }
    }
}

/// Runs `f` on the engine thread.
async fn on_world<T: Send + 'static>(
    to_engine: &std::sync::mpsc::Sender<EngineInput>,
    f: impl FnOnce(&mut World) -> T + Send + 'static,
) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    to_engine
        .send(EngineInput::Custom {
            session: None,
            run: Box::new(move |w| {
                let _ = tx.send(f(w));
            }),
        })
        .expect("the engine runs");
    tokio::task::spawn_blocking(move || rx.recv().expect("the engine answers"))
        .await
        .expect("join")
}

/// The areas of the world, by bracket. Each monster type (the room's `mongroup`) is ranked by
/// the median experience its monsters give, and the ranking is split into the four brackets:
/// the easiest types for levels 1-5, the hardest for 31-50. An area is a group of at least
/// five rooms of a bracket's types joined by exits a player can take; the six largest are
/// kept.
fn areas(w: &World) -> Vec<Vec<BTreeMap<String, AreaRoom>>> {
    let mut exp_by_type: HashMap<i16, Vec<u64>> = HashMap::new();
    for m in w.monsters.values() {
        if m.exp > 0 {
            exp_by_type
                .entry(m.montype.code() as i16)
                .or_default()
                .push(m.exp as u64);
        }
    }
    let mut rooms_by_type: HashMap<i16, usize> = HashMap::new();
    for r in w.rooms.values() {
        *rooms_by_type.entry(r.mongroup).or_default() += 1;
    }
    let mut ranked: Vec<(u64, i16)> = exp_by_type
        .into_iter()
        .filter(|(t, _)| *t != 0 && rooms_by_type.get(t).is_some_and(|&n| n >= 5))
        .map(|(t, mut v)| {
            v.sort_unstable();
            (v[v.len() / 2], t)
        })
        .collect();
    ranked.sort_unstable();
    let per = ranked.len().div_ceil(BRACKETS.len()).max(1);
    let mut out = Vec::new();
    for b in 0..BRACKETS.len() {
        let types: BTreeSet<i16> = ranked
            .iter()
            .skip(b * per)
            .take(per)
            .map(|&(_, t)| t)
            .collect();
        let fits: BTreeSet<RoomKey> = w
            .rooms
            .iter()
            .filter(|(_, r)| types.contains(&r.mongroup) && r.maxmonsters > 0)
            .map(|(k, _)| *k)
            .collect();
        // groups joined by exits a player can take
        let mut seen = BTreeSet::new();
        let mut groups: Vec<BTreeMap<String, AreaRoom>> = Vec::new();
        for &start in &fits {
            if !seen.insert(start) {
                continue;
            }
            let mut group = BTreeMap::new();
            let mut todo = vec![start];
            while let Some(k) = todo.pop() {
                let Some(r) = w.room(k) else { continue };
                let mut room = AreaRoom {
                    name: r.name.clone(),
                    exits: BTreeMap::new(),
                };
                for (i, dir) in Direction::COMPASS.iter().enumerate() {
                    if !silvermere_engine::movement::exits::exit_is_available(w, k, *dir) {
                        continue;
                    }
                    let to = RoomKey {
                        map: k.map,
                        room: r.exits[i].target,
                    };
                    if fits.contains(&to) {
                        room.exits.insert(DIRS[i].to_owned(), key_text(to));
                        if seen.insert(to) {
                            todo.push(to);
                        }
                    }
                }
                group.insert(key_text(k), room);
            }
            if group.len() >= 5 {
                groups.push(group);
            }
        }
        groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
        groups.truncate(6);
        out.push(groups);
    }
    out
}

/// What `level_up` hands back: the gear to wear, by name.
fn level_up(
    w: &mut World,
    account: i64,
    level: u16,
    start: RoomKey,
) -> Option<(u16, Vec<String>, Vec<String>)> {
    use silvermere_engine::player::{CalcMode, class_title, recalculate};
    let id: SessionId = w.session_ids().into_iter().find(|&id| {
        w.session(id)
            .and_then(|s| s.account.as_ref())
            .is_some_and(|a| a.id.0 == account)
    })?;
    // the levels a trainer gives, one at a time
    let mut c = w.session(id)?.character.clone()?;
    let (cls, rac) = (
        w.classes.get(&c.class)?.clone(),
        w.races.get(&c.race)?.clone(),
    );
    while c.level < level {
        let next = c.level + 1;
        let gains =
            silvermere_rules::progression::train_gains(next, cls.hplevel, c.lives, &mut w.rng);
        c.level = next;
        c.cp = c.cp.wrapping_add(gains.cp as i16);
        c.totalcp = c.totalcp.wrapping_add(gains.cp);
        c.randomhp = c.randomhp.wrapping_add(gains.hp as u8);
        c.lives = gains.lives;
    }
    // the experience that pays for the level, so loading keeps it
    let need = silvermere_rules::progression::new_calc_exp_needed(
        c.level as i16 - 1,
        cls.exp.wrapping_add(rac.exp),
    );
    c.expbil = need.expbil;
    c.exprem = need.exprem;
    recalculate(w, &mut c, CalcMode::KeepCurrent);
    c.title = class_title(w, c.class, c.level, c.sex);
    c.hp = c.maxhp;
    c.mana = c.maxmana as i16;
    // the spells the class may learn by now
    let spells: Vec<_> = w
        .spells
        .values()
        .filter(|s| s.level > 0 && s.level <= c.level)
        .filter(|s| silvermere_engine::spells::spellbook::user_can_use_spell(w, &c, s).is_ok())
        .map(|s| s.number)
        .collect();
    for s in spells {
        silvermere_engine::items::equip::add_spell_to_spellbook(&mut c, s);
    }
    // the best armour for each place and the best weapon the class may use, nothing limited
    let mut best: BTreeMap<String, (i32, ItemNo, String)> = BTreeMap::new();
    for it in w.items.values() {
        if it.limit > 0 || it.cost == 0 || it.level > c.level || it.strreq > c.str as i16 {
            continue;
        }
        if !silvermere_engine::items::equip::user_can_use(w, &c, it) {
            continue;
        }
        let (slot, score) = match it.item_type {
            ItemType::Weapon => (
                "weapon".to_owned(),
                i32::from(it.mindmg) + i32::from(it.maxdmg),
            ),
            ItemType::Armour if it.worn != WornLocation::Nowhere => (
                format!("{:?}", it.worn),
                i32::from(it.ac) * 10 + i32::from(it.dr),
            ),
            _ => continue,
        };
        if best.get(&slot).is_none_or(|(s, _, _)| score > *s) {
            best.insert(slot, (score, it.number, it.name.clone()));
        }
    }
    let s = w.session_mut(id)?;
    s.character = Some(c);
    let mut gear = Vec::new();
    let mut wield = Vec::new();
    for (slot, (_, item, name)) in best {
        let charges = w.items.get(&item).map_or(0, |i| i.charges);
        if silvermere_engine::items::inventory::add_item_to_inventory(w, id, item, charges) {
            if slot == "weapon" {
                wield.push(name);
            } else {
                gear.push(name);
            }
        }
    }
    silvermere_engine::movement::move_user::set_user_location(w, id, start, true);
    Some((w.session(id)?.character.as_ref()?.maxhp as u16, gear, wield))
}

/// The number a `[N]  Name` menu gives `name`, else the first.
fn menu_number(menu: &str, name: &str) -> String {
    let mut first = None;
    for line in menu.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((n, label)) = rest.split_once(']') else {
            continue;
        };
        if n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() {
            first.get_or_insert_with(|| n.to_owned());
            if label.trim().eq_ignore_ascii_case(name) {
                return n.to_owned();
            }
        }
    }
    first.unwrap_or_else(|| "1".into())
}

async fn expect(
    c: &mut TelnetClient,
    at: usize,
    pattern: &str,
    secs: u64,
) -> anyhow::Result<String> {
    let got = c
        .wait_from(
            at,
            &Wait {
                until: Some(pattern.to_owned()),
                timeout: Duration::from_secs(secs),
                ..Wait::default()
            },
        )
        .await?;
    if !got.matched {
        anyhow::bail!(
            "waited for {pattern:?}; got {:?}",
            tail(&c.plain_from(at), 400)
        );
    }
    Ok(c.plain_from(at))
}

fn tail(s: &str, n: usize) -> String {
    let start = s.len().saturating_sub(n);
    s[s.floor_char_boundary(start)..].to_owned()
}

async fn line(c: &mut TelnetClient, text: &str, until: &str, secs: u64) -> anyhow::Result<String> {
    let at = c.position();
    c.send_line(text).await?;
    expect(c, at, until, secs).await
}

/// Logs in and goes to the `[MAJORMUD]:` menu.
async fn login(c: &mut TelnetClient, name: &str, password: &str) -> anyhow::Result<()> {
    expect(c, 0, "Username:", 30).await?;
    line(c, name, "Password:", 30).await?;
    let got = line(c, password, r"Make your selection|Disconnect the other", 30).await?;
    if got.contains("Disconnect the other") {
        line(c, "Y", "Make your selection", 30).await?;
    }
    line(c, "M", r"MAJORMUD\]", 30).await?;
    Ok(())
}

/// The creation dialogue, line by line, choosing `race` and `class`; ends in the realm.
async fn create(
    c: &mut TelnetClient,
    race: &str,
    class: &str,
    surname: &str,
) -> anyhow::Result<()> {
    let menu = line(c, "E", "choose your race", 30).await?;
    let menu = line(c, &menu_number(&menu, race), "choose your class|Lawful", 30).await?;
    let mut text = if menu.contains("choose your class") {
        line(
            c,
            &menu_number(&menu, class),
            r"Lawful\?|Given Name|Strength",
            30,
        )
        .await?
    } else {
        menu
    };
    const NEXT: &str = r"Lawful\?|Given Name|Family Name|Strength|Intellect|Willpower|Agility|Health|Charm|Hair|Eye|Exit: SAVE|\[HP=";
    for _ in 0..40 {
        // the prompt is the last of them in what came
        let last = NEXT
            .split('|')
            .filter_map(|p| {
                let lit = p.replace('\\', "");
                text.rfind(&lit).map(|i| (i, lit))
            })
            .max_by_key(|(i, _)| *i)
            .map(|(_, p)| p)
            .unwrap_or_default();
        let answer = match last.as_str() {
            "[HP=" => return Ok(()),
            "Lawful?" => {
                if class.eq_ignore_ascii_case("Paladin") {
                    "Y"
                } else {
                    "N"
                }
            }
            "Family Name" => surname,
            _ => "",
        };
        text = line(c, answer, NEXT, 30).await?;
    }
    anyhow::bail!("character creation did not end: {}", tail(&text, 300))
}

async fn seed(
    database_url: String,
    count: usize,
    seed: u64,
    out: PathBuf,
    parallel: usize,
) -> anyhow::Result<()> {
    let mut d = Dice::new(seed);
    let db = silvermere_db::Db::connect_and_migrate(&database_url).await?;
    let mut taken: BTreeSet<String> = db
        .accounts()
        .all()
        .await?
        .into_iter()
        .map(|a| a.account.username.to_ascii_lowercase())
        .collect();
    let classes: Vec<String> = db
        .classes()
        .all()
        .await?
        .into_iter()
        .map(|c| c.name)
        .collect();
    let races: Vec<String> = db
        .races()
        .all()
        .await?
        .into_iter()
        .map(|r| r.name)
        .collect();
    anyhow::ensure!(
        !classes.is_empty() && !races.is_empty(),
        "no world in {database_url}: import one first"
    );

    // the plan: brackets in proportion, every class in turn, races at random
    let total: usize = BRACKETS.iter().map(|b| b.2).sum();
    let mut plan = Vec::new();
    for (i, &(lo, hi, share)) in BRACKETS.iter().enumerate() {
        let n = if i + 1 == BRACKETS.len() {
            count.saturating_sub(plan.len())
        } else {
            (count * share).div_ceil(total).min(count - plan.len())
        };
        for _ in 0..n {
            plan.push((i, d.range(lo, hi)));
        }
    }
    let mut cast = Vec::new();
    for (k, (bracket, level)) in plan.into_iter().enumerate() {
        let n = name(&mut d, &mut taken);
        let password = format!("{}-{}", n.to_ascii_lowercase(), 1000 + d.below(9000));
        cast.push((
            n,
            password,
            classes[k % classes.len()].clone(),
            d.pick(&races).clone(),
            bracket,
            level,
            d.pick(&SURNAMES).to_string(),
        ));
    }
    for (n, pw, ..) in &cast {
        silvermere_auth::cli::create_with_cost(&db, n, pw, &[Role::Player], Cost::Low).await?;
    }
    let ids: HashMap<String, i64> = db
        .accounts()
        .all()
        .await?
        .into_iter()
        .map(|a| (a.account.username.to_ascii_lowercase(), a.id.0))
        .collect();
    db.close().await;

    // the server, here
    let mut cfg = Config::default();
    cfg.database.url = database_url.clone();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.server.web_bind = String::new();
    cfg.backup.enabled = false;
    let running = silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(seed)).await?;
    let addr = running.telnet_addr;
    let tx = running.to_engine.clone();
    let by_bracket = on_world(&tx, |w| areas(w)).await;
    let mut all_areas = Vec::new();
    let mut area_of_bracket: Vec<Vec<usize>> = Vec::new();
    for groups in by_bracket {
        let mut idx = Vec::new();
        for g in groups {
            idx.push(all_areas.len());
            all_areas.push(g);
        }
        area_of_bracket.push(idx);
    }
    for (i, a) in area_of_bracket.iter().enumerate() {
        eprintln!(
            "levels {}-{}: {} areas ({} rooms)",
            BRACKETS[i].0,
            BRACKETS[i].1,
            a.len(),
            a.iter().map(|&x| all_areas[x].len()).sum::<usize>()
        );
        anyhow::ensure!(
            !a.is_empty(),
            "no area fits levels {}-{}",
            BRACKETS[i].0,
            BRACKETS[i].1
        );
    }

    let members = Arc::new(Mutex::new(Vec::new()));
    let sem = Arc::new(tokio::sync::Semaphore::new(parallel.max(1)));
    let mut tasks = Vec::new();
    for (n, pw, class, race, bracket, level, surname) in cast {
        let choices = &area_of_bracket[bracket];
        let area = *d.pick(choices);
        let rooms: Vec<&String> = all_areas[area].keys().collect();
        let start = (*d.pick(&rooms)).clone();
        let account = ids[&n.to_ascii_lowercase()];
        let (members, sem, tx) = (members.clone(), sem.clone(), tx.clone());
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await;
            let made: anyhow::Result<Member> = async {
                let mut c = TelnetClient::connect(addr).await?.with_naws(79, 24);
                login(&mut c, &n, &pw).await?;
                create(&mut c, &race, &class, &surname).await?;
                let (map, room) = start.split_once(':').unwrap_or(("1", "1"));
                let key = RoomKey {
                    map: MapId(map.parse()?),
                    room: RoomId(room.parse()?),
                };
                let (_, armour, weapons) = on_world(&tx, move |w| level_up(w, account, level, key))
                    .await
                    .ok_or_else(|| anyhow::anyhow!("{n}: not in the game"))?;
                for item in &weapons {
                    line(&mut c, &format!("wield {item}"), PROMPT, 20).await?;
                }
                for item in &armour {
                    line(&mut c, &format!("wear {item}"), PROMPT, 20).await?;
                }
                let mut gear = weapons.clone();
                gear.extend(armour);
                eprintln!(
                    "{n}: level {level} {race} {class}, {} items, at {start}",
                    gear.len()
                );
                Ok(Member {
                    name: n.clone(),
                    password: pw.clone(),
                    class: class.clone(),
                    race: race.clone(),
                    level,
                    area,
                    start: start.clone(),
                    gear,
                })
            }
            .await;
            match made {
                Ok(m) => members.lock().unwrap().push(m),
                Err(e) => eprintln!("{n}: not made: {e:#}"),
            }
        }));
    }
    for t in tasks {
        t.await?;
    }
    // the shutdown saves every character where it stands
    running
        .shutdown_gracefully(
            Duration::from_secs(10),
            Duration::from_secs(10),
            Duration::from_secs(60),
        )
        .await?;
    let mut characters = members.lock().unwrap().clone();
    characters.sort_by(|a, b| a.level.cmp(&b.level).then(a.name.cmp(&b.name)));
    let made = characters.len();
    let pop = Population {
        seed,
        characters,
        areas: all_areas,
    };
    std::fs::write(&out, serde_json::to_string_pretty(&pop)?)?;
    eprintln!(
        "{made} of {count} characters made; written to {}",
        out.display()
    );
    Ok(())
}

// ---- run ----------------------------------------------------------------------------------

#[derive(Default, Serialize)]
struct Stats {
    commands: u64,
    /// Milliseconds from a command to its prompt.
    latencies: Vec<u32>,
    moves: u64,
    fights: u64,
    kills: u64,
    deaths: u64,
    rests: u64,
    chats: u64,
    disconnects: u64,
    errors: Vec<String>,
    /// (seconds into the run, server CPU percent of one core, resident MB)
    server: Vec<(u64, f64, f64)>,
}

const GOSSIP: [&str; 8] = [
    "anyone up for a hunt?",
    "selling a spare shield, cheap",
    "watch out for the orcs out east",
    "what level do you train at?",
    "the bank is the safest place for gold",
    "lost my torch again...",
    "looking for a party",
    "good morning, realm!",
];

/// The names in an "Also here:" line that are not players: the monsters to fight.
fn monsters(screen: &str, players: &BTreeSet<String>) -> Vec<String> {
    let Some(i) = screen.rfind("Also here:") else {
        return Vec::new();
    };
    let list = &screen[i + "Also here:".len()..];
    let list = list.split('.').next().unwrap_or("");
    list.split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .filter(|s| {
            !players.contains(
                &s.split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase(),
            )
        })
        .collect()
}

/// Hit points in the last prompt.
fn hp(screen: &str) -> Option<i32> {
    let i = screen.rfind("[HP=")?;
    screen[i + 4..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect::<String>()
        .parse()
        .ok()
}

/// The exits an "Obvious exits:" line lists, as commands.
fn exits(screen: &str) -> Vec<String> {
    let Some(i) = screen.rfind("Obvious exits:") else {
        return Vec::new();
    };
    let list = screen[i + "Obvious exits:".len()..]
        .lines()
        .next()
        .unwrap_or("");
    let words = [
        ("northeast", "ne"),
        ("northwest", "nw"),
        ("southeast", "se"),
        ("southwest", "sw"),
        ("north", "n"),
        ("south", "s"),
        ("east", "e"),
        ("west", "w"),
        ("up", "u"),
        ("down", "d"),
    ];
    list.split(',')
        .filter_map(|e| {
            let e = e.trim().to_ascii_lowercase();
            if e.contains("closed") {
                return None;
            }
            let last = e.split_whitespace().last()?.to_owned();
            words
                .iter()
                .find(|(w, _)| last == *w)
                .map(|(_, d)| (*d).to_owned())
        })
        .collect()
}

async fn sleep_think(d: &mut Dice, mean: f64) {
    let ms = (mean * 1000.0 * (0.5 + d.below(100) as f64 / 100.0)) as u64;
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// One character and what its play needs.
struct Bot {
    m: Member,
    area: BTreeMap<String, AreaRoom>,
    telnet: SocketAddr,
    from: Option<IpAddr>,
    players: Arc<BTreeSet<String>>,
    think: f64,
    deadline: Option<Instant>,
    stats: Arc<Mutex<Stats>>,
    seed: u64,
}

/// One character's play until the deadline.
async fn play(bot: Bot) {
    let Bot {
        m,
        area,
        telnet,
        from,
        players,
        think,
        deadline,
        stats,
        seed,
    } = bot;
    let mut d = Dice::new(seed);
    let maxhp = 10 + i32::from(m.level) * 6; // refined from the prompt as the play goes
    let mut best_hp = maxhp.min(1);
    loop {
        if deadline.is_some_and(|t| Instant::now() > t) {
            return;
        }
        let conn = match from {
            Some(ip) => TelnetClient::connect_from(telnet, ip).await,
            None => TelnetClient::connect(telnet).await,
        };
        let mut c = match conn {
            Ok(c) => c.with_naws(79, 24),
            Err(e) => {
                stats
                    .lock()
                    .unwrap()
                    .errors
                    .push(format!("{}: connect: {e}", m.name));
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let entered: anyhow::Result<String> = async {
            login(&mut c, &m.name, &m.password).await?;
            line(&mut c, "E", PROMPT, 60).await
        }
        .await;
        let mut screen = match entered {
            Ok(s) => s,
            Err(e) => {
                stats
                    .lock()
                    .unwrap()
                    .errors
                    .push(format!("{}: login: {e:#}", m.name));
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let mut here = m.start.clone();
        loop {
            if deadline.is_some_and(|t| Instant::now() > t) {
                // the clean way out, which saves without a hang-up
                let _ = line(&mut c, "x", r"MAJORMUD\]", 90).await;
                c.disconnect().await;
                return;
            }
            sleep_think(&mut d, think).await;
            if let Some(h) = hp(&screen) {
                best_hp = best_hp.max(h);
            }
            let hurt = hp(&screen).is_some_and(|h| h * 2 < best_hp);
            let foes = monsters(&screen, &players);
            let (cmd, until, kind) = if hurt {
                ("rest".to_owned(), PROMPT.to_owned(), "rest")
            } else if !foes.is_empty() && d.chance(80) {
                let foe = foes[d.below(foes.len())]
                    .split_whitespace()
                    .last()
                    .unwrap_or("")
                    .to_owned();
                (
                    format!("a {foe}"),
                    r"\*Combat Off\*|You gain|You have been killed|no .* here|You don't see"
                        .to_owned(),
                    "fight",
                )
            } else if d.chance(2) {
                (
                    format!("gos {}", d.pick(&GOSSIP)),
                    PROMPT.to_owned(),
                    "chat",
                )
            } else {
                // an exit of the area when this room is in it, else any the room shows
                let known: Vec<(String, String)> = area
                    .get(&here)
                    .map(|r| {
                        r.exits
                            .iter()
                            .map(|(a, b)| (a.clone(), b.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                let dir = if known.is_empty() {
                    let shown = exits(&screen);
                    if shown.is_empty() {
                        "look".to_owned()
                    } else {
                        d.pick(&shown).clone()
                    }
                } else {
                    let (dir, to) = d.pick(&known).clone();
                    here = to;
                    dir
                };
                (dir, r"Obvious exits|no exit|\[HP=".to_owned(), "move")
            };
            let sent = Instant::now();
            let at = c.position();
            if c.send_line(&cmd).await.is_err() {
                break;
            }
            let secs = if kind == "fight" { 60 } else { 30 };
            let got = c
                .wait_from(
                    at,
                    &Wait {
                        until: Some(until.clone()),
                        timeout: Duration::from_secs(secs),
                        ..Wait::default()
                    },
                )
                .await;
            let Ok(got) = got else { break };
            screen = c.plain_from(at);
            {
                let mut s = stats.lock().unwrap();
                s.commands += 1;
                if got.matched {
                    s.latencies
                        .push(sent.elapsed().as_millis().min(u128::from(u32::MAX)) as u32);
                }
                match kind {
                    "rest" => s.rests += 1,
                    "chat" => s.chats += 1,
                    "move" => s.moves += 1,
                    _ => {
                        s.fights += 1;
                        if screen.contains("You gain") {
                            s.kills += 1;
                        }
                    }
                }
                if screen.contains("You have been killed") || screen.contains("You are dead") {
                    s.deaths += 1;
                    here = String::new();
                }
            }
            if kind == "rest" {
                // rest until the prompt shows most hit points back, or a minute
                let until = Instant::now() + Duration::from_secs(60);
                while Instant::now() < until && hp(&screen).is_some_and(|h| h * 10 < best_hp * 9) {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    match line(&mut c, "", PROMPT, 20).await {
                        Ok(t) => screen = t,
                        Err(_) => break,
                    }
                }
            }
            if !c.is_open() {
                break;
            }
        }
        stats.lock().unwrap().disconnects += 1;
    }
}

/// CPU seconds and resident MB of `pid` (Linux `/proc`).
fn proc_sample(pid: u32) -> Option<(f64, f64)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let ticks: f64 = fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let rss_kb: f64 = status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((ticks / 100.0, rss_kb / 1024.0))
}

#[allow(clippy::too_many_arguments)] // a command line's options
async fn run(
    telnet: SocketAddr,
    population: PathBuf,
    minutes: u64,
    from: String,
    think: f64,
    server_pid: Option<u32>,
    report: PathBuf,
    only: Option<usize>,
) -> anyhow::Result<()> {
    let pop: Population = serde_json::from_str(&std::fs::read_to_string(&population)?)?;
    let mut members = pop.characters.clone();
    if let Some(n) = only {
        members.truncate(n);
    }
    let players: Arc<BTreeSet<String>> = Arc::new(
        pop.characters
            .iter()
            .map(|m| m.name.to_ascii_lowercase())
            .collect(),
    );
    let first: Option<Ipv4Addr> = match from.as_str() {
        "same" => None,
        ip => Some(ip.parse()?),
    };
    let started = Instant::now();
    let deadline = (minutes > 0).then(|| started + Duration::from_secs(minutes * 60));
    let stats = Arc::new(Mutex::new(Stats::default()));
    let mut tasks = Vec::new();
    for (i, m) in members.into_iter().enumerate() {
        let area = pop.areas.get(m.area).cloned().unwrap_or_default();
        let ip = first.map(|f| IpAddr::V4(Ipv4Addr::from(u32::from(f) + i as u32)));
        let (players, stats) = (players.clone(), stats.clone());
        let seed = pop.seed ^ (i as u64 + 1).wrapping_mul(0x9E37);
        tasks.push(tokio::spawn(async move {
            // a few at a time, as players arrive
            tokio::time::sleep(Duration::from_millis(250 * i as u64)).await;
            play(Bot {
                m,
                area,
                telnet,
                from: ip,
                players,
                think,
                deadline,
                stats,
                seed,
            })
            .await;
        }));
    }
    // the server's CPU and memory, every 10 seconds
    let sampler = server_pid.map(|pid| {
        let stats = stats.clone();
        tokio::spawn(async move {
            let mut last = proc_sample(pid);
            let mut t = 0;
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                t += 10;
                let now = proc_sample(pid);
                if let (Some((c0, _)), Some((c1, rss))) = (last, now) {
                    stats
                        .lock()
                        .unwrap()
                        .server
                        .push((t, (c1 - c0) * 10.0, rss));
                }
                last = now;
            }
        })
    });
    // a line every 30 seconds
    let progress = {
        let stats = stats.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let s = stats.lock().unwrap();
                eprintln!(
                    "{:>4}s: {} commands, {} moves, {} fights ({} kills), {} deaths, {} disconnects",
                    started.elapsed().as_secs(),
                    s.commands,
                    s.moves,
                    s.fights,
                    s.kills,
                    s.deaths,
                    s.disconnects
                );
            }
        })
    };
    for t in tasks {
        let _ = t.await;
    }
    progress.abort();
    if let Some(s) = sampler {
        s.abort();
    }
    let s = stats.lock().unwrap();
    let mut lat = s.latencies.clone();
    lat.sort_unstable();
    let pct = |p: f64| {
        lat.get(((lat.len() as f64 - 1.0) * p) as usize)
            .copied()
            .unwrap_or(0)
    };
    let secs = started.elapsed().as_secs_f64();
    let summary = serde_json::json!({
        "characters": players.len().min(only.unwrap_or(usize::MAX)),
        "seconds": secs.round(),
        "commands": s.commands,
        "commands_per_second": (s.commands as f64 / secs * 10.0).round() / 10.0,
        "latency_ms": { "p50": pct(0.5), "p90": pct(0.9), "p99": pct(0.99), "max": lat.last().copied().unwrap_or(0) },
        "moves": s.moves, "fights": s.fights, "kills": s.kills, "deaths": s.deaths,
        "rests": s.rests, "chats": s.chats, "disconnects": s.disconnects,
        "errors": s.errors.iter().take(20).collect::<Vec<_>>(),
        "server": s.server.iter().map(|(t, cpu, rss)| serde_json::json!({ "t": t, "cpu_pct": (cpu * 10.0).round() / 10.0, "rss_mb": rss.round() })).collect::<Vec<_>>(),
    });
    std::fs::write(&report, serde_json::to_string_pretty(&summary)?)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Seed {
            database_url,
            count,
            seed: s,
            out,
            parallel,
        } => seed(database_url, count, s, out, parallel).await,
        Cmd::Run {
            telnet,
            population,
            minutes,
            from,
            think,
            server_pid,
            report,
            only,
        } => {
            run(
                telnet, population, minutes, from, think, server_pid, report, only,
            )
            .await
        }
    }
}
