//! A Btrieve 5 writer for test fixtures (T-107): a live realm's `.DAT` built from a stock
//! `.VIR`.
//!
//! The template is a stock `.VIR` with no records. Its pages (the FCR, the key and
//! alternate collating pages, any empty data page) are kept byte for byte, as Btrieve wrote
//! them; data pages holding the records are appended, and the FCR's record count (+0x1A)
//! and page count (+0x26) are updated. A data page is a 6-byte header (the page number as
//! a high and a low word, then the usage count with the data-page bit 0x8000) and the
//! records in slot order, each padded to the physical record length with zeros (the
//! duplicate-key pointers of a key without duplicates). The index pages are not rebuilt:
//! the reader walks data pages and never reads an index, so the file reads as the realm's
//! would, but Btrieve itself would need its indexes rebuilt (`BUTIL -CLONE`/`-COPY`)
//! before opening it.

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn put_hi_lo(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 2].copy_from_slice(&((v >> 16) as u16).to_le_bytes());
    b[at + 2..at + 4].copy_from_slice(&(v as u16).to_le_bytes());
}

/// The template with `records` appended (each exactly the template's record length).
pub fn write_v5(template: &[u8], records: &[Vec<u8>]) -> Vec<u8> {
    assert_ne!(&template[..4], b"FC\0\0", "a v5 template");
    let page_length = usize::from(u16_at(template, 0x08));
    let record_length = usize::from(u16_at(template, 0x16));
    let physical = usize::from(u16_at(template, 0x18));
    assert_eq!(template.len() % page_length, 0);
    let per_page = (page_length - 6) / physical;
    let mut out = template.to_vec();
    let mut pages = (template.len() / page_length) as u32;
    for chunk in records.chunks(per_page) {
        let mut page = vec![0u8; page_length];
        put_hi_lo(&mut page, 0, pages);
        page[4..6].copy_from_slice(&0x8001u16.to_le_bytes());
        for (slot, r) in chunk.iter().enumerate() {
            assert_eq!(r.len(), record_length, "record length");
            assert!(
                r[4..].iter().any(|&b| b != 0),
                "a slot that reads as unused"
            );
            let at = 6 + slot * physical;
            page[at..at + record_length].copy_from_slice(r);
        }
        out.extend_from_slice(&page);
        pages += 1;
    }
    put_hi_lo(&mut out, 0x1A, records.len() as u32);
    put_hi_lo(&mut out, 0x26, pages);
    out
}

/// The template with its record length (+0x16) changed, for a file of another struct
/// whose records still fit the physical length.
pub fn with_record_length(template: &[u8], len: u16) -> Vec<u8> {
    let mut out = template.to_vec();
    out[0x16..0x18].copy_from_slice(&len.to_le_bytes());
    out
}
