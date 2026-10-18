//! The text block script engine (T-054) over the stock records: every block a script can
//! reach parsed, a monster's random text, the Newhaven skiff, the level-10 crypt quest
//! walked by a Witchunter, monster talk and quest flags through save and load. The
//! synthetic-world tests are `silvermere-engine/tests/scripts.rs`.
//!
//! The records come from the downloaded distribution ([`extract`]). Every test lives in
//! `mod scripts`, so `cargo test -p silvermere-testkit scripts::` selects them.

#[path = "../../silvermere-engine/tests/common/mod.rs"]
mod common;
#[path = "support/stock_records.rs"]
mod stock_records;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{Value, json};
use stock_records::{Row, int, ints, table, text};

/// The stock records the tests below read, as one JSON object:
///
/// - `blocks`: every text block a script can reach: room command texts, monster talk
///   texts with their answers and the answers' links, monster random texts, the Text Block
///   (148) and Death Text (155) abilities of items, spells, monsters, races and classes,
///   and the blocks `random`, `checkspell` and `testskill` name;
/// - `roots`: those block numbers by kind;
/// - `rooms`, `messages`, `items`, `monsters`: the map 1 rooms of the Graveyard crypt and
///   the class quest area (`Crypt*`, `Marble*`, `Large Chamber`, `Large Tomb`), the Tomb
///   Entrance (834), Newhaven Docks (2149) and the Small Pier (2335), the messages and
///   items their exits and floors name, and five monsters (witchunter spirit, dark
///   cultist, troll, wild dog, pirate ghost).
fn extract() -> Value {
    // (part, number) -> (link, text); a later record of the same key replaces the earlier
    let mut blocks: HashMap<(i64, i64), (i64, String)> = HashMap::new();
    for r in table("WCCTEXT") {
        blocks.insert(
            (int(r, "part"), int(r, "number")),
            (int(r, "link"), text(r, "text").to_owned()),
        );
    }
    let parts = |n: i64| -> Vec<String> {
        (0..)
            .map_while(|p| blocks.get(&(p, n)).map(|(_, t)| t.clone()))
            .collect()
    };
    let kinds = ["cmdtext", "talktext", "randtext", "textblock", "deathtext"];
    let mut roots: BTreeMap<&str, BTreeSet<i64>> =
        kinds.iter().map(|k| (*k, BTreeSet::new())).collect();
    let mut rooms: BTreeMap<(i64, i64), &Row> = BTreeMap::new();
    for r in table("WCCMP001") {
        rooms.insert((int(r, "mapnum"), int(r, "roomnum")), r);
        if int(r, "cmdtext") != 0 {
            roots.get_mut("cmdtext").unwrap().insert(int(r, "cmdtext"));
        }
    }
    let mut monsters: HashMap<i64, &Row> = HashMap::new();
    for r in table("WCCKNMSR") {
        monsters.insert(int(r, "number"), r);
        for k in ["talktext", "randtext"] {
            if int(r, k) != 0 {
                roots.get_mut(k).unwrap().insert(int(r, k));
            }
        }
    }
    let mut items: HashMap<i64, &Row> = HashMap::new();
    for f in ["WCCITEMS", "WCCSPELS", "WCCKNMSR", "WCCRACE", "WCCCLASS"] {
        for r in table(f) {
            if f == "WCCITEMS" {
                items.insert(int(r, "number"), r);
            }
            for (a, v) in ints(r, "abilities").into_iter().zip(ints(r, "abilvals")) {
                if a == 148 && v != 0 {
                    roots.get_mut("textblock").unwrap().insert(v);
                }
                if a == 155 && v != 0 {
                    roots.get_mut("deathtext").unwrap().insert(v);
                }
            }
        }
    }

    // a room's floor as the game sets it up at start: its items, then each permanently
    // placed item it does not hold yet
    let floor = |r: &Row| -> Vec<i64> {
        let mut out: Vec<i64> = ints(r, "items").into_iter().filter(|&i| i != 0).collect();
        for p in ints(r, "placed") {
            if p != 0 && !out.contains(&p) {
                out.push(p);
            }
        }
        out
    };

    // the leading integer after a line's first colon, if any
    let answer = |line: &str| -> Option<i64> {
        let rest = line.split_once(':')?.1.trim_start();
        let end = rest
            .char_indices()
            .find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && c == '-')))
            .map_or(rest.len(), |(i, _)| i);
        rest[..end].parse().ok()
    };
    let talk = &roots["talktext"];
    let mut keep: BTreeSet<i64> = BTreeSet::new();
    let mut todo: Vec<i64> = kinds
        .iter()
        .flat_map(|k| roots[k].iter().copied())
        .collect();
    while let Some(n) = todo.pop() {
        if !keep.insert(n) {
            continue;
        }
        for t in parts(n) {
            for line in t.split('\n') {
                for c in line.split(':') {
                    let w: Vec<&str> = c.split_whitespace().collect();
                    let Some(first) = w.first() else { continue };
                    let k = first.to_lowercase();
                    let arg = |i: usize| w[i].parse::<i64>().expect("a block number");
                    if k.starts_with("random") && w.len() > 1 {
                        todo.push(arg(1));
                    } else if k.starts_with("checkspell") && w.len() > 2 {
                        todo.push(arg(2));
                    } else if k.starts_with("testskill") && w.len() > 3 {
                        todo.push(arg(3));
                    }
                }
                if talk.contains(&n)
                    && let Some(a) = answer(line)
                {
                    todo.push(a);
                    if let Some(&(link, _)) = blocks.get(&(0, a))
                        && link != 0
                    {
                        todo.push(link);
                    }
                }
            }
        }
        if talk.contains(&n)
            && let Some(&(link, _)) = blocks.get(&(0, n))
            && link != 0
        {
            todo.push(link);
        }
    }

    // the crypt (map 1) and the Newhaven skiff
    let walk: Vec<(i64, i64)> = rooms
        .iter()
        .filter(|(k, r)| {
            let name = text(r, "name");
            k.0 == 1
                && (["Crypt", "Marble", "Large Chamber", "Large Tomb"]
                    .iter()
                    .any(|p| name.starts_with(p))
                    || [834, 2149, 2335].contains(&k.1))
        })
        .map(|(k, _)| *k)
        .collect();
    let mut msgs_wanted = BTreeSet::new();
    let mut items_wanted = BTreeSet::new();
    for k in &walk {
        let r = rooms[k];
        let (exits, types) = (ints(r, "exits"), ints(r, "exittypes"));
        let (par1, par3) = (ints(r, "exitpar1s"), ints(r, "exitpar3s"));
        for d in 0..10 {
            if exits[d] != 0 {
                msgs_wanted.extend([par1[d], par3[d]]);
                if [2, 3].contains(&types[d]) {
                    items_wanted.insert(par1[d]);
                }
            }
        }
        items_wanted.extend(floor(r).into_iter().chain(ints(r, "hiditems")));
    }
    let mut msgs: BTreeMap<i64, Value> = BTreeMap::new();
    for r in table("WCCMSG") {
        let n = int(r, "number");
        if n != 0 && msgs_wanted.contains(&n) {
            msgs.insert(n, r["msgs"].clone());
        }
    }
    // the witchunter spirit (Large Tomb), a dark cultist (random text 9108), the troll
    // (talk block 119), a wild dog (a talk block of text only) and a pirate ghost (no talk
    // block)
    let mons = [85, 29, 82, 50, 874];
    for m in mons {
        let t = int(monsters[&m], "talktext");
        keep.insert(t);
        for p in parts(t) {
            for line in p.split('\n') {
                if line.contains(':') {
                    keep.insert(answer(line).expect("an answer's block"));
                }
            }
        }
        if let Some(&(link, _)) = blocks.get(&(0, t)) {
            keep.insert(link);
        }
    }

    let kept: BTreeMap<_, _> = blocks.iter().filter(|(k, _)| keep.contains(&k.1)).collect();
    json!({
        "roots": roots.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
        "blocks": kept.iter().map(|((p, n), (link, t))| json!({"part": p, "number": n, "link": link, "text": t})).collect::<Vec<_>>(),
        "rooms": walk.iter().map(|k| {
            let r = rooms[k];
            let ex = ["exits", "exittypes", "exitpar1s", "exitpar2s", "exitpar3s", "exitpar4s"].map(|f| ints(r, f));
            json!({
                "map": k.0, "room": k.1, "name": r["name"], "type": r["type"],
                "minlevel": r["minlevel"], "maxlevel": r["maxlevel"], "cmdtext": r["cmdtext"],
                "exits": (0..10).filter(|&d| ex[0][d] != 0).map(|d| {
                    json!([d, ex[0][d], ex[1][d], ex[2][d], ex[3][d], ex[4][d], ex[5][d]])
                }).collect::<Vec<_>>(),
                "items": floor(r),
            })
        }).collect::<Vec<_>>(),
        "messages": msgs.iter().map(|(n, m)| json!({"number": n, "msgs": m})).collect::<Vec<_>>(),
        "items": items_wanted.iter().filter_map(|n| items.get(n).map(|r| json!({
            "number": n, "name": r["name"], "type": r["type"], "limit": r["limit"],
            "weight": r["weight"], "charges": r["charges"],
        }))).collect::<Vec<_>>(),
        "monsters": mons.iter().map(|m| {
            let r = monsters[m];
            json!({"number": m, "name": r["name"], "talktext": r["talktext"], "randtext": r["randtext"]})
        }).collect::<Vec<_>>(),
    })
}

mod scripts {
    use std::collections::{BTreeMap, BTreeSet};

    use super::common::{enter, test_config, tiny_snapshot};
    use silvermere_engine::commands::input::execute_input;
    use silvermere_engine::commands::pending::fast_update_delay;
    use silvermere_engine::cycles::slow::perform_random_event;
    use silvermere_engine::items::inventory::add_item_to_inventory;
    use silvermere_engine::movement::exits::{
        LOCK_CLOSED, LOCK_LOCKED, LOCK_OPEN, live_exit, set_par2,
    };
    use silvermere_engine::player::save::save_player;
    use silvermere_engine::player_session::AccountRef;
    use silvermere_engine::scripts::{
        Clause, Keyword, STOCK_TELEPORTS, pattern_lines, whole_lines,
    };
    use silvermere_engine::tell::palette_of;
    use silvermere_engine::world::blank;
    use silvermere_engine::{EngineOutput, PersistRequest, SeededRng, SessionId, World};
    use silvermere_types::enums::{
        Alignment, Direction, ExitType, GroupRole, ItemType, MessageLevel, MonsterType, RoomType,
    };
    use silvermere_types::flags::{ActiveMonsterFlags, ExitFlags, PlayerSettings};
    use silvermere_types::ids::{
        AccountId, ActiveMonsterId, ClassNo, ItemNo, MessageNo, MonsterNo, RoomId, RoomKey,
        SpellNo, TextBlockNo,
    };
    use silvermere_types::records::{
        ActiveMonster, ActiveSpell, Coins, Exit, Message, RoomItem, TextBlock,
    };
    use silvermere_types::sizes::{NAMTRAIL, NMONITEMS, NMONSPELLS};

    fn message(n: i32, texts: [&str; 3]) -> Message {
        Message {
            number: MessageNo(n),
            msgs: texts.map(String::from),
        }
    }

    fn active_monster(number: i32, known: i32, name: &str, room: u32) -> ActiveMonster {
        ActiveMonster {
            number: ActiveMonsterId(number),
            knownnum: MonsterNo(known),
            level: 0,
            map: 1.into(),
            room: room.into(),
            poison: 0,
            energy: 0,
            hp: 10,
            following: String::new(),
            trail: [0.into(); NAMTRAIL],
            hunters: [ActiveMonsterId(0); 10],
            hunting: ActiveMonsterId(0),
            name: name.into(),
            wornitem: ItemNo(0),
            weapon: ItemNo(0),
            items: [(ItemNo(0), 0); NMONITEMS],
            cash: Coins::default(),
            maxhp: 10,
            alignment: Alignment::from_code(0),
            followpct: 0,
            dr: 0,
            ac: 0,
            exp: 0,
            maxenergy: 0,
            friendly: 0,
            genroom: room.into(),
            inactive: 0,
            flags: ActiveMonsterFlags::empty(),
            montype: MonsterType::from_code(0),
            target: 0,
            hpregen: 0,
            lastdir: 0,
            lastupd: 0,
            grouprole: GroupRole::from_code(0),
            spells: [ActiveSpell {
                spell: SpellNo(0),
                value: 0,
                rounds: 0,
            }; NMONSPELLS],
            downrounds: 0,
            created: 0,
        }
    }

    /// Puts an active monster of kind `known` into `room` (map 1).
    fn place(w: &mut World, number: i32, known: i32, name: &str, room: u32) {
        w.active_monsters
            .insert(active_monster(number, known, name, room));
        let st = w.room_state.get_mut(&RoomKey::new(1, room)).unwrap();
        let slot = st.monsters.iter().position(|m| m.0 == 0).unwrap();
        st.monsters[slot] = ActiveMonsterId(number);
    }

    // -----------------------------------------------------------------------------------
    // The stock data.

    struct Stock {
        json: serde_json::Value,
    }

    fn stock() -> Stock {
        Stock {
            json: super::extract(),
        }
    }

    impl Stock {
        fn blocks(&self) -> BTreeMap<(i16, i32), (i32, String)> {
            self.json["blocks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| {
                    (
                        (
                            b["part"].as_i64().unwrap() as i16,
                            b["number"].as_i64().unwrap() as i32,
                        ),
                        (
                            b["link"].as_i64().unwrap() as i32,
                            b["text"].as_str().unwrap().to_owned(),
                        ),
                    )
                })
                .collect()
        }

        fn roots(&self, kind: &str) -> Vec<i32> {
            self.json["roots"][kind]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap() as i32)
                .collect()
        }

        /// The stock rooms, messages, items, monsters and blocks in a world.
        fn world(&self) -> World {
            let mut s = tiny_snapshot();
            let j = &self.json;
            for r in j["rooms"].as_array().unwrap() {
                let n = |k: &str| r[k].as_i64().unwrap();
                let mut room = blank::clear_room(RoomKey::new(n("map") as u32, n("room") as u32));
                room.name = r["name"].as_str().unwrap().into();
                room.room_type = RoomType::from_code(n("type") as u16);
                room.minlevel = n("minlevel") as i16;
                room.maxlevel = n("maxlevel") as i16;
                room.cmdtext = TextBlockNo(n("cmdtext") as i32);
                for e in r["exits"].as_array().unwrap() {
                    let e: Vec<i64> = e
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_i64().unwrap())
                        .collect();
                    room.exits[e[0] as usize] = Exit {
                        target: RoomId(e[1] as u32),
                        kind: ExitType::from_code(e[2] as u16),
                        par1: e[3] as u32,
                        par2: e[4] as i16,
                        par3: e[5] as i32,
                        par4: e[6] as i32,
                    };
                }
                for (i, it) in r["items"].as_array().unwrap().iter().enumerate() {
                    room.items[i] = RoomItem {
                        item: ItemNo(it.as_i64().unwrap() as i32),
                        uses: -1,
                        count: 0,
                    };
                }
                s.rooms.retain(|x| x.key != room.key);
                s.rooms.push(room);
            }
            for m in j["messages"].as_array().unwrap() {
                let t: Vec<&str> = m["msgs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
                    .collect();
                s.messages.push(message(
                    m["number"].as_i64().unwrap() as i32,
                    [t[0], t[1], t[2]],
                ));
            }
            for i in j["items"].as_array().unwrap() {
                let mut it = blank::clear_item(ItemNo(i["number"].as_i64().unwrap() as i32));
                it.name = i["name"].as_str().unwrap().into();
                it.item_type = ItemType::from_code(i["type"].as_i64().unwrap() as u16);
                it.weight = i["weight"].as_i64().unwrap() as i16;
                it.charges = i["charges"].as_i64().unwrap() as i16;
                it.gettable = 1;
                s.items.push(it);
            }
            for m in j["monsters"].as_array().unwrap() {
                let mut k =
                    blank::clear_known_monster(MonsterNo(m["number"].as_i64().unwrap() as i32));
                k.name = m["name"].as_str().unwrap().into();
                k.talktext = TextBlockNo(m["talktext"].as_i64().unwrap() as i32);
                k.randtext = TextBlockNo(m["randtext"].as_i64().unwrap() as i32);
                s.monsters.push(k);
            }
            for ((part, number), (link, text)) in self.blocks() {
                s.text_blocks.push(TextBlock {
                    part,
                    number: TextBlockNo(number),
                    link: TextBlockNo(link),
                    text,
                });
            }
            let mut w = World::load(test_config(), 8, s).expect("the stock rooms load");
            w.rng = SeededRng::new(11);
            w
        }
    }

    // T-054 Done-when: every block a room, a monster's talk, a spell, an item or a death
    // reaches parses; the clauses with no keyword are listed (they do nothing on the original).
    #[test]
    fn every_stock_block_parses() {
        let st = stock();
        let blocks = st.blocks();
        let text = |n: i32| -> Vec<String> {
            (0..50i16)
                .map_while(|p| blocks.get(&(p, n)).map(|(_, t)| t.clone()))
                .collect()
        };
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum Kind {
            Pattern,
            Random,
            Whole,
        }
        let mut todo: Vec<(Kind, i32)> = Vec::new();
        todo.extend(st.roots("cmdtext").into_iter().map(|n| (Kind::Pattern, n)));
        todo.extend(st.roots("textblock").into_iter().map(|n| (Kind::Whole, n)));
        todo.extend(st.roots("deathtext").into_iter().map(|n| (Kind::Whole, n)));
        // a monster's answers: each answer's link runs as a whole block
        for n in st.roots("talktext") {
            for t in text(n) {
                for pl in pattern_lines(&t) {
                    let a = silvermere_engine::scripts::leading_int32(&pl.action);
                    if let Some((link, _)) = blocks.get(&(0, a))
                        && *link != 0
                    {
                        todo.push((Kind::Whole, *link));
                    }
                }
            }
        }
        let mut seen = BTreeSet::new();
        let mut used: BTreeMap<Keyword, usize> = BTreeMap::new();
        let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
        let mut prose_lines = 0usize;
        let (mut lines, mut clauses) = (0usize, 0usize);
        while let Some((kind, n)) = todo.pop() {
            if !seen.insert((kind, n)) {
                continue;
            }
            for t in text(n) {
                let actions: Vec<String> = match kind {
                    Kind::Whole => whole_lines(&t).into_iter().map(String::from).collect(),
                    Kind::Pattern | Kind::Random => {
                        pattern_lines(&t).into_iter().map(|p| p.action).collect()
                    }
                };
                for line in actions {
                    lines += 1;
                    let parsed: Vec<Clause> = silvermere_engine::scripts::clauses(&line)
                        .into_iter()
                        .map(|(c, _)| Clause::parse(c, &STOCK_TELEPORTS))
                        .collect();
                    if !line.trim().is_empty() && parsed.iter().all(|c| c.keyword().is_none()) {
                        prose_lines += 1;
                        continue;
                    }
                    for c in parsed {
                        clauses += 1;
                        match &c {
                            Clause::Unknown(s) if s.trim().is_empty() => {}
                            Clause::Unknown(s) => {
                                *unknown.entry(s.trim().to_owned()).or_default() += 1
                            }
                            Clause::Random { block } => todo.push((Kind::Random, *block)),
                            Clause::CheckSpell { block, .. } => todo.push((Kind::Whole, *block)),
                            Clause::TestSkill { block, .. } => {
                                todo.push((Kind::Whole, i32::from(*block)))
                            }
                            _ => {}
                        }
                        if let Some(k) = c.keyword() {
                            *used.entry(k).or_default() += 1;
                        }
                    }
                }
            }
        }
        // the stock data's own mistakes: clauses that do nothing on the original
        let want: BTreeMap<String, usize> = [
            ("2683 1", 1),
            ("add delay 5", 5),
            ("check class", 195),
            ("delay 5", 20),
            ("goodability -51 3154", 2),
            ("levelcheck", 62),
            ("make ornate key", 1),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        eprintln!("{lines} lines, {clauses} clauses, {prose_lines} prose lines");
        eprintln!("keywords used: {used:?}");
        assert_eq!(unknown, want, "clauses with no keyword");
        // 37 of the 42 keywords appear in the stock data; hideitem, roomtext, checkskill,
        // test_tournament and flag do not
        assert_eq!(used.len(), 37, "{used:?}");
    }

    // A monster's random text is `percent text` lines for the slow cycle, not a script.
    #[test]
    fn stock_random_texts_are_percent_lines() {
        let st = stock();
        let blocks = st.blocks();
        for n in st.roots("randtext") {
            let (_, t) = &blocks[&(0, n)];
            let ok = silvermere_engine::cycles::slow::event_line(t, 0).is_some();
            // block 107 is a talk block ("shit:80") used as random text by mistake
            assert!(ok || n == 107, "{n}: {t:?}");
        }
    }

    // -----------------------------------------------------------------------------------
    // Over the stock data.

    struct Walker {
        w: World,
        me: SessionId,
        log: String,
    }

    impl Walker {
        fn new(room: u32, level: u16, class: u16) -> Self {
            let mut w = stock().world();
            w.classes
                .entry(ClassNo(class))
                .or_insert_with(|| blank::clear_class(ClassNo(class)));
            let me = enter(&mut w, "Kang", room, MessageLevel::High);
            let p = w.session_mut(me).unwrap().character.as_mut().unwrap();
            p.level = level;
            p.class = ClassNo(class);
            p.str = 60;
            p.nkeyslots = 10;
            p.maxwgt = 3000;
            p.hp = 100;
            p.maxhp = 100;
            Self {
                w,
                me,
                log: String::new(),
            }
        }

        /// Types `line` (with no command delay pending) and returns what came back.
        fn go(&mut self, line: &str) -> String {
            execute_input(&mut self.w, self.me, line, false);
            // the fast ticks carry out a queued move and run down the delay
            for _ in 0..20 {
                if self.w.session(self.me).unwrap().player().unwrap().delay == 0 {
                    break;
                }
                fast_update_delay(&mut self.w, self.me);
            }
            let palette = palette_of(&self.w, self.me);
            let s = self.w.session_mut(self.me).unwrap();
            let out = String::from_utf8_lossy(&s.output.render(palette, b"<P>")).into_owned();
            s.output.clear();
            self.log.push_str(&format!("> {line} [{}]\n", self.at()));
            out
        }

        /// Walks `dirs` (space-separated) and returns the room reached.
        fn walk(&mut self, dirs: &str) -> u32 {
            for d in dirs.split_whitespace() {
                let before = self.at();
                let out = self.go(d);
                assert_ne!(
                    self.at(),
                    before,
                    "{d} from {before}: {out:?}\n{}",
                    self.log
                );
            }
            self.at()
        }

        /// The key `item` handed over (the monster that drops it is not fought yet) and
        /// USE {key} {dir} stood in for in T-053: the Key exit's lock goes to closed, as the
        /// key leaves it; then OPEN.
        fn unlock_with(&mut self, item: i32, dir: Direction) {
            assert!(add_item_to_inventory(
                &mut self.w,
                self.me,
                ItemNo(item),
                -2
            ));
            let room = RoomKey::new(1, self.at());
            let exit = live_exit(&self.w, room, dir).unwrap();
            assert_eq!(
                (exit.kind, exit.par1, exit.par2),
                (ExitType::Key, item as u32, LOCK_LOCKED as i16)
            );
            set_par2(&mut self.w, room, dir, LOCK_CLOSED as i16);
            let out = self.go(&format!(
                "open {}",
                ["n", "s", "e", "w"][usize::from(dir.code())]
            ));
            assert_eq!(
                live_exit(&self.w, room, dir).unwrap().par2,
                LOCK_OPEN as i16,
                "{out:?}"
            );
        }

        fn at(&self) -> u32 {
            self.w.session(self.me).unwrap().player().unwrap().room.0
        }

        fn name(&self) -> String {
            self.w
                .room(RoomKey::new(1, self.at()))
                .unwrap()
                .name
                .clone()
        }
    }

    // Newhaven Docks (2149): `borrow skiff` is a Text exit (WCCMSG 8564) to the Small
    // Pier (2335), whose way back is an Action exit (T-026; m4_quest).
    #[test]
    fn the_newhaven_skiff() {
        let mut k = Walker::new(2149, 5, 1);
        let out = k.go("borrow skiff");
        assert_eq!(k.at(), 2335, "{out:?}");
        assert!(
            out.contains("You climb into one of the skiffs, and row to Silvermere."),
            "{out:?}"
        );
        let out = k.go("row skiff");
        assert_eq!(k.at(), 2335, "the pier has no Text exit: {out:?}");
        let out = k.go("n");
        assert_eq!(k.at(), 2149, "the Action exit back: {out:?} {}", k.log);
    }

    // The level-10 class quest in the Graveyard crypt (as played on the original under MBBSEmu), as
    // a level-10 Witchunter walks it over the stock rooms: the two levers are Remote Action
    // exits on the Hidden exit north of 1056 (two blocks, any order); `search s` finds the
    // way down to the second level; `push button` at the statue is a Remote Action exit on
    // 1250's own south exit; the third level's last room is a Level exit for level 10 only;
    // the mummy's bone key and the troll's iron key open Key exits; the clawed-hand glyph's
    // `push button` and the tomb's `go portal` are Text exits. Not carried yet, and stood
    // in for: killing the mummy and the troll (combat, T-034/T-036: the keys are handed
    // over) and USE {key} {dir} (T-053: the lock is set to closed, as `use` leaves it, then
    // `open`).
    #[test]
    fn the_crypt_quest() {
        let mut k = Walker::new(834, 10, 2);
        assert_eq!(k.name(), "Graveyard, Tomb Entrance");
        // Every door is locked again at start-up: OPEN refuses and
        // the door is bashed (or picked)
        let out = k.go("open n");
        assert!(out.contains("The door is locked."), "{out:?}");
        // A door that bursts open takes the basher through (BASH, T-026)
        for _ in 0..200 {
            if k.at() != 834 {
                break;
            }
            k.w.session_mut(k.me)
                .unwrap()
                .character
                .as_mut()
                .unwrap()
                .hp = 100;
            k.go("bash n");
        }
        assert_eq!(k.at(), 835);
        k.walk("d");
        assert_eq!(k.name(), "Crypt, Lower Level");
        // the first level: a lever at the bottom of each side corridor
        assert_eq!(k.walk("n n n n e e s s s s"), 1038);
        let out = k.go("pull lever");
        assert!(
            out.contains("You pull the lever. Off in the distance you hear a small click."),
            "{out:?}"
        );
        let hidden = RoomKey::new(1, 1056);
        let flags = |k: &Walker| {
            ExitFlags::from_bits_retain(live_exit(&k.w, hidden, Direction::North).unwrap().par1)
        };
        assert_eq!(
            flags(&k),
            ExitFlags::BLOCKED1,
            "the east lever takes block 2"
        );
        assert_eq!(k.walk("n n n n w w w w s s s s"), 1044);
        k.go("pull lever");
        assert_eq!(
            flags(&k),
            ExitFlags::VISBLOCKED,
            "both gone: the passage is open"
        );
        // north to the end, through the new passage and down
        assert_eq!(k.walk("n n n n e e n n n n n n n n"), 1056);
        assert_eq!(k.walk("n d"), 1070);
        assert_eq!(k.name(), "Crypt, Small Chamber");
        // the second level: search south in the arrival room
        let mut found = false;
        for _ in 0..40 {
            let out = k.go("search s");
            if k.go("s").is_empty() || k.at() != 1070 {
                found = true;
                break;
            }
            let _ = out;
        }
        assert!(found, "{}", k.log);
        assert_eq!(k.at(), 1237);
        assert_eq!(k.walk("se sw se se se sw sw sw"), 1250);
        let out = k.go("push button");
        assert!(out.contains("You push the button."), "{out:?}");
        assert_eq!(k.walk("s s s d n n n n e n n w n n n n"), 1297);
        assert_eq!(k.walk("n"), 1309);
        assert_eq!(k.name(), "Crypt, Sealed Tomb");
        // a level-11 character is turned away from it
        let mut late = Walker::new(1297, 11, 2);
        late.go("n");
        assert_eq!(late.at(), 1297);
        // the mummy's bone key opens the Key exit north
        k.unlock_with(177, Direction::North);
        assert_eq!(k.walk("n"), 1377);
        assert_eq!(k.name(), "Crypt, Shadowed Hall");
        // the hall's west exit is a Warrior's (Class exit 1): a Witchunter goes north
        // first, to the hall whose west exit is his (Class exit 2)
        k.go("w");
        assert_eq!(k.at(), 1377);
        assert_eq!(k.walk("n w w w w w w w w"), 2219);
        assert_eq!(k.name(), "Marble Room");
        // the clawed-hand glyph: south, south-west, `push button` (a Text exit)
        assert_eq!(k.walk("s sw"), 2223);
        let out = k.go("push button");
        assert_eq!(k.at(), 2224, "{out:?}");
        assert!(
            out.contains("The wall swivels rapidly around, moving you into another room!"),
            "{out:?}"
        );
        assert_eq!(k.name(), "Large Chamber");
        // the troll's iron key
        k.unlock_with(338, Direction::North);
        assert_eq!(k.walk("n n w w"), 2236);
        assert_eq!(k.name(), "Large Tomb");
        // the witchunter spirit and the mithril cutlass
        place(&mut k.w, 901, 85, "witchunter spirit", 2236);
        let out = k.go("greet witch");
        assert!(
            out.contains("The witchunter spirit greets you with a bow"),
            "{out:?}"
        );
        assert!(
            out.contains("seek the old one in the heart of the"),
            "{out:?}"
        );
        k.go("get mith");
        assert!(
            k.w.session(k.me)
                .unwrap()
                .player()
                .unwrap()
                .items
                .contains(&ItemNo(342)),
            "{}",
            k.log
        );
        let out = k.go("go portal");
        assert_eq!(k.at(), 834, "{out:?}");
        assert!(
            out.contains("You step through the shimmering portal."),
            "{out:?}"
        );
    }

    // ASK and GREET over stock monsters: the troll's talk block (`gullable:9261`,
    // `gullible:9261`, linking to 120), a wild dog's that is plain text (no lines, no link)
    // and a pirate ghost with none.
    #[test]
    fn ask_and_greet_stock_monsters() {
        let mut k = Walker::new(2224, 10, 2);
        place(&mut k.w, 902, 82, "troll", 2224);
        place(&mut k.w, 903, 50, "wild dog", 2224);
        place(&mut k.w, 904, 874, "pirate ghost", 2224);
        let gullible = k.go("ask troll are you gullible?");
        let mut refusals = String::new();
        for line in [
            "ask troll hello",
            "greet wild",
            "ask wild dog hello",
            "ask pirate hello",
            "greet pirate",
        ] {
            refusals.push_str(&format!("{line}: {:?}\n", k.go(line)));
        }
        insta::assert_snapshot!("monster_talk_refusals", refusals);
        let greet = k.go("greet troll");
        assert!(!greet.contains("nothing to tell"), "{greet:?}");
        assert!(!gullible.contains("nothing to tell"), "{gullible:?}");
        // the question is the rest of the line: "ask huge" style names keep their words
        let out = k.go("ask TROLL GULLABLE");
        assert_eq!(
            out,
            gullible.replace("are you gullible?", ""),
            "upper case matches too"
        );
        // no monster by that name: not ASK at all (the line goes on to speech)
        let out = k.go("ask nobody anything");
        assert!(!out.contains("nothing to tell"), "{out:?}");
    }

    // A WCCTEXT text shown with display_long_text: the witchunter
    // spirit's greeting, its %s the monster's name, wrapped at 79 columns.
    #[test]
    fn a_text_block_render() {
        let mut k = Walker::new(2236, 10, 2);
        place(&mut k.w, 901, 85, "witchunter spirit", 2236);
        let out = k.go("greet witchunter");
        insta::assert_snapshot!("greet_witchunter_spirit", out.replace('\r', ""));
    }

    // The random monster text: a dark cultist's stock random text
    // (block 9108, "100 The fanatic screams ...") on the slow cycle, to the whole room.
    #[test]
    fn a_monster_with_random_text_speaks_on_the_slow_cycle() {
        let mut k = Walker::new(2224, 10, 2);
        place(&mut k.w, 905, 29, "dark cultist", 2224);
        k.w.cycles.order = vec![k.me];
        k.w.rng = SeededRng::new(5);
        perform_random_event(&mut k.w, 10);
        let palette = palette_of(&k.w, k.me);
        let s = k.w.session_mut(k.me).unwrap();
        let out = String::from_utf8_lossy(&s.output.render(palette, b"<P>")).into_owned();
        assert_eq!(
            out,
            "\u{1b}[79D\u{1b}[KThe fanatic screams \"Death to those who oppose the Blood God!\"\r\n<P>"
        );
    }

    // T-054 Done-when: the quest flags (`qflags`, `qflags2`) go
    // out with the saved record and come back on load. (No script clause changes them:
    // see `kw_flag`.)
    #[test]
    fn quest_flags_survive_save_and_load() {
        use silvermere_engine::player::PLAYER_VERSION;
        use silvermere_engine::player::load_player;

        let mut w = stock().world();
        let id = enter(&mut w, "Kang", 2224, MessageLevel::High);
        {
            let s = w.session_mut(id).unwrap();
            s.account = Some(AccountRef {
                id: AccountId(1),
                username: "USER1".into(),
                sysop: false,
                gameop: false,
                sex: b'M',
            });
            s.created = true;
            let c = s.character.as_mut().unwrap();
            c.settings |= PlayerSettings::VALIDATED;
            c.version = PLAYER_VERSION;
            c.qflags = 0x8000_0005;
            c.qflags2 = 0x0001_0040;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let mut engine = silvermere_engine::Engine::new(
            w,
            super::common::TestClock::new(),
            SeededRng::new(1),
            tx,
        );
        assert!(save_player(&mut engine.world, id));
        engine.flush();
        let saved = rx
            .try_iter()
            .find_map(|o| match o {
                EngineOutput::Persist(PersistRequest::Character { character, .. }) => {
                    Some(*character)
                }
                _ => None,
            })
            .expect("the record was saved");
        assert_eq!((saved.qflags, saved.qflags2), (0x8000_0005, 0x0001_0040));
        let mut w = stock().world();
        let id = enter(&mut w, "Kang", 2224, MessageLevel::High);
        load_player(&mut w, id, saved, 0).unwrap();
        let c = w.session(id).unwrap().player().unwrap();
        assert_eq!((c.qflags, c.qflags2), (0x8000_0005, 0x0001_0040));
    }
}
