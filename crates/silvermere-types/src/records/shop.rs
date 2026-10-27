//! The `shop` record (WCCSHOPS).

use serde::{Deserialize, Serialize};

use crate::enums::ShopType;
use crate::ids::{ClassNo, ItemNo, ShopNo};
use crate::records::layout::{self, Layout};
use crate::sizes::NSHOPITEMS;

/// One stock slot of a shop: the `items`, `maxqty`, `qty`, `regentime`, `regennum` and
/// `regenpct` arrays at one index.
///
/// In a gang shop (`SHOP_GANG`) the original reuses the arrays: `maxqty` holds the owner's
/// user-id as a string, `regentime` the asking price and `regennum` its currency.
/// The decoder keeps the raw numbers; the gang shop code reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShopSlot {
    /// +0x0D7 item number sold.
    pub item: ItemNo,
    /// +0x127 restock ceiling.
    pub maxqty: u16,
    /// +0x14F quantity in stock.
    pub qty: u16,
    /// +0x177 minutes between restocks.
    pub regentime: u16,
    /// +0x19F copies per restock.
    pub regennum: i16,
    /// +0x1C7 restock success percent.
    pub regenpct: u8,
}

/// A shop: the `shop` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shop {
    /// +0x000 shop number (key 0).
    pub number: ShopNo,
    /// +0x004 shop name.
    pub name: String,
    /// +0x02C description, three lines.
    pub desc: [String; 3],
    /// +0x0CB shop type (`type`).
    pub shop_type: ShopType,
    /// +0x0CD training: lowest level.
    pub minlevel: u16,
    /// +0x0CF training: highest level.
    pub maxlevel: u16,
    /// +0x0D1 percent over item cost.
    pub markup: u16,
    /// +0x0D3 sysop dump only.
    pub ownable: u8,
    /// +0x0D4 sysop dump only.
    pub expandable: u8,
    /// +0x0D5 training: class, 0 = any.
    pub trainclass: ClassNo,
    /// The twenty stock slots.
    pub stock: [ShopSlot; NSHOPITEMS],
}

impl Shop {
    /// Byte layout of the `shop` record.
    pub const LAYOUT: &'static Layout = &layout::SHOP;
    /// The `shop` record length: 500 bytes.
    pub const RECORD_LEN: usize = 500;
}
