//! [`handle_commands`]: one typed game command to its `cmd_*` handler.
//!
//! [`handle_commands`] parses the first word; [`dispatch`] then runs the delay safeguard, the
//! mortally-wounded / paralysed gate with the original's exact allowed list, the colour reset,
//! then one `match` arm per [`Cmd`] calling the family module that owns the handler
//! (`commands/mod.rs` names the owners). The small arms (the argument-count checks of the display
//! commands, SAVE's refusal, the display modes, VERSION, TALK, STATLINE, PAGE, ACTION on/off,
//! BREAK's flags) are here or in `misc`; everything else calls the family module that owns the
//! routine.
//! The result is a [`Handled`]: not a command (or the command has no routine), and
//! `execute_input` goes on to exits, spells and speech; handled, and the prompt follows; or
//! the player must still wait out a delay, with no prompt.

use silvermere_text::Msg;
use silvermere_types::flags::{PlayerStatus, PlayerStatus2};
use silvermere_types::ids::SessionId;

use super::parse::{CommandLine, eq_ignore_case, parse_command};
use super::{
    Cmd, ask_greet, bank, combat, comms, gang, group, info, inventory, misc, movement, settings,
    shop, sysop, training, use_item,
};
use crate::world::World;

/// What [`handle_commands`] answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    /// Not a game command (or no routine); `execute_input` goes on.
    NotCommand,
    /// Handled, the prompt follows.
    Done,
    /// A command delay is still running; no prompt.
    Wait,
}

impl Handled {
    /// From a handler's yes/no answer: handled, or not a command.
    pub fn from_rc(rc: bool) -> Self {
        if rc { Self::Done } else { Self::NotCommand }
    }
}

/// The commands a mortally wounded (`hp <= 0`) or paralysed player may still use
/// Every other code, JOIN included, gets the refusal.
pub const WOUNDED_ALLOWED: [Cmd; 24] = [
    Cmd::Help,
    Cmd::Save,
    Cmd::Look,
    Cmd::Sound,
    Cmd::Status,
    Cmd::Health,
    Cmd::Inventory,
    Cmd::Keys,
    Cmd::Whisper,
    Cmd::Broadcast,
    Cmd::Suicide,
    Cmd::Reroll,
    Cmd::Identity,
    Cmd::Topten,
    Cmd::Quiet,
    Cmd::Verbose,
    Cmd::Scan,
    Cmd::Brief,
    Cmd::StatLine,
    Cmd::Profile,
    Cmd::Sysop,
    Cmd::Experience,
    Cmd::Feedback,
    Cmd::Global,
];

/// An empty line is no command; the first word goes through `parse_command` and a real command to
/// [`dispatch`].
pub fn handle_commands(world: &mut World, id: SessionId, line: &CommandLine) -> Handled {
    if line.argc() == 0 {
        return Handled::NotCommand;
    }
    match parse_command(line.first()) {
        Some(cmd) => dispatch(world, id, cmd, line),
        None => Handled::NotCommand,
    }
}

/// [`handle_commands`] after the parse: the delay safeguard, the wounded gate, the colour
/// reset and the switch.
pub fn dispatch(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = world.session(id).and_then(|s| s.character.as_ref()) else {
        return Handled::NotCommand;
    };
    if plr.delay > 0 {
        return Handled::Wait;
    }
    let wounded = plr.hp <= 0 || plr.status.contains(PlayerStatus::PARALYSED);
    let Some(s) = world.session_mut(id) else {
        return Handled::NotCommand;
    };
    if wounded && !WOUNDED_ALLOWED.contains(&cmd) {
        s.output.push(Msg::MortallyWounded);
        return Handled::Done;
    }
    let normal_at = s.output.items().len();
    s.output.push(Msg::CmdNormal);
    crate::metrics::command(cmd);
    // the owner of the prf buffer for the `tell_*` family while the command runs
    let outer = world.prf_owner.replace(id);
    let handled = run(world, id, cmd, line);
    world.prf_owner = outer;
    // The colour reset sits in the output buffer and goes out with the command's first send. It
    // never reaches the player when the buffer is dropped or sent elsewhere: a word that is not a
    // command (the text exits, `borrow skiff`) and a queued move (its `tell_room` for the others
    // clears the buffer; the room it shows brings its own). The room-walk capture has neither in
    // the first bytes of those commands. SEARCH keeps or loses it on its own paths
    // (`movement::search`).
    let queued_move = handled == Handled::Wait
        && matches!(
            cmd,
            Cmd::North
                | Cmd::South
                | Cmd::East
                | Cmd::West
                | Cmd::NorthEast
                | Cmd::NorthWest
                | Cmd::SouthEast
                | Cmd::SouthWest
                | Cmd::Up
                | Cmd::Down
        );
    if (handled == Handled::NotCommand || queued_move)
        && let Some(s) = world.session_mut(id)
    {
        s.output.remove_msg_at(normal_at, &Msg::CmdNormal);
    }
    handled
}

/// The command switch of [`handle_commands`].
fn run(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    match cmd {
        Cmd::None | Cmd::Unknown(_) => Handled::NotCommand,
        Cmd::Quit => misc::cmd_quit(world, id, cmd, line),
        // the attack commands (combat::cmd_any_attack)
        Cmd::Punch => combat::cmd_punch(world, id, cmd, line),
        Cmd::Kick => combat::cmd_kick(world, id, cmd, line),
        Cmd::Jumpkick => combat::cmd_jumpkick(world, id, cmd, line),
        Cmd::Bash => combat::cmd_bash(world, id, cmd, line),
        Cmd::Smash => combat::cmd_smash(world, id, cmd, line),
        Cmd::Killblow => combat::cmd_killblow(world, id, cmd, line),
        Cmd::Backstab => combat::cmd_backstab(world, id, cmd, line),
        Cmd::Attack => combat::cmd_attack(world, id, cmd, line),
        // the ten directions: moving ends resting and meditating
        Cmd::North
        | Cmd::South
        | Cmd::East
        | Cmd::West
        | Cmd::NorthEast
        | Cmd::NorthWest
        | Cmd::SouthEast
        | Cmd::SouthWest
        | Cmd::Up
        | Cmd::Down => {
            stop_resting(world, id);
            movement::cmd_move(world, id, cmd, line)
        }
        Cmd::Help => info::cmd_help(world, id, cmd, line),
        Cmd::Save => misc::cmd_save(world, id, cmd, line),
        Cmd::Look => info::cmd_look(world, id, cmd, line),
        Cmd::Map => info::cmd_map(world, id, cmd, line),
        // the displays take no argument: with one, the word is speech
        Cmd::Status => alone(line, || info::show_status(world, id)),
        Cmd::Health => alone(line, || info::show_health(world, id)),
        Cmd::Inventory => alone(line, || inventory::display_inventory_items(world, id)),
        Cmd::Powers | Cmd::Spells => alone(line, || info::display_users_spells(world, id, cmd)),
        Cmd::Light => inventory::cmd_light(world, id, cmd, line),
        Cmd::Use | Cmd::Read => use_item::cmd_use(world, id, cmd, line),
        Cmd::Keys => alone(line, || inventory::display_users_keys(world, id)),
        Cmd::Eat => use_item::cmd_eat(world, id, cmd, line),
        Cmd::Drink => use_item::cmd_drink(world, id, cmd, line),
        Cmd::Get => inventory::cmd_get(world, id, cmd, line),
        Cmd::Stash => inventory::cmd_stash(world, id, cmd, line),
        Cmd::Hide => misc::cmd_hide(world, id, cmd, line),
        Cmd::Drop => inventory::cmd_drop(world, id, cmd, line),
        Cmd::Track => misc::cmd_track(world, id, cmd, line),
        Cmd::Tame => misc::cmd_tame(world, id, cmd, line),
        Cmd::Mesmerize => misc::cmd_mesmerize(world, id, cmd, line),
        Cmd::Cast | Cmd::Invoke => super::cast::cmd_cast(world, id, cmd, line),
        Cmd::Give => inventory::cmd_give(world, id, cmd, line),
        Cmd::Rob => misc::cmd_rob(world, id, cmd, line),
        Cmd::List => shop::cmd_list(world, id, cmd, line),
        Cmd::Stock => shop::cmd_stock(world, id, cmd, line),
        Cmd::Unstock => shop::cmd_unstock(world, id, cmd, line),
        Cmd::MarkUp => shop::cmd_markup(world, id, cmd, line),
        Cmd::Buy => shop::cmd_buy(world, id, cmd, line),
        Cmd::Sell => shop::cmd_sell(world, id, cmd, line),
        Cmd::Leave => group::cmd_leave(world, id, cmd, line),
        Cmd::Join => group::cmd_join(world, id, cmd, line),
        Cmd::Broadcast => comms::cmd_broadcast(world, id, cmd, line),
        Cmd::Suicide | Cmd::Reroll => misc::cmd_suicide(world, id, cmd, line),
        Cmd::Arm => inventory::cmd_arm(world, id, cmd, line),
        Cmd::Wield => inventory::cmd_wield(world, id, cmd, line),
        Cmd::Wear => inventory::cmd_wear(world, id, cmd, line),
        Cmd::Ready => inventory::cmd_ready(world, id, cmd, line),
        Cmd::Equip => inventory::cmd_equip(world, id, cmd, line),
        Cmd::Remove => inventory::cmd_remove(world, id, cmd, line),
        // DISARM TRAP {direction}; any other DISARM is UNREADY
        Cmd::Disarm if line.argc() == 3 && arg_is(line, 1, "trap") => {
            misc::cmd_disarm(world, id, cmd, line)
        }
        // UNREADY: ends resting, meditating, sneaking and hiding, then unreadies the weapon
        Cmd::Unready | Cmd::Disarm => {
            stop_resting(world, id);
            if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
                plr.status.remove(PlayerStatus::SNEAKING);
                plr.hidden = 0;
            }
            inventory::cmd_unready(world, id, cmd, line)
        }
        Cmd::Follow => group::cmd_follow(world, id, cmd, line),
        Cmd::Identity => info::cmd_identity(world, id, cmd, line),
        Cmd::Topten => info::cmd_topten(world, id, cmd, line),
        Cmd::Quiet | Cmd::Brief => misc::cmd_quiet(world, id, cmd, line),
        Cmd::Verbose => misc::cmd_verbose(world, id, cmd, line),
        Cmd::Version => misc::cmd_version(world, id, cmd, line),
        Cmd::Talk => misc::cmd_talk(world, id, cmd, line),
        Cmd::Scan => alone(line, || info::display_scan_of_users(world, id)),
        Cmd::Guard => misc::cmd_guard(world, id, cmd, line),
        Cmd::Sneak => misc::cmd_sneak(world, id, cmd, line),
        Cmd::PickLock => misc::cmd_picklock(world, id, cmd, line),
        Cmd::Search => misc::cmd_search(world, id, cmd, line),
        Cmd::Exits => alone(line, || info::display_visible_exits(world, id)),
        Cmd::StatLine => misc::cmd_statline(world, id, cmd, line),
        Cmd::Profile => alone(line, || info::display_profile(world, id)),
        Cmd::Deposit => bank::cmd_deposit(world, id, cmd, line),
        Cmd::Withdraw => bank::cmd_withdraw(world, id, cmd, line),
        Cmd::Borrow => bank::cmd_borrow(world, id, cmd, line),
        Cmd::Repay => bank::cmd_repay(world, id, cmd, line),
        Cmd::BankBook => bank::cmd_bankbook(world, id, cmd, line),
        Cmd::Open => movement::cmd_open(world, id, cmd, line),
        Cmd::Close => movement::cmd_close(world, id, cmd, line),
        Cmd::Break => combat::cmd_break(world, id, cmd, line),
        Cmd::Aid => misc::cmd_aid(world, id, cmd, line),
        Cmd::Drag => movement::cmd_drag(world, id, cmd, line),
        Cmd::Sysop => sysop::cmd_sysop(world, id, cmd, line),
        Cmd::Invite => group::cmd_invite(world, id, cmd, line),
        Cmd::Uninvite => group::cmd_uninvite(world, id, cmd, line),
        // the ranks: end resting and meditating, then the change (no argument)
        Cmd::BackRank | Cmd::FrontRank | Cmd::Midrank => {
            stop_resting(world, id);
            alone(line, || group::change_user_rank(world, id, cmd))
        }
        Cmd::Train => training::cmd_train(world, id, cmd, line),
        Cmd::Party => alone(line, || group::display_party(world, id)),
        Cmd::Page => misc::cmd_page(world, id, cmd, line),
        Cmd::Set => settings::cmd_set(world, id, cmd, line),
        Cmd::Experience => info::cmd_experience(world, id, cmd, line),
        Cmd::Whisper => comms::cmd_whisper(world, id, cmd, line),
        Cmd::Lock => movement::cmd_lock(world, id, cmd, line),
        Cmd::Broadgang => gang::cmd_broadgang(world, id, cmd, line),
        Cmd::Disband => group::cmd_disband(world, id, cmd, line),
        Cmd::Ask => ask_greet::cmd_ask(world, id, cmd, line),
        Cmd::Greet => ask_greet::cmd_greet(world, id, cmd, line),
        Cmd::Feedback => comms::cmd_feedback(world, id, cmd, line),
        Cmd::Create => gang::cmd_create(world, id, cmd, line),
        Cmd::Forgive => misc::cmd_forgive(world, id, cmd, line),
        Cmd::Share => group::cmd_share(world, id, cmd, line),
        Cmd::Wealth => inventory::display_user_money(world, id),
        Cmd::Rest => misc::cmd_rest(world, id, cmd, line),
        Cmd::Meditate => misc::cmd_meditate(world, id, cmd, line),
        Cmd::Action => misc::cmd_action(world, id, cmd, line),
        Cmd::DirectMsg => comms::cmd_direct_message(world, id, cmd, line),
        Cmd::Gossip => comms::cmd_gossip(world, id, cmd, line),
        Cmd::Auction => comms::cmd_auction(world, id, cmd, line),
        Cmd::Appraise => shop::cmd_appraise(world, id, cmd, line),
        Cmd::Purge => crate::spells::spellbook::cmd_purge(world, id, cmd, line),
        Cmd::Global => comms::cmd_global(world, id, cmd, line),
        // codes with no routine: the word is speech
        Cmd::Sound
        | Cmd::Throw
        | Cmd::Fire
        | Cmd::Donate
        | Cmd::Play
        | Cmd::Shoot
        | Cmd::Take
        | Cmd::Text
        | Cmd::Autocombat
        | Cmd::Defend
        | Cmd::Hunt
        | Cmd::Dismiss
        | Cmd::Research => Handled::NotCommand,
        Cmd::Forget => misc::cmd_forget(world, id, cmd, line),
        Cmd::Promote => gang::cmd_promote(world, id, cmd, line),
        Cmd::Demote => gang::cmd_demote(world, id, cmd, line),
    }
}

/// A display command with an argument is no command.
fn alone(line: &CommandLine, display: impl FnOnce()) -> Handled {
    if line.argc() == 1 {
        display();
        Handled::Done
    } else {
        Handled::NotCommand
    }
}

/// Ends resting and meditating, before a move, a rank change or UNREADY.
fn stop_resting(world: &mut World, id: SessionId) {
    if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        plr.status.remove(PlayerStatus::RESTING);
        plr.status2.remove(PlayerStatus2::MEDITATING);
    }
}

/// `eq_ignore_case` of word `i` and `word`, for the arms that take one fixed keyword.
pub fn arg_is(line: &CommandLine, i: usize, word: &str) -> bool {
    eq_ignore_case(line.arg(i), word)
}
