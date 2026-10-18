//! The metric names and their help texts. Values are recorded through the `metrics`
//! facade (calls are no-ops without a recorder); `docs/metrics.md` says how to read each.
//! No label takes an unbounded value: no character names, no raw command text.

use metrics::{Unit, counter, describe_counter, describe_gauge, describe_histogram};
use silvermere_types::enums::CommandCode as Cmd;

pub const SESSIONS: &str = "silvermere_sessions";
pub const COMMANDS_TOTAL: &str = "silvermere_commands_total";
/// Kept at the name `silvermere-api` documents for T-061's soak.
pub const TICK_SECONDS: &str = "silvermere_tick_seconds";
pub const PERSIST_QUEUE_DEPTH: &str = "silvermere_persist_queue_depth";
/// Kept at the name `silvermere-api` documents; label `op`.
pub const DB_LATENCY_SECONDS: &str = "silvermere_db_latency_seconds";
pub const LOGINS_TOTAL: &str = "silvermere_logins_total";
pub const DEATHS_TOTAL: &str = "silvermere_deaths_total";
pub const LEVELUPS_TOTAL: &str = "silvermere_levelups_total";
pub const HANDLER_PANICS_TOTAL: &str = "silvermere_handler_panics_total";
pub const OUTPUT_DROPPED_TOTAL: &str = "silvermere_output_dropped_total";
pub const PERSIST_ERRORS_TOTAL: &str = "silvermere_persist_errors_total";
pub const BUILD_INFO: &str = "silvermere_build_info";

/// The values of the `family` label of [`COMMANDS_TOTAL`].
pub const FAMILIES: [&str; 7] = [
    "movement", "combat", "info", "comms", "shop", "sysop", "other",
];

/// Registers every help text. Call once at start-up, after the recorder is installed.
pub fn describe_all() {
    describe_gauge!(SESSIONS, "Sessions connected to the engine");
    describe_counter!(COMMANDS_TOTAL, "Commands run, by family");
    describe_histogram!(TICK_SECONDS, Unit::Seconds, "Fast tick duration");
    describe_gauge!(
        PERSIST_QUEUE_DEPTH,
        "Writes queued for the persistence worker"
    );
    describe_histogram!(
        DB_LATENCY_SECONDS,
        Unit::Seconds,
        "Persistence batch commit latency, by op"
    );
    describe_counter!(LOGINS_TOTAL, "Characters that entered the realm");
    describe_counter!(DEATHS_TOTAL, "Player deaths");
    describe_counter!(LEVELUPS_TOTAL, "Player level gains");
    describe_counter!(HANDLER_PANICS_TOTAL, "Event handlers that panicked");
    describe_counter!(
        OUTPUT_DROPPED_TOTAL,
        "Writes shed because a session's bounded queue was full"
    );
    describe_counter!(
        PERSIST_ERRORS_TOTAL,
        "Persistence batches that failed and were retried"
    );
    describe_gauge!(BUILD_INFO, "Always 1; labels carry the version and backend");
    // Export every command-family series from the start, at 0, so a scrape never depends on
    // whether a command of that family has already been dispatched.
    for f in FAMILIES {
        counter!(COMMANDS_TOTAL, "family" => f).increment(0);
    }
}

/// The family of a command code, for [`COMMANDS_TOTAL`]: the module its handler lives in
/// (`commands::dispatch`), banks counted with shops.
pub fn family(cmd: Cmd) -> &'static str {
    use Cmd::*;
    match cmd {
        North | South | East | West | NorthEast | NorthWest | SouthEast | SouthWest | Up | Down
        | Open | Close | Lock | Drag => "movement",
        Punch | Kick | Jumpkick | Bash | Smash | Killblow | Backstab | Attack | Break => "combat",
        Help | Look | Map | Status | Health | Inventory | Powers | Spells | Keys | Identity
        | Topten | Scan | Exits | Profile | Experience | Version | Wealth => "info",
        Broadcast | Whisper | Feedback | DirectMsg | Gossip | Auction | Global => "comms",
        List | Stock | Unstock | MarkUp | Buy | Sell | Appraise | Deposit | Withdraw | Borrow
        | Repay | BankBook => "shop",
        Sysop | Purge => "sysop",
        _ => "other",
    }
}

/// Counts one command that reached the dispatcher.
pub fn command(cmd: Cmd) {
    counter!(COMMANDS_TOTAL, "family" => family(cmd)).increment(1);
}

/// Counts a character entering the realm.
pub fn login() {
    counter!(LOGINS_TOTAL).increment(1);
}

/// Counts a player death. TODO(T-033): call it where `check_kill_user` applies a death.
pub fn death() {
    counter!(DEATHS_TOTAL).increment(1);
}

/// Counts a level gain (`World::level_gained`, from `train_level`).
pub fn levelup() {
    counter!(LEVELUPS_TOTAL).increment(1);
}
