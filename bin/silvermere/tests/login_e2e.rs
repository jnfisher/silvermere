//! The login flow over a socket (T-021): a telnet client logs in through
//! `silvermere-auth`'s dialogue, passes the BBS menu shim, and reaches the realm's
//! prompt; `NEW` creates an account that reaches the `[MAJORMUD]:` menu.

mod common;

use std::time::Duration;

use silvermere::config::Config;
use silvermere_auth::Role;
use silvermere_auth::cli::create_with_cost;
use silvermere_auth::password::Cost;
use silvermere_db::Db;
use silvermere_engine::{SeededRng, SystemClock, clear_player};
use silvermere_testkit::TelnetClient;
use silvermere_types::enums::StatlineMode;
use silvermere_types::ids::{ClassNo, MapId, RaceNo, RoomId};

const WAIT: Duration = Duration::from_secs(30);

async fn server(url: String) -> silvermere::Running {
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.database.url = url;
    silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(7))
        .await
        .expect("starts")
}

// An existing account logs in and enters the realm; docs/auth.md "The dialogue".
#[tokio::test]
async fn existing_character_reaches_the_prompt() {
    let (_t, url) = common::seeded(&[]).await;
    {
        let db = Db::connect_and_migrate(&url).await.unwrap();
        let account = create_with_cost(&db, "Aldric", "aldric1", &[Role::Player], Cost::Low)
            .await
            .unwrap();
        let mut c = clear_player();
        c.userid = "ALDRIC".into();
        c.firstname = "Aldric".into();
        c.map = MapId(1);
        c.room = RoomId(common::special_rooms()[0]);
        c.hp = 35;
        c.maxhp = 35;
        c.class = ClassNo(1);
        c.race = RaceNo(1);
        c.statline = StatlineMode::Full;
        // A stored row as `load_player` (T-023) accepts it: record version 22, level 1, stats
        // inside the zeroed test race's range (5..=50), the name validated, and `hpbonus`
        // making the recomputed maximum (50/2 + 10) the 35 hit points the prompt shows.
        c.version = 22;
        c.level = 1;
        (c.str, c.intel, c.wil, c.agi, c.hea, c.cha) = (50, 50, 50, 50, 50, 50);
        (c.basstr, c.basint, c.baswil, c.basagi, c.bashea, c.bascha) = (50, 50, 50, 50, 50, 50);
        c.hpbonus = 10;
        c.settings |= silvermere_types::flags::PlayerSettings::VALIDATED;
        db.characters().insert(account.id, &c).await.unwrap();
        db.close().await;
    }
    let r = server(url).await;
    let mut client = TelnetClient::connect(r.telnet_addr).await.unwrap();
    client.until("Username:", WAIT).await.unwrap();
    client.send_line("Aldric").await.unwrap();
    client.until("Password:", WAIT).await.unwrap();
    client.send_line("aldric1").await.unwrap();
    let got = client
        .until(r"Make your selection \(X to exit\):", WAIT)
        .await
        .unwrap();
    assert!(
        got.plain
            .contains("Fantasy awaits you in MajorMUD v1.11p-WG!")
    );
    client.send_line("M").await.unwrap();
    client.until(r"\[MAJORMUD\]:", WAIT).await.unwrap();
    client.send_line("E").await.unwrap();
    let got = client.until(r"\[HP=35\]:", WAIT).await.unwrap();
    assert!(got.plain.contains("MajorMUD - ROL"));
    r.stop().await.unwrap();
}

// docs/auth.md "Signup (NEW)": a new account has no character yet, so it stops at the
// module menu (T-022 creates the character).
#[tokio::test]
async fn new_account_reaches_the_module_menu() {
    let (_t, url) = common::seeded(&[]).await;
    let r = server(url).await;
    let mut client = TelnetClient::connect(r.telnet_addr).await.unwrap();
    client.until("Username:", WAIT).await.unwrap();
    for (line, next) in [
        ("NEW", "Please enter a unique Username"),
        ("Brenna", "Please enter a strong Password"),
        ("s3cret", "re-enter your password"),
        ("s3cret", "e-Mail Address"),
        ("", "gender"),
        ("F", r"Make your selection \(X to exit\):"),
        ("M", r"\[MAJORMUD\]:"),
    ] {
        client.send_line(line).await.unwrap();
        client.until(next, WAIT).await.unwrap();
    }
    r.stop().await.unwrap();
}

// No character yet: a socket signup creates one, the name check is answered from the
// database (`store.rs`, T-023), the character enters the realm, and its first save
// reaches the `characters` table.
#[tokio::test]
async fn new_account_creates_a_character_and_enters_the_realm() {
    let (_t, url) = common::seeded(&[]).await;
    let r = server(url.clone()).await;
    // a 79-column window: the line-mode stat editor (80x23 ANSI gets the full-screen
    // form, T-063)
    let mut client = TelnetClient::connect(r.telnet_addr)
        .await
        .unwrap()
        .with_naws(79, 24);
    client.until("Username:", WAIT).await.unwrap();
    let script: &[(&str, &str)] = &[
        ("NEW", "Please enter a unique Username"),
        ("Brenna", "Please enter a strong Password"),
        ("s3cret", "re-enter your password"),
        ("s3cret", "e-Mail Address"),
        ("", "gender"),
        ("F", r"Make your selection \(X to exit\):"),
        ("M", r"\[MAJORMUD\]:"),
        ("E", r"Please choose your race"),
        ("1", r"Please choose your class"),
        ("1", r"Do you want to be Lawful\?"),
        ("n", r"Given name \[Brenna\]: "),
        ("", r"Family name \[\]: "),
        ("Stone", r"Strength \(0-0\) \[0\]: "),
        ("", "Intellect"),
        ("", "Willpower"),
        ("", "Agility"),
        ("", "Health"),
        ("", "Charm"),
        ("", "Hair length"),
        ("", "Hair colour"),
        ("", "Eye colour"),
        ("", r"SAVE or QUIT\? \[SAVE\]: "),
    ];
    for (line, next) in script {
        client.send_line(line).await.unwrap();
        client.until(next, WAIT).await.unwrap();
    }
    // SAVE: the name check runs and is answered; the realm prompt follows. The zeroed
    // test race and class give 0/2 + (0-50)*1/16 = -3 hit points.
    client.send_line("").await.unwrap();
    let got = client.until(r"\[HP=-3\]:", WAIT).await.unwrap();
    assert!(got.plain.contains("Please wait - Validating your name."));
    r.stop().await.unwrap();

    let db = Db::connect_and_migrate(&url).await.unwrap();
    let row = db
        .characters()
        .by_first_name("Brenna")
        .await
        .unwrap()
        .expect("the first save reached the table");
    assert_eq!(row.character.lastname, "Stone");
    assert_eq!(row.character.sex, b'F'); // roll_stats copies the account's sex
    assert!(
        row.character
            .settings
            .contains(silvermere_types::flags::PlayerSettings::VALIDATED)
    );
    db.close().await;
}

// T-118: the login art, then the usual bytes up to `Username:`, unchanged.
#[tokio::test]
async fn login_art_comes_before_the_username_prompt() {
    let (_t, url) = common::seeded(&[]).await;
    let r = server(url).await;
    let mut client = TelnetClient::connect(r.telnet_addr).await.unwrap();
    let got = client.until("Username: ", WAIT).await.unwrap();
    let art = silvermere_auth::art::DEFAULT;
    let at = got
        .raw
        .windows(art.len())
        .position(|w| w == art)
        .expect("the login art is sent");
    let prompts: String = [
        silvermere_auth::AuthMsg::WelcomeEnd,
        silvermere_auth::AuthMsg::LoginIntro,
        silvermere_auth::AuthMsg::UsernamePrompt,
    ]
    .iter()
    .map(|m| m.text())
    .collect();
    assert_eq!(
        &got.raw[at + art.len()..],
        silvermere_text::cp437::encode(&prompts).as_slice()
    );
    r.stop().await.unwrap();
}

// T-115: a stray Enter at either prompt re-asks it and counts toward nothing; more of them
// than the three-attempt limit still leaves the login open.
#[tokio::test]
async fn empty_lines_at_the_login_prompts_reprompt_without_counting() {
    let (_t, url) = common::seeded(&[]).await;
    {
        let db = Db::connect_and_migrate(&url).await.unwrap();
        create_with_cost(&db, "Aldric", "aldric1", &[Role::Player], Cost::Low)
            .await
            .unwrap();
        db.close().await;
    }
    let r = server(url).await;
    let mut client = TelnetClient::connect(r.telnet_addr).await.unwrap();
    client.until("Username:", WAIT).await.unwrap();
    for stray in ["", "   ", "", "\t "] {
        client.send_line(stray).await.unwrap();
        client.until("Username:", WAIT).await.unwrap();
    }
    client.send_line("Aldric").await.unwrap();
    client.until("Password:", WAIT).await.unwrap();
    for stray in ["", " ", "", "  "] {
        client.send_line(stray).await.unwrap();
        client.until("Password:", WAIT).await.unwrap();
    }
    client.send_line("aldric1").await.unwrap();
    let got = client
        .until(r"\[MAJORMUD\]:|Make your selection|gender|Invalid", WAIT)
        .await
        .unwrap();
    assert!(!got.plain.contains("Invalid"), "{}", got.plain);
    r.stop().await.unwrap();
}

// T-115: raw line ends in front of the password: a bare CR, and LF then CR, each re-ask
// `Password:` (an LF alone is swallowed by the line editor after a CR; LF+CR ends a blank line).
#[tokio::test]
async fn bare_line_ends_before_the_password_reprompt() {
    let (_t, url) = common::seeded(&[]).await;
    {
        let db = Db::connect_and_migrate(&url).await.unwrap();
        create_with_cost(&db, "Aldric", "aldric1", &[Role::Player], Cost::Low)
            .await
            .unwrap();
        db.close().await;
    }
    let r = server(url).await;
    let mut client = TelnetClient::connect(r.telnet_addr).await.unwrap();
    client.until("Username:", WAIT).await.unwrap();
    client.send_line("Aldric").await.unwrap();
    client.until("Password:", WAIT).await.unwrap();
    for raw in [&b"\r"[..], &b"\n\r"[..]] {
        client.send_raw(raw).await.unwrap();
        client.until("Password:", WAIT).await.unwrap();
    }
    client.send_line("aldric1").await.unwrap();
    let got = client
        .until(r"Make your selection|Invalid", WAIT)
        .await
        .unwrap();
    assert!(!got.plain.contains("Invalid"), "{}", got.plain);
    r.stop().await.unwrap();
}
