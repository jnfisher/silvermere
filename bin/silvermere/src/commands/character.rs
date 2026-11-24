//! `character create`: a finished character for an existing account, made outside the
//! game (`silvermere_engine::player::make_character`), for test and staff characters.
//!
//! It writes the `characters` table, so it takes the store's single-writer guard and
//! refuses while a server runs: a running server would not see the row until the
//! account's next login, and a character it holds would be saved over it.

use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Args, Subcommand};
use silvermere_auth::account::AccountStore as _;
use silvermere_db::{Db, TryAcquire, WriterGuard};
use silvermere_engine::World;
use silvermere_engine::player::make::{CharacterSpec, STAT_NAMES, make_character};
use silvermere_types::ids::{ClassNo, ItemNo, RaceNo};

use crate::config::Config;

/// `character` subcommands.
#[derive(Debug, Subcommand)]
pub enum CharacterCmd {
    /// Make a finished character for an account (stop the server first).
    Create(CreateArgs),
}

/// `character create` flags.
#[derive(Debug, Args)]
pub struct CreateArgs {
    /// The account (made first with `silvermere account create`).
    pub account: String,
    /// First name: letters only, at most 10.
    #[arg(long)]
    pub name: String,
    /// Last name: letters only, at most 18.
    #[arg(long, default_value = "")]
    pub last: String,
    /// Race, by name or number.
    #[arg(long)]
    pub race: String,
    /// Class, by name or number.
    #[arg(long)]
    pub class: String,
    /// Level, 1 to 255; the experience is set to what reaches it.
    #[arg(long, default_value_t = 1)]
    pub level: u16,
    /// A stat, as `strength=90` (also intellect, willpower, agility, health, charm, or
    /// their first three letters); a stat not given is the race's maximum.
    #[arg(long = "stat", value_name = "STAT=N")]
    pub stats: Vec<String>,
    /// An item by exact name or number, put in the pack and worn or wielded when it can
    /// be; repeat it for more. Limited items are refused.
    #[arg(long = "item", value_name = "ITEM")]
    pub items: Vec<String>,
    /// Gold crowns in hand.
    #[arg(long, default_value_t = 0)]
    pub gold: u32,
    /// Replace the account's character if it has one.
    #[arg(long)]
    pub replace: bool,
}

pub fn run(cfg: &Config, action: &CharacterCmd) -> anyhow::Result<ExitCode> {
    let CharacterCmd::Create(args) = action;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        super::ensure_sqlite_dir(cfg)?;
        let guard = match WriterGuard::try_acquire(&cfg.database.url).await? {
            TryAcquire::Taken(guard) => guard,
            TryAcquire::Held(by) => bail!(
                "a server is running on this store ({by}); stop it first, since it would save \
                 its own copy of the character over this one"
            ),
        };
        let db = Db::connect_and_migrate(&cfg.database.url)
            .await
            .with_context(|| format!("opening {}", cfg.database_shown()))?;
        let result = create(cfg, &db, args).await;
        db.close().await;
        guard.release().await;
        result
    })
}

async fn create(cfg: &Config, db: &Db, args: &CreateArgs) -> anyhow::Result<ExitCode> {
    let Some(account) = db.by_username(&args.account).await? else {
        bail!(
            "no account named {}; make it first with `silvermere account create {}`",
            args.account,
            args.account
        );
    };
    if let Some(row) = db.characters().by_account(account.id).await?
        && !args.replace
    {
        bail!(
            "{} already has the character {}; pass --replace to replace it",
            account.username,
            row.character.firstname
        );
    }
    if let Some(row) = db.characters().by_first_name(args.name.trim()).await?
        && row.account != account.id
    {
        bail!(
            "another account's character is named {}",
            row.character.firstname
        );
    }
    let rooms: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rooms")
        .fetch_one(&db.pool())
        .await
        .context("counting rooms")?;
    if rooms == 0 {
        bail!("this store has no world yet; run `silvermere setup` or `silvermere import` first");
    }

    let snapshot = crate::bridge::load_snapshot(db).await?;
    let max_sessions = u16::try_from(cfg.server.max_sessions).unwrap_or(u16::MAX);
    let mut world = World::load(cfg.game.clone(), max_sessions, snapshot)?;
    let spec = CharacterSpec {
        firstname: args.name.clone(),
        lastname: args.last.clone(),
        race: RaceNo(pick(
            "race",
            &args.race,
            world.races.values().map(|r| (r.number.0, r.name.as_str())),
        )?),
        class: ClassNo(pick(
            "class",
            &args.class,
            world
                .classes
                .values()
                .map(|c| (c.number.0, c.name.as_str())),
        )?),
        sex: account.sex.as_byte(),
        level: args.level,
        stats: stats(&args.stats)?,
        items: args
            .items
            .iter()
            .map(|i| {
                pick(
                    "item",
                    i,
                    world.items.values().map(|t| (t.number.0, t.name.as_str())),
                )
                .map(ItemNo)
            })
            .collect::<anyhow::Result<_>>()?,
        gold: args.gold,
    };
    let today = silvermere_engine::event_loop::day_start(jiff::Timestamp::now().as_second());
    let made = make_character(&mut world, account.id, &account.username, &spec, today)?;

    db.delete_characters(account.id).await?;
    db.characters().insert(account.id, &made.character).await?;
    let c = &made.character;
    let class = world.classes.get(&c.class).map_or("", |k| k.name.as_str());
    let race = world.races.get(&c.race).map_or("", |r| r.name.as_str());
    println!(
        "made {} {} for {}: level {} {race} {class}, {} hit points, {} mana",
        c.firstname, c.lastname, account.username, c.level, c.maxhp, c.maxmana
    );
    println!(
        "  strength {}, intellect {}, willpower {}, agility {}, health {}, charm {}",
        c.str, c.intel, c.wil, c.agi, c.hea, c.cha
    );
    for line in &made.items {
        println!("  {line}");
    }
    Ok(ExitCode::SUCCESS)
}

/// A record by number, or by name ignoring case: an exact name first, then the only
/// name that starts with `want`.
fn pick<'a, N: Copy + std::fmt::Display + std::str::FromStr + PartialEq>(
    what: &str,
    want: &str,
    all: impl Iterator<Item = (N, &'a str)>,
) -> anyhow::Result<N> {
    let all: Vec<(N, &str)> = all.collect();
    if let Ok(n) = want.trim().parse::<N>() {
        if all.iter().any(|&(k, _)| k == n) {
            return Ok(n);
        }
        bail!("no {what} number {n}");
    }
    let want = want.trim();
    if let Some(&(n, _)) = all.iter().find(|(_, name)| name.eq_ignore_ascii_case(want)) {
        return Ok(n);
    }
    let lower = want.to_ascii_lowercase();
    let starts: Vec<_> = all
        .iter()
        .filter(|(_, name)| name.to_ascii_lowercase().starts_with(&lower))
        .collect();
    match starts.as_slice() {
        [(n, _)] => Ok(*n),
        [] => bail!("no {what} named {want}"),
        many => bail!(
            "{want} could be more than one {what}: {}",
            many.iter()
                .take(8)
                .map(|(n, name)| format!("{name} ({n})"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// `--stat` values in [`STAT_NAMES`] order.
fn stats(given: &[String]) -> anyhow::Result<[Option<u16>; 6]> {
    let mut out = [None; 6];
    for s in given {
        let Some((key, value)) = s.split_once('=') else {
            bail!("--stat takes STAT=N, got {s}");
        };
        let key = key.trim().to_ascii_lowercase();
        let i = STAT_NAMES
            .iter()
            .position(|n| key.len() >= 3 && n.starts_with(&key))
            .with_context(|| format!("no stat {key}; one of {}", STAT_NAMES.join(", ")))?;
        out[i] = Some(
            value
                .trim()
                .parse()
                .with_context(|| format!("--stat {s}: not a number"))?,
        );
    }
    Ok(out)
}
