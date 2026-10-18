//! WCCSHOPS shops: the `shop` record.
//!
//! `qty[20]` (+0x14F) is the live stock: the writer stores it in `shop_stock`
//! ([`ShopStock`]). Not carried: `dirty` (cache byte), `unk_0x1dc`.

use silvermere_db::repo::ShopStock;
use silvermere_types::enums::ShopType;
use silvermere_types::ids::{ClassNo, ItemNo, ShopNo};
use silvermere_types::records::{Shop, ShopSlot, layout};

use super::{Raw, Text, record};

record! {
    /// The `shop` record, WCCSHOPS: every member, in record order.
    pub struct ShopRecord = layout::SHOP {
        number: i32,
        name: Text<40>,
        desc: [Text<53>; 3],
        type_ = "type": i16,
        minlevel: u16,
        maxlevel: u16,
        markup: u16,
        ownable: u8,
        expandable: u8,
        trainclass: u16,
        items: [i32; 20],
        maxqty: [u16; 20],
        qty: [u16; 20],
        regentime: [u16; 20],
        regennum: [i16; 20],
        regenpct: [u8; 20],
        dirty: u8,
        unk_0x1dc: Raw<24>,
    }
}

/// The schema's shop and its initial stock.
pub fn split_shop(r: &ShopRecord) -> (Shop, ShopStock) {
    let shop = Shop {
        number: ShopNo(r.number),
        name: r.name.trimmed(),
        desc: r.desc.clone().map(|t| t.trimmed()),
        shop_type: ShopType::from_code(r.type_ as u16),
        minlevel: r.minlevel,
        maxlevel: r.maxlevel,
        markup: r.markup,
        ownable: r.ownable,
        expandable: r.expandable,
        trainclass: ClassNo(r.trainclass),
        stock: std::array::from_fn(|i| ShopSlot {
            item: ItemNo(r.items[i]),
            maxqty: r.maxqty[i],
            qty: r.qty[i],
            regentime: r.regentime[i],
            regennum: r.regennum[i],
            regenpct: r.regenpct[i],
        }),
    };
    let stock = ShopStock::of(&shop);
    (shop, stock)
}
