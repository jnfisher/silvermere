//! Roles and permissions where the module called `haskey(lock)`.

use jiff::Timestamp;
use silvermere_auth::{
    Account, AccountFlags, Perm, PermSet, Role, RoleSet, Sex, has_permission, has_role,
};
use silvermere_types::ids::AccountId;

fn account(roles: &[Role]) -> Account {
    Account {
        id: AccountId(1),
        username: "x".into(),
        password_hash: String::new(),
        email: None,
        sex: Sex::Male,
        roles: roles.iter().copied().collect::<RoleSet>(),
        perms: PermSet::empty(),
        flags: AccountFlags::NONE,
        created_at: Timestamp::UNIX_EPOCH,
        last_login_at: None,
        failed_logins: 0,
        locked_until: None,
    }
}

// A sysop (the original's SYSOPKEY) holds every permission but the developer gate,
// which is an explicit grant (T-055)
#[test]
fn sysop_has_every_perm() {
    let mut a = account(&[Role::Player, Role::Sysop]);
    a.flags.insert(AccountFlags::NO_GOSSIP);
    a.flags.insert(AccountFlags::NO_AUCTION);
    a.flags.insert(AccountFlags::NO_LOAN);
    assert!(has_role(&a, Role::Sysop));
    for p in Perm::ALL {
        assert_eq!(has_permission(&a, *p), *p != Perm::Debug, "{p}");
    }
}

// A game operator (the original's GAMOPKEY). T-055: every SYS_* key but SYS_DISBAND_KEY,
// which let a game operator in only when set and held
#[test]
fn gameop_has_the_sysop_keys_but_is_not_a_sysop() {
    let a = account(&[Role::Player, Role::GameOp]);
    assert!(has_role(&a, Role::GameOp));
    assert!(!has_role(&a, Role::Sysop));
    for p in Perm::sysop_perms() {
        assert_eq!(has_permission(&a, p), p != Perm::SysDisband, "{p}");
    }
}

#[test]
fn a_player_has_no_sysop_keys_unless_granted() {
    let mut a = account(&[Role::Player]);
    for p in Perm::sysop_perms() {
        assert!(!has_permission(&a, p), "{p}");
    }
    a.perms.insert(Perm::SysGoto);
    assert!(has_permission(&a, Perm::SysGoto));
    assert!(!has_permission(&a, Perm::SysJail));
}

// AUCTKEY/GOSSKEY take auctions and gossip away, from players but not from staff
#[test]
fn gossip_and_auction_are_granted_unless_taken_away() {
    let mut a = account(&[Role::Player]);
    assert!(has_permission(&a, Perm::Gossip));
    assert!(has_permission(&a, Perm::Auction));
    assert!(has_permission(&a, Perm::Loan));
    a.flags.insert(AccountFlags::NO_GOSSIP);
    assert!(!has_permission(&a, Perm::Gossip));
    assert!(has_permission(&a, Perm::Auction));
    a.flags.insert(AccountFlags::NO_AUCTION);
    assert!(!has_permission(&a, Perm::Auction));
    a.flags.insert(AccountFlags::NO_LOAN);
    assert!(!has_permission(&a, Perm::Loan));
    // A game operator keeps gossip and auctions (the keys take them from others only).
    a.roles.insert(Role::GameOp);
    assert!(has_permission(&a, Perm::Gossip));
    assert!(has_permission(&a, Perm::Auction));
}

// The 21 SYS_* keys as WCCMMUD.INI names them
#[test]
fn ini_names_round_trip() {
    const INI: [&str; 21] = [
        "SYS_MAP_KEY",
        "SYS_REPORT_KEY",
        "SYS_GOTO_KEY",
        "SYS_LIGHTNING_KEY",
        "SYS_STATUS_KEY",
        "SYS_RESET_ROOMS",
        "SYS_ADDLIFE_KEY",
        "SYS_RETRAIN_KEY",
        "SYS_ALIGNMENT_KEY",
        "SYS_EDITEVIL_KEY",
        "SYS_JAIL_KEY",
        "SYS_SPEAK_KEY",
        "SYS_LISTUSER_KEY",
        "SYS_LISTLIMIT_KEY",
        "SYS_USERINFO_KEY",
        "SYS_CHANGE_KEY",
        "SYS_TOPTEN_KEY",
        "SYS_DISBAND_KEY",
        "SYS_BUFFERS_KEY",
        "SYS_CLEARSUICIDE_KEY",
        "SYS_DISABLE_GANGS",
    ];
    let names: Vec<&str> = Perm::sysop_perms().map(Perm::ini_name).collect();
    assert_eq!(names, INI);
    // The nineteen `SYS_*_KEY` keys the card names are among them.
    assert_eq!(INI.iter().filter(|n| n.ends_with("_KEY")).count(), 19);
    for name in INI {
        let p = Perm::from_ini_name(name).unwrap();
        assert_eq!(p.ini_name(), name);
        assert_eq!(name.parse::<Perm>().unwrap(), p);
        assert_eq!(p.to_string(), name);
        assert_eq!(Perm::from_ini_name(&name.to_lowercase()), Some(p));
    }
    assert_eq!(Perm::from_ini_name("SYS_NOPE_KEY"), None);
}

#[test]
fn role_names_round_trip() {
    for r in Role::ALL {
        assert_eq!(r.name().parse::<Role>().unwrap(), r);
    }
}
