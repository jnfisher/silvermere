//! `SysopMenu(..)`: the `[MAJORMUD Sysop]` menu, its prompts and its jobs.
//!
//! The jobs were polling routines, one record per poll: clearing the kept evil points
//! (`C`) and the profiles (`CP`), the defunct-user check (`D`) and the hoarder check
//! (`H`). Here a job is per-session state that [`on_tick`] advances one record a tick,
//! reporting the same progress lines (`Clearing:  ` and the spinner); `X` cancels it as
//! it did. `D`, `H` and REPORT USERS need every stored character: the engine asks `bin`
//! (`EngineOutput::Roster`), and [`on_roster`] receives them.
//!
//! Not carried (`docs/differences.md`): `U` (an update file: the importer CLI), `B` and
//! `E` (the kernel's editors), `A` (add-on codes) and their states;
//! the sysop status line (a `devmode` console line).

use std::collections::VecDeque;

use silvermere_text::Msg;
use silvermere_types::enums::{SessionState, SysopStep};
use silvermere_types::ids::{AccountId, ItemNo, SessionId};
use silvermere_types::records::Character;

use crate::commands::sysop::{audit, leading_int};
use crate::event_loop::{EngineOutput, PersistRequest};
use crate::state::login::{key, show_menu};
use crate::state::{Input, Outcome};
use crate::world::World;

/// A running menu job.
#[derive(Debug, Clone)]
pub enum Job {
    /// `C`: the accounts whose kept evil points are still to clear.
    ClearEvil(VecDeque<AccountId>),
    /// `CP`: the accounts whose profiles are still to clear.
    ClearProfiles(VecDeque<AccountId>),
    /// `D` (`items` false) or `H` (`items` true).
    Xref(Box<Xref>),
}

/// A user check's place: the user being checked and how far the cross-reference has got.
#[derive(Debug, Clone, Default)]
pub struct Xref {
    /// `H`: the hoarder check; else `D`.
    pub items: bool,
    /// The stored characters still to look at, by user-id; `None` until they arrive.
    pub rows: Option<VecDeque<(AccountId, Character)>>,
    /// The character the last question was about.
    pub current: Option<(AccountId, Character)>,
    /// A question is out.
    pub asked: bool,
    /// The answer was `Y`.
    pub yes: bool,
}

fn push(world: &mut World, id: SessionId, msg: Msg) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
}

fn set_state(world: &mut World, id: SessionId, state: SessionState) {
    if let Some(s) = world.session_mut(id) {
        s.state = state;
    }
}

/// The hidden word `sysop` of the main menu, for the sysop role: the menu and its prompt.
pub fn open(world: &mut World, id: SessionId) {
    set_state(world, id, SessionState::SysopMenu(SysopStep::Menu));
    push(world, id, Msg::SysopMenu);
}

/// One input line in a state this module owns.
pub fn on_line(world: &mut World, input: Input<'_>) -> Outcome {
    let id = input.id;
    let Some(state) = world.session(id).map(|s| s.state) else {
        return Outcome::Logoff;
    };
    let words: Vec<&str> = input.line.split_whitespace().collect();
    let one_x = words.len() == 1 && words[0].starts_with(['x', 'X']);
    let yes = words.first().is_some_and(|w| w.starts_with(['y', 'Y']));
    let SessionState::SysopMenu(step) = state else {
        return Outcome::Unimplemented(state);
    };
    match step {
        SysopStep::Menu => menu_line(world, id, input.line),
        // X cancels; anything else shows the progress text
        SysopStep::Clearing => {
            if one_x {
                world.sysop.jobs.remove(&id);
                push(world, id, Msg::SysopCancelled);
                set_state(world, id, SessionState::SysopMenu(SysopStep::Menu));
            } else {
                push(world, id, Msg::SysopClearing);
            }
        }
        SysopStep::XrefUsers => push(world, id, Msg::SysopCheckingDeletedCr),
        SysopStep::XrefItems => push(world, id, Msg::SysopCheckingItemsCr),
        // Y or not, X ends the check
        SysopStep::XrefStrip | SysopStep::XrefDelete => {
            if let Some(Job::Xref(x)) = world.sysop.jobs.get_mut(&id) {
                x.yes = yes;
            }
            if one_x {
                cancel_check(world, id);
            } else {
                push(world, id, Msg::SysopSpace);
                let next = if step == SysopStep::XrefStrip {
                    SysopStep::XrefItems
                } else {
                    SysopStep::XrefUsers
                };
                set_state(world, id, SessionState::SysopMenu(next));
            }
        }
        // the three purge limits; a number above 0 replaces the limit
        SysopStep::PurgeMinutes => {
            if let Some(n) = positive(&words) {
                world.sysop.purge_mins = n;
            }
            let n = i64::from(world.sysop.purge_mins);
            push(world, id, Msg::SysopPurgeMinutes { n });
            set_state(world, id, SessionState::SysopMenu(SysopStep::PurgeDays));
        }
        SysopStep::PurgeDays => {
            if let Some(n) = positive(&words) {
                world.sysop.purge_days = n;
            }
            let n = i64::from(world.sysop.purge_days);
            push(world, id, Msg::SysopPurgeDays { n });
            set_state(world, id, SessionState::SysopMenu(SysopStep::PurgeExp));
        }
        SysopStep::PurgeExp => {
            if let Some(n) = positive(&words) {
                world.sysop.purge_exp = n;
            }
            let n = i64::from(world.sysop.purge_exp);
            push(world, id, Msg::SysopPurgeExp { n });
            push(world, id, Msg::SysopMenuPromptBare);
            set_state(world, id, SessionState::SysopMenu(SysopStep::Menu));
        }
        // the update states (`U`) and the reindexing state (`R`) are never entered
        SysopStep::UpdateFile | SysopStep::Updating | SysopStep::Reindexing => {
            push(world, id, Msg::SysopMenuPromptCr);
            set_state(world, id, SessionState::SysopMenu(SysopStep::Menu));
        }
    }
    Outcome::Continue
}

fn positive(words: &[&str]) -> Option<i32> {
    let n = leading_int(words.first()?);
    (n > 0).then(|| n.min(i64::from(i32::MAX)) as i32)
}

/// A check cancelled: the check key reset, back to the sysop menu.
fn cancel_check(world: &mut World, id: SessionId) {
    if let Some(Job::Xref(x)) = world.sysop.jobs.remove(&id)
        && x.items
    {
        world.sysop.item_check_running = false;
    }
    push(world, id, Msg::SysopCancelledChecking);
    set_state(world, id, SessionState::SysopMenu(SysopStep::Menu));
}

/// `SysopStep::Menu`: one letter a job.
fn menu_line(world: &mut World, id: SessionId, line: &str) {
    let word = line.split_whitespace().next().unwrap_or("");
    match key(line) {
        None => push(world, id, Msg::SysopMenu),
        Some('X') => {
            set_state(world, id, SessionState::MainMenu);
            show_menu(world, id);
        }
        Some('R') => {
            world.sysop.need_recov = true;
            push(world, id, Msg::SysopObsoleteReindex);
        }
        Some('D') => begin_user_xref(world, id, false),
        Some('H') => begin_user_xref(world, id, true),
        Some('L') => {
            let (mins, days, exp) = (
                i64::from(world.sysop.purge_mins),
                i64::from(world.sysop.purge_days),
                i64::from(world.sysop.purge_exp),
            );
            push(world, id, Msg::SysopPurgeLimits { mins, days, exp });
            set_state(world, id, SessionState::SysopMenu(SysopStep::PurgeMinutes));
        }
        Some(c @ ('I' | 'N' | 'S')) => {
            // the file as the host's paged listing, then the main menu's prompt, and the
            // main menu.
            let file = match c {
                'I' => "WCCMMUD.RLN",
                'N' => "WCCMMSYS.REL",
                _ => "WCCMMSYS.NOT",
            };
            let text = world
                .display
                .assets
                .get(file)
                .map(silvermere_text::render::normalise_line_ends);
            if let Some(s) = world.session_mut(id) {
                if let Some(bytes) = text {
                    s.output.push_raw(bytes);
                }
                s.output.push(Msg::MudMenuPromptCr);
                s.state = SessionState::MainMenu;
            }
        }
        Some('C') => {
            // C, or CP for the profiles
            let profiles = word
                .as_bytes()
                .get(1)
                .is_some_and(|b| b.eq_ignore_ascii_case(&b'P'));
            let mut queue: Vec<AccountId> = if profiles {
                world.profiles.keys().copied().collect()
            } else {
                world.evil_points.keys().copied().collect()
            };
            queue.sort_by_key(|a| a.0);
            if queue.is_empty() {
                push(world, id, Msg::SysopClearingError);
            } else {
                let queue = VecDeque::from(queue);
                world.sysop.jobs.insert(
                    id,
                    if profiles {
                        Job::ClearProfiles(queue)
                    } else {
                        Job::ClearEvil(queue)
                    },
                );
                push(world, id, Msg::SysopClearing);
                set_state(world, id, SessionState::SysopMenu(SysopStep::Clearing));
            }
        }
        // anything else, and the dropped U, B, E, A: the prompt again
        Some(_) => push(world, id, Msg::SysopMenuPromptCr),
    }
}

/// The defunct-user check (`items` false) or the hoarder check: the check starts from the
/// first character once `bin` sends them.
fn begin_user_xref(world: &mut World, id: SessionId, items: bool) {
    if items {
        set_state(world, id, SessionState::SysopMenu(SysopStep::XrefItems));
        push(world, id, Msg::SysopCheckingItems);
        if world.sysop.item_check_running {
            push(world, id, Msg::SysopXrefAnother);
        }
        world.sysop.item_check_running = true;
    } else {
        set_state(world, id, SessionState::SysopMenu(SysopStep::XrefUsers));
        push(world, id, Msg::SysopCheckingDeleted);
    }
    world.sysop.jobs.insert(
        id,
        Job::Xref(Box::new(Xref {
            items,
            ..Xref::default()
        })),
    );
    world.outbox.push(EngineOutput::Roster { id });
}

/// The answer to `EngineOutput::Roster`: every stored character with its account. A
/// running user check gets them in user-id order (key 0 of WCCUSERS), a paged report in
/// experience order.
pub fn on_roster(world: &mut World, id: SessionId, chars: Vec<(AccountId, Character)>) {
    // T-088: the records queued since the store read stand for their rows
    let mut chars = crate::player::with_queued(world, chars);
    if let Some(r) = world.sysop.reports.get_mut(&id) {
        r.rows = Some(crate::display::sysop::report_order(chars));
        return;
    }
    if let Some(Job::Xref(x)) = world.sysop.jobs.get_mut(&id) {
        chars.sort_by(|a, b| {
            a.1.userid
                .to_ascii_uppercase()
                .cmp(&b.1.userid.to_ascii_uppercase())
        });
        x.rows = Some(chars.into());
    }
}

/// The next spinner frame.
fn spinner(world: &mut World, id: SessionId) {
    let c = ['-', '\\', '|', '/'][usize::from(world.sysop.spinner % 4)];
    world.sysop.spinner = (world.sysop.spinner + 1) % 4;
    push(world, id, Msg::SysopSpinner { c });
}

/// One poll of every sysop job and paged report: the per-poll routines.
pub fn on_tick(world: &mut World) {
    let ids: Vec<SessionId> = world.sysop.jobs.keys().copied().collect();
    for id in ids {
        if world.session(id).is_none() {
            world.sysop.jobs.remove(&id);
            continue;
        }
        step_job(world, id);
    }
    let ids: Vec<SessionId> = world.sysop.reports.keys().copied().collect();
    for id in ids {
        if world
            .session(id)
            .is_none_or(|s| s.state != SessionState::UserReport)
        {
            world.sysop.reports.remove(&id);
            continue;
        }
        // the whole remaining list in one tick, as the polls between two ticks would
        while crate::display::sysop::next_reported_user(world, id) {}
    }
    let ids: Vec<(SessionId, (u8, SessionId))> =
        world.sysop.info.iter().map(|(k, v)| (*k, *v)).collect();
    for (id, (part, shown)) in ids {
        god_info_part(world, id, part, shown);
    }
}

/// The GOD user INFO display, one part a poll: the spells and the powers, the profile,
/// the abilities, then the prompt. The bank books of part 0 are not shown:
/// `display_users_bankbooks` writes to its own player (TODO(T-042)).
fn god_info_part(world: &mut World, id: SessionId, part: u8, shown: SessionId) {
    use silvermere_types::enums::CommandCode as Cmd;
    match part {
        0 => {
            crate::display::spells::display_users_spells(world, shown, id, Cmd::Spells);
            crate::display::spells::display_users_spells(world, shown, id, Cmd::Powers);
        }
        1 => crate::display::profile::display_profile_of(world, shown, id),
        2 => crate::display::sysop::display_user_abilities(world, shown, id),
        _ => {
            if let Some(s) = world.session_mut(id) {
                s.push_prompt();
            }
            world.sysop.info.remove(&id);
            return;
        }
    }
    world.sysop.info.insert(id, (part + 1, shown));
}

fn step_job(world: &mut World, id: SessionId) {
    let Some(job) = world.sysop.jobs.get(&id).cloned() else {
        return;
    };
    match job {
        Job::ClearEvil(mut q) | Job::ClearProfiles(mut q) => {
            let profiles = matches!(world.sysop.jobs.get(&id), Some(Job::ClearProfiles(_)));
            spinner(world, id);
            match q.pop_front() {
                Some(account) => {
                    let userid = if profiles {
                        world.profiles.remove(&account).map(|r| r.userid)
                    } else {
                        world.evil_points.remove(&account).map(|r| r.userid)
                    }
                    .unwrap_or_default();
                    world.outbox.push(EngineOutput::Persist(if profiles {
                        PersistRequest::DeleteProfile(account)
                    } else {
                        PersistRequest::DeleteEvilPoints(account)
                    }));
                    push(
                        world,
                        id,
                        if profiles {
                            Msg::SysopClearedProfile {
                                userid: userid.clone(),
                            }
                        } else {
                            Msg::SysopClearedEvil {
                                userid: userid.clone(),
                            }
                        },
                    );
                    audit(
                        world,
                        id,
                        if profiles {
                            "sysop.clear_profile"
                        } else {
                            "sysop.clear_evil"
                        },
                        userid,
                    );
                    world.sysop.jobs.insert(
                        id,
                        if profiles {
                            Job::ClearProfiles(q)
                        } else {
                            Job::ClearEvil(q)
                        },
                    );
                }
                None => {
                    world.sysop.jobs.remove(&id);
                    push(world, id, Msg::SysopJobDone);
                    set_state(world, id, SessionState::SysopMenu(SysopStep::Menu));
                }
            }
        }
        Job::Xref(x) => step_xref(world, id, x),
    }
}

/// One step of the defunct-user check or the hoarder check.
fn step_xref(world: &mut World, id: SessionId, mut x: Box<Xref>) {
    let want = if x.items {
        SysopStep::XrefItems
    } else {
        SysopStep::XrefUsers
    };
    if world.session(id).map(|s| s.state) != Some(SessionState::SysopMenu(want)) {
        return;
    }
    let Some(mut rows) = x.rows.take() else {
        return;
    };
    // the answer to the last question
    if x.asked {
        x.asked = false;
        if x.yes
            && let Some((account, plr)) = x.current.take()
        {
            if x.items {
                strip_limited(world, id, account, plr);
            } else {
                push(
                    world,
                    id,
                    Msg::SysopXrefDeleting {
                        userid: plr.userid.clone(),
                    },
                );
                // deleting an offline user: a player online keeps his record
                let online = world.session_ids().into_iter().any(|s| {
                    world
                        .session(s)
                        .and_then(|s| s.account.as_ref())
                        .is_some_and(|a| a.id == account)
                });
                if !online {
                    crate::player::queue_character_delete(world, account);
                    audit(world, id, "sysop.delete_user", plr.userid.clone());
                }
            }
        }
    }
    spinner(world, id);
    // Each step takes the next record as it stands now: a record queued since the store's
    // read stands for its row, and a queued deletion skips it (T-088)
    let next = std::iter::from_fn(|| rows.pop_front()).find_map(|(account, plr)| {
        crate::player::queued_or_stored(world, account, Some(plr)).map(|c| (account, c))
    });
    let Some((account, plr)) = next else {
        world.sysop.jobs.remove(&id);
        if x.items {
            world.sysop.item_check_running = false;
        }
        push(world, id, Msg::SysopXrefDone);
        set_state(world, id, SessionState::SysopMenu(SysopStep::Menu));
        return;
    };
    let today = world.today;
    let days = (today - plr.lastdate).div_euclid(86_400);
    let last = crate::display::sysop::format_date(plr.lastdate);
    if x.items {
        let limited: Vec<(String, i64)> = plr
            .items
            .iter()
            .chain(plr.keys.iter())
            .filter(|n| n.0 != 0)
            .filter_map(|n| world.items.get(n))
            .filter(|i| i.limit != 0)
            .map(|i| (i.name.clone(), i64::from(i.limit)))
            .collect();
        for (item, limit) in &limited {
            push(
                world,
                id,
                Msg::SysopXrefItem {
                    userid: plr.userid.clone(),
                    item: item.clone(),
                    limit: *limit,
                },
            );
        }
        if !limited.is_empty() {
            let race = world
                .races
                .get(&plr.race)
                .map_or_else(String::new, |r| r.name.clone());
            let class = world
                .classes
                .get(&plr.class)
                .map_or_else(String::new, |c| c.name.clone());
            push(
                world,
                id,
                Msg::SysopXrefHoarder {
                    userid: plr.userid.clone(),
                    name: plr.firstname.clone(),
                    race,
                    class,
                    evil: i64::from(plr.evil),
                    mins: i64::from(plr.lifemins),
                    last,
                    days,
                    exp: i64::from(plr.exp),
                },
            );
            set_state(world, id, SessionState::SysopMenu(SysopStep::XrefStrip));
            x.asked = true;
            x.yes = false;
            x.current = Some((account, plr));
        }
    } else {
        let s = &world.sysop;
        let defunct = i64::from(s.purge_mins) > i64::from(plr.lifemins)
            || days > i64::from(s.purge_days)
            || i64::from(s.purge_exp) > i64::from(plr.exp);
        if defunct {
            push(
                world,
                id,
                Msg::SysopXrefDefunct {
                    userid: plr.userid.clone(),
                    name: plr.firstname.clone(),
                    mins: i64::from(plr.lifemins),
                    last,
                    days,
                    exp: i64::from(plr.exp),
                },
            );
            set_state(world, id, SessionState::SysopMenu(SysopStep::XrefDelete));
            x.asked = true;
            x.current = Some((account, plr));
        }
    }
    x.rows = Some(rows);
    world.sysop.jobs.insert(id, Job::Xref(x));
}

/// Every limited item of an offline character taken away (slot emptied, weight off,
/// unworn and unwielded, its copy released), the record saved.
fn strip_limited(world: &mut World, id: SessionId, account: AccountId, plr: Character) {
    let online = world.session_ids().into_iter().any(|s| {
        world
            .session(s)
            .and_then(|s| s.player())
            .is_some_and(|p| p.userid == plr.userid)
    });
    if online {
        push(world, id, Msg::SysopXrefOnline { userid: plr.userid });
        return;
    }
    push(
        world,
        id,
        Msg::SysopXrefRemoving {
            userid: plr.userid.clone(),
        },
    );
    // Nothing is done if the character is gone; a write queued since the row was read
    // stands for it (T-088)
    let Some(mut plr) = crate::player::queued_or_stored(world, account, Some(plr)) else {
        return;
    };
    let limited = |w: &World, n: ItemNo| w.items.get(&n).filter(|i| i.limit != 0).cloned();
    let mut released = Vec::new();
    // the pack (an empty slot's uses -1), then the key ring (-2)
    for empty in [-1i16, -2] {
        let (list, uses) = if empty == -1 {
            (&mut plr.items, &mut plr.itemuses)
        } else {
            (&mut plr.keys, &mut plr.keyuses)
        };
        for (i, slot) in list.iter_mut().enumerate() {
            let n = *slot;
            if n.0 == 0 {
                continue;
            }
            let Some(itm) = limited(world, n) else {
                continue;
            };
            plr.wgt = plr.wgt.saturating_sub(itm.weight as u16);
            *slot = ItemNo(0);
            if let Some(u) = uses.get_mut(i) {
                *u = empty;
            }
            for w in plr.worn.iter_mut().filter(|w| **w == n) {
                *w = ItemNo(0);
            }
            if empty == -1 && plr.weapon == n {
                plr.weapon = ItemNo(0);
            }
            released.push(n);
        }
    }
    for n in released {
        crate::items::limited::release(world, n, crate::items::limited::Holder::User(&plr.userid));
    }
    audit(world, id, "sysop.strip_limited", plr.userid.clone());
    crate::player::queue_character(world, account, Box::new(plr));
}
