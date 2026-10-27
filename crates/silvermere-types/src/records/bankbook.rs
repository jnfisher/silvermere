//! The `bankbook` record (WCCBANKS).

use serde::{Deserialize, Serialize};

use crate::ids::ShopNo;
use crate::records::layout::{self, Layout};

/// A bank account: the `bankbook` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankBook {
    /// +0x000 owner (key 0 part 1).
    pub userid: String,
    /// +0x01E the bank's shop number (key 0 part 2).
    pub bank: ShopNo,
    /// +0x022 money on deposit.
    pub balance: u32,
    /// +0x026 money owed.
    pub loan: u32,
}

impl BankBook {
    /// Byte layout of the `bankbook` record.
    pub const LAYOUT: &'static Layout = &layout::BANKBOOK;
    /// The `bankbook` record length: 72 bytes.
    pub const RECORD_LEN: usize = 72;
}
