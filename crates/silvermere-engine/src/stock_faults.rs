//! Known faults in the stock content, fixed as it loads.
//!
//! Each entry names one text block, the exact stock text of one of its lines and the line it
//! becomes. A block that was edited, or is missing, does not match and is left alone. The fix
//! is made on the loaded copy only, so an existing database needs no re-import and nothing a
//! player owns is written.

use std::collections::HashMap;

use silvermere_types::ids::TextBlockNo;
use silvermere_types::records::TextBlock;

/// One known fault in the stock text blocks.
pub struct Fault {
    /// The block holding the line.
    pub block: i32,
    /// The whole line as shipped.
    pub stock: &'static str,
    /// The line it is replaced by.
    pub fixed: &'static str,
    /// What was wrong, for the load log.
    pub reason: &'static str,
}

/// The known stock data faults.
pub const FAULTS: &[Fault] = &[Fault {
    block: 9578,
    stock: "go nightblack portal:roomitem 1419 1373:minlevel 40 1374:message 1375:2683 1:message 1376",
    fixed: "go nightblack portal:roomitem 1419 1373:minlevel 40 1374:message 1375:teleport 2683 1:message 1376",
    reason: "the Negative Power Plane's `go nightblack portal` lacked `teleport`, so it showed \
             its messages and left the player in place",
}];

/// Replaces `fault.stock` by `fault.fixed` where it is a whole line of `text`.
fn fix_text(text: &str, fault: &Fault) -> Option<String> {
    let mut out = String::with_capacity(text.len() + 16);
    let mut changed = false;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line == fault.stock {
            out.push_str(fault.fixed);
            changed = true;
        } else {
            out.push_str(line);
        }
    }
    changed.then_some(out)
}

/// Applies the known faults to the loaded text blocks, logging each one fixed.
pub fn apply(blocks: &mut HashMap<(i16, TextBlockNo), TextBlock>) {
    for fault in FAULTS {
        let mut fixed = false;
        for block in blocks.values_mut().filter(|b| b.number.0 == fault.block) {
            if let Some(text) = fix_text(&block.text, fault) {
                block.text = text;
                fixed = true;
            }
        }
        if fixed {
            tracing::info!(
                block = fault.block,
                "stock data fault fixed: {}",
                fault.reason
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(number: i32, text: &str) -> HashMap<(i16, TextBlockNo), TextBlock> {
        let b = TextBlock {
            part: 0,
            number: TextBlockNo(number),
            link: TextBlockNo(0),
            text: text.to_owned(),
        };
        HashMap::from([((0, b.number), b)])
    }

    #[test]
    fn fixes_only_the_exact_stock_line() {
        let f = &FAULTS[0];
        let mut m = block(f.block, &format!("go portal:x\n{}\nenter y:z", f.stock));
        apply(&mut m);
        assert_eq!(
            m[&(0, TextBlockNo(f.block))].text,
            format!("go portal:x\n{}\nenter y:z", f.fixed)
        );
    }

    #[test]
    fn leaves_an_edited_line_or_another_block_alone() {
        let f = &FAULTS[0];
        let edited = format!("{} ", f.stock);
        let mut m = block(f.block, &edited);
        apply(&mut m);
        assert_eq!(m[&(0, TextBlockNo(f.block))].text, edited);
        let mut m = block(f.block + 1, f.stock);
        apply(&mut m);
        assert_eq!(m[&(0, TextBlockNo(f.block + 1))].text, f.stock);
    }
}
