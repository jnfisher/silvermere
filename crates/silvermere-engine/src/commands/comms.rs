//! Communication: telepath, say-to, broadcast, gossip, auction.
//!
//! Say and yell are the speech tail of `execute_input` (`commands::input`); the shared
//! pieces (finding a player by name, the page allowance, the forget list) are
//! [`crate::comms`]. As in the original, [`cmd_whisper`] *is* the telepath (`/name text`,
//! `WHISPER name text`) and [`cmd_direct_message`] is say-to (`>name text`); there is no
//! other whisper. Under MBBSEmu GLOBAL lines are the BBS's global commands, which do not
//! exist here (no host command layer): [`cmd_global`] is always [`Handled::NotCommand`], so
//! the line goes on to speech, as it does in the original with no global command registered.

use silvermere_text::Msg;
use silvermere_text::colour::ColRow;
use silvermere_types::flags::{PlayerFlags, PlayerSettings, PlayerStatus};
use silvermere_types::ids::{RoomKey, SessionId};

use super::dispatch::Handled;
use super::{Cmd, CommandLine};
use crate::comms::allowance::{count_page, page_left};
use crate::comms::forget::is_user_forgotten;
use crate::comms::{UserMatch, find_user_number};
use crate::event_loop::{AuditValue, EngineOutput, GameEvent};
use crate::items::inventory::show_options;
use crate::tell::{Noise, TellBlind, tell_game, tell_group, tell_room, tell_user};
use crate::world::World;

/// The target named by the first word after its prefix character (`/name`, `>name`) or by
/// the second word after the command word. `Err(())` for the prefix on its own.
fn target_word(line: &CommandLine, prefix: u8) -> Result<&str, ()> {
    let first = line.first();
    if first.as_bytes().first() == Some(&prefix) {
        if first.len() == 1 {
            return Err(());
        }
        Ok(&first[1..])
    } else {
        Ok(line.arg(1))
    }
}

/// The message after the target: the rest of the raw line after `/name`, or after `WHISPER name`;
/// `None` for a command word and a name but no message.
fn message_text(line: &CommandLine, prefix: u8) -> Option<&str> {
    if line.raw.as_bytes().first() == Some(&prefix) {
        Some(line.rest_from(1))
    } else if line.argc() > 2 {
        Some(line.rest_from(2))
    } else {
        None
    }
}

fn first_name(world: &World, id: SessionId) -> String {
    world
        .session(id)
        .and_then(|s| s.player())
        .map(|p| p.firstname.clone())
        .unwrap_or_default()
}

/// The telepath. The receiver must still have a page left this period (`pagecnt <= pagemax`), not
/// block telepaths and not have forgotten the sender; the sender is told only `Sent` or `Not Sent`,
/// never why. A delivered telepath counts against the receiver's allowance.
pub fn cmd_whisper(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if world.session(id).and_then(|s| s.player()).is_none() {
        return Handled::NotCommand;
    }
    // nothing after the command word or the "/"
    if line.argc() == 1 {
        tell_user(world, id, Msg::TelepathNothing);
        return Handled::Done;
    }
    let Ok(name) = target_word(line, b'/') else {
        tell_user(world, id, Msg::TelepathNoTarget);
        return Handled::Done;
    };
    let found = find_user_number(world, id, name, false);
    // the name fits several players; the list, then the prompt
    if let UserMatch::Several(names) = &found {
        show_options(world, id, names);
        return Handled::Done;
    }
    // a player who blocks telepaths may not send them either
    let blocking = world
        .session(id)
        .and_then(|s| s.player())
        .is_some_and(|p| p.settings.contains(PlayerSettings::NOTELEPATH));
    if blocking {
        tell_user(world, id, Msg::TelepathBlocking);
        return Handled::Done;
    }
    match found {
        UserMatch::One(t) if t != id => {
            let open = world.session(t).and_then(|s| s.player()).is_some_and(|tp| {
                page_left(tp) && !tp.settings.contains(PlayerSettings::NOTELEPATH)
            }) && !is_user_forgotten(world, t, id);
            if !open {
                tell_user(world, id, Msg::TelepathNotSent);
                return Handled::Done;
            }
            let Some(tplr) = world.session_mut(t).and_then(|s| s.character.as_mut()) else {
                return Handled::Done;
            };
            // one more page this period (before the message is checked, sic)
            count_page(tplr);
            let target = tplr.firstname.clone();
            let Some(text) = message_text(line, b'/') else {
                tell_user(world, id, Msg::TelepathWhat);
                return Handled::Done;
            };
            let who = first_name(world, id);
            if let Some(s) = world.session_mut(t) {
                s.output.push(Msg::Telepath {
                    who,
                    text: text.to_string(),
                });
                s.push_prompt();
            }
            tell_user(world, id, Msg::TelepathSent { who: target });
        }
        UserMatch::One(_) => tell_user(world, id, Msg::TelepathSelf),
        _ => tell_user(world, id, Msg::UserUnknown),
    }
    Handled::Done
}

/// Say-to: `>name text` said out loud to one player in the same room. The target hears `says (to
/// you)`, the rest of the room `says (to <target>)`. It is not sent when the target is in another
/// room or hidden; blocking telepaths and the forget list do not matter here. A delivered line
/// counts against the target's page allowance, which is not checked.
pub fn cmd_direct_message(
    world: &mut World,
    id: SessionId,
    _cmd: Cmd,
    line: &CommandLine,
) -> Handled {
    let Some((key, muted)) = world.session(id).and_then(|s| s.player()).map(|p| {
        (
            RoomKey::new(p.map.0, p.room.0),
            p.status.contains(PlayerStatus::MUTE),
        )
    }) else {
        return Handled::NotCommand;
    };
    if line.argc() == 1 {
        tell_user(world, id, Msg::DirectNothing);
        return Handled::Done;
    }
    let Ok(name) = target_word(line, b'>') else {
        tell_user(world, id, Msg::DirectNoTarget);
        return Handled::Done;
    };
    match find_user_number(world, id, name, false) {
        UserMatch::Several(names) => show_options(world, id, &names),
        UserMatch::One(t) if t != id => {
            // a muted player cannot speak
            if muted {
                tell_user(world, id, Msg::DirectMuted);
                return Handled::Done;
            }
            let Some(tplr) = world.session_mut(t).and_then(|s| s.character.as_mut()) else {
                return Handled::Done;
            };
            // the target must be in the same room and not hidden
            if tplr.room != key.room || tplr.map != key.map || tplr.hidden != 0 {
                tell_user(world, id, Msg::MessageNotSent);
                return Handled::Done;
            }
            count_page(tplr);
            let target = tplr.firstname.clone();
            let Some(text) = message_text(line, b'>') else {
                tell_user(world, id, Msg::DirectWhat);
                return Handled::Done;
            };
            let who = first_name(world, id);
            if let Some(s) = world.session_mut(t) {
                s.output.push(Msg::SaysToYou {
                    who: who.clone(),
                    text: text.to_string(),
                });
                s.push_prompt();
            }
            // everyone in the room but the two of them
            tell_room(
                world,
                key,
                [Some(id), Some(t)],
                Noise::Always,
                TellBlind::All,
                true,
                None,
                &Msg::SaysTo {
                    who,
                    target: target.clone(),
                    text: text.to_string(),
                },
            );
            tell_user(world, id, Msg::MessageDirected { who: target });
        }
        UserMatch::One(_) => tell_user(world, id, Msg::DirectSelf),
        UserMatch::None => tell_user(world, id, Msg::UserUnknown),
    }
    Handled::Done
}

/// `'text`, `-text` or `BROADCAST text` to everyone on the speaker's channel (the speaker
/// included, each with a prompt; the speaker then gets the command's prompt as well, sic). The
/// quote, dash or command word alone lists the channel. Off any channel the line is not a command.
/// No profanity check applies (sic: the original's never fires) and the host's profanity filter is
/// not carried.
pub fn cmd_broadcast(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(channel) = world
        .session(id)
        .and_then(|s| s.player())
        .map(|p| p.bchannel)
    else {
        return Handled::NotCommand;
    };
    if channel == 0 {
        return Handled::NotCommand;
    }
    let first = line.first();
    let quoted = matches!(first.as_bytes().first(), Some(b'\'' | b'-'));
    if line.argc() > 1 || (quoted && first.len() > 1) {
        // without the quote or dash when it is joined to the first word
        let text = if quoted {
            &line.rest_from(0)[1..]
        } else {
            line.rest_from(1)
        };
        let msg = Msg::Broadcast {
            who: first_name(world, id),
            text: text.to_string(),
        };
        tell_group(world, channel, None, &msg);
        return Handled::Done;
    }
    // list the channel
    let names: Vec<String> = world
        .session_ids()
        .into_iter()
        .filter(|&i| world.is_in_game(i))
        .filter_map(|i| world.session(i).and_then(|s| s.player()))
        .filter(|p| p.bchannel == channel)
        .map(|p| p.firstname.clone())
        .collect();
    tell_user(
        world,
        id,
        Msg::ChannelUsers {
            n: i32::from(channel),
            names: names.join(", "),
        },
    );
    Handled::Done
}

/// The two game-wide channels: gossip and auction, twins with their own switch, key and
/// allowance.
#[derive(Clone, Copy)]
enum Channel {
    Gossip,
    Auction,
}

/// GOSSIP and AUCTION.
fn game_channel(world: &mut World, id: SessionId, line: &CommandLine, ch: Channel) -> Handled {
    let (mask, on, off, refused, turned_off) = match ch {
        Channel::Gossip => (
            PlayerFlags::NOGOSSIP,
            Msg::SetGossipOn,
            Msg::SetGossipOff,
            Msg::GossipRefused,
            Msg::GossipTurnedOff,
        ),
        Channel::Auction => (
            PlayerFlags::NOAUCTION,
            Msg::SetAuctionOn,
            Msg::SetAuctionOff,
            Msg::AuctionRefused,
            Msg::AuctionTurnedOff,
        ),
    };
    let Some(s) = world.session_mut(id) else {
        return Handled::NotCommand;
    };
    // The original's per-player gossip and auction keys (GOSSKEY/AUCTKEY) become the account's
    // gossip and auction permissions (05 section 8): login sets the flag for a permitted player who
    // is not staff; staff always may.
    let staff = s.flags.sysop || s.flags.gameop;
    let permitted = staff
        || match ch {
            Channel::Gossip => s.flags.hasgoss,
            Channel::Auction => s.flags.hasauct,
        };
    let Some(plr) = s.character.as_mut() else {
        return Handled::NotCommand;
    };
    if line.argc() == 1 {
        // a bare GOSSIP flips the switch
        let msg = if plr.flags.contains(mask) {
            plr.flags.remove(mask);
            on
        } else {
            plr.flags.insert(mask);
            off
        };
        s.output.push(msg);
        return Handled::Done;
    }
    if !permitted {
        s.output.push(refused);
        return Handled::Done;
    }
    if plr.flags.contains(mask) {
        s.output.push(turned_off);
        return Handled::Done;
    }
    let who = plr.firstname.clone();
    let prefix = match ch {
        Channel::Gossip => Msg::GossipPrefix { who },
        Channel::Auction => Msg::AuctionPrefix { who },
    };
    // "{name} gossips: " first, then the text in the broadcast colour with the prompt: two
    // game-wide messages, each counted against every listener's allowance
    tell_game(world, None, mask, false, None, &prefix);
    let text = Msg::ChannelText {
        text: line.rest_from(1).to_string(),
    };
    tell_game(
        world,
        None,
        mask,
        true,
        Some(ColRow::Broadcast.index()),
        &text,
    );
    Handled::Done
}

/// `GOSSIP` alone switches it on or off.
pub fn cmd_gossip(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    game_channel(world, id, line, Channel::Gossip)
}

/// `AUCTION` alone switches it on or off.
pub fn cmd_auction(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    game_channel(world, id, line, Channel::Auction)
}

/// GLOBAL: no BBS global commands exist, so the line is not a command and goes on to speech, as
/// in the original with none registered.
pub fn cmd_global(_world: &mut World, _id: SessionId, _cmd: Cmd, _line: &CommandLine) -> Handled {
    Handled::NotCommand
}

/// FEEDBACK {comments} sends the comments on, then thanks the player; alone it shows the syntax.
///
/// The original mailed them to WCC's support address ("user-id -- first name (map/room)",
/// then `INP:` and the typed line, then the module version). silvermere has no BBS mail: the same
/// text goes out as a `feedback` audit event for the operators.
pub fn cmd_feedback(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() == 1 {
        tell_user(world, id, Msg::FeedbackSyntax);
        return Handled::Done;
    }
    if let Some(p) = world.session(id).and_then(|s| s.player()) {
        let user = p.userid.clone();
        let text = feedback_text(
            &p.userid,
            &p.firstname,
            p.map.0,
            p.room.0,
            line.rest_from(0),
            &world.compat.mud_version,
        );
        tracing::info!(target: "audit", kind = "feedback", user = %user, "{text}");
        world.outbox.push(EngineOutput::Event(GameEvent::Audit {
            kind: "feedback",
            fields: vec![
                ("user", AuditValue::Text(user)),
                ("text", AuditValue::Text(text)),
            ],
        }));
    }
    tell_user(world, id, Msg::FeedbackSent);
    Handled::Done
}

/// The feedback mail text as the original sends it, the typed line cut so that the whole stays
/// within 0x77C characters.
pub fn feedback_text(
    userid: &str,
    firstname: &str,
    map: u32,
    room: u32,
    input: &str,
    mudver: &str,
) -> String {
    let mut buf = format!("{userid} -- {firstname} ({map}/{room})\n\nINP:");
    let room_left = 0x77C_usize.saturating_sub(buf.len() + mudver.len());
    let cut = input
        .char_indices()
        .nth(room_left)
        .map_or(input, |(i, _)| &input[..i]);
    buf.push_str(cut);
    buf.push_str("\r\nV");
    buf.push_str(mudver);
    buf
}

#[cfg(test)]
mod tests {
    use super::feedback_text;

    // the original's feedback mail text
    #[test]
    fn feedback_text_is_the_mail_of_the_original() {
        assert_eq!(
            feedback_text(
                "ALDRIC",
                "Aldric",
                1,
                3000,
                "fe the orcs are too strong",
                "1.11p"
            ),
            "ALDRIC -- Aldric (1/3000)\n\nINP:fe the orcs are too strong\r\nV1.11p"
        );
        // the typed line is cut so the text and the version stay within 0x77C
        let text = feedback_text("ALDRIC", "Aldric", 1, 3000, &"x".repeat(3000), "1.11p");
        assert_eq!(text.len(), 0x77C + "\r\nV".len());
        assert!(text.starts_with("ALDRIC -- Aldric (1/3000)\n\nINP:xxx"));
        assert!(text.ends_with("x\r\nV1.11p"));
    }
}
