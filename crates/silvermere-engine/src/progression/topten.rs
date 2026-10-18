//! The experience top list behind TOPTEN: the start-up build ([`TopTen::rebuild`]), the
//! update of one character ([`TopTen::update`]) and the removal of a rerolled one
//! ([`TopTen::remove`]), over a sorted `Vec`.
//!
//! The list holds only the entries in use, highest experience first (billions, then the
//! remainder), and an update puts the entry straight at its place: an entry moving up
//! stops below the entries with as much experience, one moving down stops above them.
//! (sic, not carried) Under the original some updates can lose or misplace an entry; that
//! has no defined result to reproduce.

use std::ops::Deref;

use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::SessionId;
use silvermere_types::records::{Character, TopEntry};
use silvermere_types::sizes::NTOPLIST;

use crate::world::{World, top_entry};

/// The top list: at most [`NTOPLIST`] (100) entries, highest experience first. TOPTEN
/// shows `maxtop` of them (T-027).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopTen(Vec<TopEntry>);

impl Deref for TopTen {
    type Target = [TopEntry];

    fn deref(&self) -> &[TopEntry] {
        &self.0
    }
}

/// An entry's experience as one comparable number (billions first).
fn exp_of(e: &TopEntry) -> u64 {
    (u64::from(e.exphi) << 32) | u64::from(e.explo)
}

/// `"EDITED"` for an immortal character.
fn mark(c: &Character) -> String {
    if c.immortal != 0 {
        "EDITED".to_string()
    } else {
        String::new()
    }
}

impl TopTen {
    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// The entries, highest first.
    pub fn entries(&self) -> &[TopEntry] {
        &self.0
    }

    /// The start-up build: the list from the stored characters, leaving out the ones the
    /// sysop took off TOPTEN (`PS_NOTOPTEN`), highest experience first; equal experience
    /// keeps the order the characters come in (under the original, of two characters with
    /// equal experience the later by user-id lists above the earlier). Immortals are marked
    /// `EDITED`. At most [`NTOPLIST`] are kept.
    ///
    /// The pre-1.11 experience conversion and its `log.log` are not carried: every stored
    /// character is on the 1.11 system.
    pub fn rebuild<'a>(characters: impl IntoIterator<Item = &'a Character>) -> Self {
        let mut list: Vec<TopEntry> = characters
            .into_iter()
            .filter(|c| !c.settings.contains(PlayerSettings::NOTOPTEN))
            .map(|c| TopEntry {
                edited: mark(c),
                ..top_entry(c)
            })
            .collect();
        list.sort_by_key(|e| std::cmp::Reverse(exp_of(e)));
        list.truncate(NTOPLIST);
        Self(list)
    }

    /// The update for one character: an entry found by user-id gets the character's names,
    /// experience and `EDITED` mark (class and gang are not refreshed, as in the original)
    /// and moves to its place. A character not on the list joins it with more experience
    /// than the slot past the end, an empty slot when the list is not full, and so with any
    /// experience at all; a full list drops its last entry for him.
    ///
    /// Under the original a character the sysop took off TOPTEN could come back on it before
    /// the next start-up; here an update takes him off (or keeps him off) the list.
    pub fn update(&mut self, c: &Character) {
        let found = self.0.iter().position(|e| e.userid == c.userid);
        if c.settings.contains(PlayerSettings::NOTOPTEN) {
            if let Some(i) = found {
                self.0.remove(i);
            }
            return;
        }
        let (old, entry) = match found {
            Some(i) => {
                let mut e = self.0.remove(i);
                e.name = c.firstname.clone();
                e.lastname = c.lastname.clone();
                e.exphi = c.expbil;
                e.explo = c.exprem;
                e.edited = mark(c);
                (i, e)
            }
            None => {
                let e = TopEntry {
                    edited: mark(c),
                    ..top_entry(c)
                };
                let floor = if self.0.len() >= NTOPLIST {
                    self.0.last().map_or(0, exp_of)
                } else {
                    0
                };
                if exp_of(&e) <= floor {
                    return;
                }
                if self.0.len() >= NTOPLIST {
                    self.0.pop();
                }
                (self.0.len(), e)
            }
        };
        let exp = exp_of(&entry);
        let above = self.0.iter().filter(|e| exp_of(e) > exp).count();
        let not_below = self.0.iter().filter(|e| exp_of(e) >= exp).count();
        self.0.insert(old.clamp(above, not_below), entry);
    }

    /// The character with `userid` leaves the list
    /// and the entries below move up one place.
    pub fn remove(&mut self, userid: &str) {
        // The last match; user-ids are unique, so it is the only one.
        if let Some(i) = self.0.iter().rposition(|e| e.userid == userid) {
            self.0.remove(i);
        }
    }
}

impl World {
    /// [`TopTen::update`] for the session's character. The experience gain calls it
    /// ([`crate::progression::add_experience`]); the save and combat-stop paths (T-023,
    /// T-033/T-034) call it too.
    pub fn update_top_list(&mut self, id: SessionId) {
        let Some(c) = self.sessions.get(&id).and_then(|s| s.character.as_ref()) else {
            return;
        };
        if !c.userid.is_empty() {
            self.topten.update(c);
        }
    }

    /// [`TopTen::remove`] for the session's character.
    pub fn remove_from_top_list(&mut self, id: SessionId) {
        let Some(userid) = self
            .sessions
            .get(&id)
            .and_then(|s| s.character.as_ref())
            .map(|c| c.userid.clone())
        else {
            return;
        };
        self.topten.remove(&userid);
    }
}
