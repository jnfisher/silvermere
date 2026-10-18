//! Roles and permissions: what the module asked with `haskey(lock)`.
//!
//! The module tested Worldgroup keys in three ways, and each has a home here:
//!
//! - `SYSOPKEY` and `GAMOPKEY`, read once on entering the game: [`Role::Sysop`] and
//!   [`Role::GameOp`].
//! - The 21 `SYS_*` keys of `WCCMMUD.INI` that `cmd_sysop` checks on top of the sysop
//!   role: [`Perm`]'s sysop variants, named by their INI key.
//! - `GOSSKEY` and `AUCTKEY`, which *take away* gossip and auctions, and `LOANKEY`,
//!   needed for bank loans: [`Perm::Gossip`],
//!   [`Perm::Auction`] and [`Perm::Loan`], granted by default and withdrawn with the
//!   account flags `no_gossip`, `no_auction`, `no_loan`.
//!
//! `PLAYKEY`/`NOPLAYKY` are dropped: everyone may play.

use std::fmt;
use std::str::FromStr;

use crate::account::{Account, AccountFlags};

/// An account role. Stored as its lowercase name in `account_roles.role`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    /// Every account.
    Player,
    /// The original's `SYSOPKEY`.
    Sysop,
    /// The original's `GAMOPKEY`.
    GameOp,
}

impl Role {
    /// Every role.
    pub const ALL: [Role; 3] = [Role::Player, Role::Sysop, Role::GameOp];

    /// The stored name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Player => "player",
            Self::Sysop => "sysop",
            Self::GameOp => "gameop",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A name that is not a role or permission.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown {what}: {name}")]
pub struct UnknownName {
    /// `role` or `permission`.
    pub what: &'static str,
    /// The name given.
    pub name: String,
}

impl FromStr for Role {
    type Err = UnknownName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Role::ALL
            .into_iter()
            .find(|r| r.name().eq_ignore_ascii_case(s))
            .ok_or_else(|| UnknownName {
                what: "role",
                name: s.to_string(),
            })
    }
}

/// The roles an account holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct RoleSet(u8);

impl RoleSet {
    const fn bit(role: Role) -> u8 {
        1 << role as u8
    }

    /// No roles.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Only `player`.
    pub const fn player() -> Self {
        Self(Self::bit(Role::Player))
    }

    /// Whether `role` is held.
    pub const fn contains(self, role: Role) -> bool {
        self.0 & Self::bit(role) != 0
    }

    /// Adds `role`.
    pub fn insert(&mut self, role: Role) {
        self.0 |= Self::bit(role);
    }

    /// Removes `role`.
    pub fn remove(&mut self, role: Role) {
        self.0 &= !Self::bit(role);
    }

    /// The roles held, in [`Role::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = Role> {
        Role::ALL.into_iter().filter(move |r| self.contains(*r))
    }
}

impl FromIterator<Role> for RoleSet {
    fn from_iter<I: IntoIterator<Item = Role>>(iter: I) -> Self {
        let mut set = Self::empty();
        for r in iter {
            set.insert(r);
        }
        set
    }
}

macro_rules! perms {
    ($( $(#[$doc:meta])* $variant:ident = $name:literal, )*) => {
        /// A permission. The sysop sub-command permissions are named by their
        /// `WCCMMUD.INI` key, which is also how they are stored in `account_roles.role`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Perm {
            $( $(#[$doc])* $variant, )*
        }

        impl Perm {
            /// Every permission, sysop ones first in `WCCMMUD.INI` order.
            pub const ALL: &'static [Perm] = &[$(Perm::$variant,)*];

            /// The serialised name: the INI key for a sysop permission.
            pub const fn ini_name(self) -> &'static str {
                match self {
                    $( Perm::$variant => $name, )*
                }
            }
        }
    };
}

perms! {
    /// `SYS_MAP_KEY`: SYS MAP.
    SysMap = "SYS_MAP_KEY",
    /// `SYS_REPORT_KEY`: SYS REPORT.
    SysReport = "SYS_REPORT_KEY",
    /// `SYS_GOTO_KEY`: SYS GOTO.
    SysGoto = "SYS_GOTO_KEY",
    /// `SYS_LIGHTNING_KEY`: SYS LIGHTNING.
    SysLightning = "SYS_LIGHTNING_KEY",
    /// `SYS_STATUS_KEY`: SYS STATUS.
    SysStatus = "SYS_STATUS_KEY",
    /// `SYS_RESET_ROOMS`: SYS RESET.
    SysResetRooms = "SYS_RESET_ROOMS",
    /// `SYS_ADDLIFE_KEY`: GOD user ADD LIFE.
    SysAddLife = "SYS_ADDLIFE_KEY",
    /// `SYS_RETRAIN_KEY`: GOD user RETRAIN.
    SysRetrain = "SYS_RETRAIN_KEY",
    /// `SYS_ALIGNMENT_KEY`: GOD user GOOD/SAINT/NEUTRAL/LAWFUL/UNLAWFUL.
    SysAlignment = "SYS_ALIGNMENT_KEY",
    /// `SYS_EDITEVIL_KEY`: GOD user ADD EVIL.
    SysEditEvil = "SYS_EDITEVIL_KEY",
    /// `SYS_JAIL_KEY`: GOD user JAIL.
    SysJail = "SYS_JAIL_KEY",
    /// `SYS_SPEAK_KEY`: GOD user SPEAK.
    SysSpeak = "SYS_SPEAK_KEY",
    /// `SYS_LISTUSER_KEY`: LIST USERS, LIST EVIL, LIST GANG.
    SysListUser = "SYS_LISTUSER_KEY",
    /// `SYS_LISTLIMIT_KEY`: LIST LIMITED_ITEMS.
    SysListLimit = "SYS_LISTLIMIT_KEY",
    /// `SYS_USERINFO_KEY`: the GOD user commands that show details.
    SysUserInfo = "SYS_USERINFO_KEY",
    /// `SYS_CHANGE_KEY`: GOD user CHANGE (name or sex).
    SysChange = "SYS_CHANGE_KEY",
    /// `SYS_TOPTEN_KEY`: GOD user ENABLE/DISABLE TOPTEN.
    SysTopTen = "SYS_TOPTEN_KEY",
    /// `SYS_DISBAND_KEY`: SYS DISBAND gang.
    SysDisband = "SYS_DISBAND_KEY",
    /// `SYS_BUFFERS_KEY`: SYS BUFFERS.
    SysBuffers = "SYS_BUFFERS_KEY",
    /// `SYS_CLEARSUICIDE_KEY`: GOD user CLEAR SUICIDE.
    SysClearSuicide = "SYS_CLEARSUICIDE_KEY",
    /// `SYS_DISABLE_GANGS`: SYS DISABLE gang.
    SysDisableGangs = "SYS_DISABLE_GANGS",
    /// May gossip; `GOSSKEY` took it away.
    Gossip = "GOSSIP",
    /// May auction; `AUCTKEY` took it away.
    Auction = "AUCTION",
    /// May take bank loans; the original needed `LOANKEY`.
    Loan = "LOAN",
    /// The developer gate of `cmd_sysop` (T-055): the twinks codec at 78 (`CODEC_OF_TWINKS`)
    /// for DEBUG, MONSTER, SUMMON, EXPRESS_TRAIN and TWEAK, and the registration-number
    /// `devmode` for the developer LISTs and CONFIGURE GENRATE/MINWAIT. Held only when
    /// granted, a sysop included (see [`has_permission`]).
    Debug = "DEBUG",
}

impl Perm {
    /// The sysop sub-command permissions (the `SYS_*` INI keys).
    pub fn sysop_perms() -> impl Iterator<Item = Perm> {
        Self::ALL.iter().copied().filter(|p| p.is_sysop_key())
    }

    /// Whether this is one of the `SYS_*` INI keys.
    pub const fn is_sysop_key(self) -> bool {
        !matches!(
            self,
            Self::Gossip | Self::Auction | Self::Loan | Self::Debug
        )
    }

    /// The permission an INI key (or `GOSSIP`/`AUCTION`/`LOAN`) names, ignoring case.
    pub fn from_ini_name(name: &str) -> Option<Perm> {
        Self::ALL
            .iter()
            .copied()
            .find(|p| p.ini_name().eq_ignore_ascii_case(name))
    }
}

impl fmt::Display for Perm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.ini_name())
    }
}

impl FromStr for Perm {
    type Err = UnknownName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Perm::from_ini_name(s).ok_or_else(|| UnknownName {
            what: "permission",
            name: s.to_string(),
        })
    }
}

/// Permissions granted to an account explicitly (beyond what its roles give).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct PermSet(u32);

impl PermSet {
    const fn bit(perm: Perm) -> u32 {
        1 << perm as u32
    }

    /// No permissions.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Whether `perm` is in the set.
    pub const fn contains(self, perm: Perm) -> bool {
        self.0 & Self::bit(perm) != 0
    }

    /// Adds `perm`.
    pub fn insert(&mut self, perm: Perm) {
        self.0 |= Self::bit(perm);
    }

    /// Removes `perm`.
    pub fn remove(&mut self, perm: Perm) {
        self.0 &= !Self::bit(perm);
    }

    /// The permissions in the set, in [`Perm::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = Perm> {
        Perm::ALL.iter().copied().filter(move |p| self.contains(*p))
    }
}

impl FromIterator<Perm> for PermSet {
    fn from_iter<I: IntoIterator<Item = Perm>>(iter: I) -> Self {
        let mut set = Self::empty();
        for p in iter {
            set.insert(p);
        }
        set
    }
}

/// Whether `account` holds `role`, the sysop or game-operator standing the original read
/// from the keys once on entering the game. Keys were never disabled there, so there is no
/// disabled-key case.
pub fn has_role(account: &Account, role: Role) -> bool {
    account.roles.contains(role)
}

/// Whether `account` may do what `perm` guards; called where the module called
/// `haskey(lock)`.
///
/// - `Debug` (the developer gate) only when granted, to anyone, a sysop included.
/// - A sysop may do everything else.
/// - A game operator holds every `SYS_*` permission too: the INI keys were extra locks on
///   top of the sysop/gameop test, empty by default, and the
///   sub-commands that were the sysop's alone check the sysop role themselves (T-055).
/// - `SysDisband`: a game operator disbanded only when the key was set and held,
///   so a game operator holds it only when granted it.
/// - Anyone else holds a `SYS_*` permission only when granted it explicitly.
/// - `Gossip`/`Auction`/`Loan` are held unless the account's `no_gossip`/`no_auction`/
///   `no_loan` flag is set; sysops and game operators keep gossip and auctions even then
///   (the keys take gossip and auctions away from other players only).
pub fn has_permission(account: &Account, perm: Perm) -> bool {
    let staff = account.roles.contains(Role::Sysop) || account.roles.contains(Role::GameOp);
    match perm {
        // the twinks codec must be 78: CODEC_OF_TWINKS is in neither stock file, so
        // nobody passed the developer gate unless the INI set it, a sysop included
        Perm::Debug => account.perms.contains(perm),
        _ if account.roles.contains(Role::Sysop) => true,
        Perm::Gossip => staff || !account.flags.contains(AccountFlags::NO_GOSSIP),
        Perm::Auction => staff || !account.flags.contains(AccountFlags::NO_AUCTION),
        Perm::Loan => !account.flags.contains(AccountFlags::NO_LOAN),
        // a game operator disbands only with the key set and held
        Perm::SysDisband => account.perms.contains(perm),
        sys => account.roles.contains(Role::GameOp) || account.perms.contains(sys),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_one_sysop_keys_and_four_others() {
        assert_eq!(Perm::sysop_perms().count(), 21);
        assert_eq!(Perm::ALL.len(), 25);
        assert!(Perm::ALL.len() <= 32, "PermSet is a u32");
    }

    fn account(name: &str) -> Account {
        Account {
            id: silvermere_types::ids::AccountId(1),
            username: name.into(),
            password_hash: String::new(),
            email: None,
            sex: crate::Sex::Male,
            roles: RoleSet::empty(),
            perms: PermSet::empty(),
            flags: AccountFlags::default(),
            created_at: jiff::Timestamp::UNIX_EPOCH,
            last_login_at: None,
            failed_logins: 0,
            locked_until: None,
        }
    }

    // T-055: the game operator's default set
    #[test]
    fn gameop_defaults() {
        let mut a = account("gil");
        a.roles.insert(Role::GameOp);
        assert!(has_permission(&a, Perm::SysGoto));
        assert!(!has_permission(&a, Perm::SysDisband));
        assert!(!has_permission(&a, Perm::Debug));
        a.perms.insert(Perm::SysDisband);
        assert!(has_permission(&a, Perm::SysDisband));
        let mut s = account("sid");
        s.roles.insert(Role::Sysop);
        assert!(
            Perm::ALL
                .iter()
                .all(|p| has_permission(&s, *p) == (*p != Perm::Debug))
        );
        // the developer gate is an explicit grant, even for a sysop
        s.perms.insert(Perm::Debug);
        assert!(has_permission(&s, Perm::Debug));
    }

    #[test]
    fn roles_parse_any_case() {
        assert_eq!("GameOp".parse::<Role>(), Ok(Role::GameOp));
        assert!("wizard".parse::<Role>().is_err());
    }
}
