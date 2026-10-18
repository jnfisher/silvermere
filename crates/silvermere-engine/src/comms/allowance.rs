//! The per-period message allowances.
//!
//! Each character may receive a limited number of telepaths, gossip lines and auction
//! lines per medium cycle (3 s): `pagecnt`/`gosscnt`/`auctcnt` count them and
//! `pagemax`/`gossmax`/`auctmax` (default 3, `SET MAXPAGES/MAXGOSSIP/MAXAUCTION`, saved
//! with the character) cap them. The counters live in the character record;
//! `cycles::medium` zeroes them each pass. The test is `count <= max`, so one
//! more than the maximum gets through each period (sic). The gossip and auction checks
//! are in `tell::tell_game`, per listener; the page check is here.

use silvermere_types::records::Character;

/// The default maximum of each allowance, set when a character is created.
pub const DEFAULT_MAX: u8 = 3;

/// The telepath check: the receiver still has a page left this period.
pub fn page_left(plr: &Character) -> bool {
    plr.pagecnt <= plr.pagemax
}

/// One more page received this period. The counter wraps from 255 back to 0 (sic).
pub fn count_page(plr: &mut Character) {
    plr.pagecnt = plr.pagecnt.wrapping_add(1);
}
