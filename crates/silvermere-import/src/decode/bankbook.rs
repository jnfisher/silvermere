//! WCCBANKS bank books: the `bankbook` record, state. Not carried: `dirty`
//! (cache byte), `unk_0x02b`.

use silvermere_types::ids::ShopNo;
use silvermere_types::records::{BankBook, layout};

use super::{Raw, Text, record};

record! {
    /// The `bankbook` record, WCCBANKS: every member, in record order.
    pub struct BankbookRecord = layout::BANKBOOK {
        userid: Text<30>,
        bank: i32,
        balance: u32,
        loan: u32,
        dirty: u8,
        unk_0x02b: Raw<29>,
    }
}

/// The schema's bank book.
pub fn to_bank_book(r: &BankbookRecord) -> BankBook {
    BankBook {
        userid: r.userid.trimmed(),
        bank: ShopNo(r.bank),
        balance: r.balance,
        loan: r.loan,
    }
}
