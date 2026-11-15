//! Checking an entry against its field: type, `MIN=`, `MAX=` and the choices.

use crate::MAX_HELP;
use crate::form::Field;
use crate::template::Kind;

/// Does the entry suit the field's type? Not for a multiple-choice field, whose entry must
/// be a choice.
pub(crate) fn type_ok(field: &Field, answer: &[u8]) -> bool {
    match field.place.kind {
        Kind::Text => !field.spec.multichoice,
        Kind::Number => {
            let digits = answer.strip_prefix(b"-").unwrap_or(answer);
            !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
        }
    }
}

/// Checks the entry against `MIN=`; the message if it is below (as `FSD.C`'s `chkmin`).
/// For text a `MIN=` of up to two digits, no more than the width, is a minimum length.
pub(crate) fn min_ok(field: &Field, answer: &[u8]) -> Result<(), Vec<u8>> {
    let Some(min) = &field.spec.min else {
        return Ok(());
    };
    match field.place.kind {
        Kind::Number => {
            let min = scan_long(min);
            if min <= scan_long(answer) {
                Ok(())
            } else {
                Err(format!("Enter at least {min}").into_bytes())
            }
        }
        Kind::Text => {
            if min.len() <= 2 && min.iter().all(u8::is_ascii_digit) {
                let len = min.iter().fold(0, |n, d| n * 10 + usize::from(d - b'0'));
                if len <= field.place.width {
                    return if answer.len() >= len {
                        Ok(())
                    } else {
                        Err(format!("Enter at least {len} character(s)").into_bytes())
                    };
                }
            }
            if min.to_ascii_lowercase() <= answer.to_ascii_lowercase() {
                Ok(())
            } else {
                Err(quoted(b"Enter at least \"", min, MAX_HELP - 17))
            }
        }
    }
}

/// Checks the entry against `MAX=`; the message if it is above (as `FSD.C`'s `chkmax`).
pub(crate) fn max_ok(field: &Field, answer: &[u8]) -> Result<(), Vec<u8>> {
    let Some(max) = &field.spec.max else {
        return Ok(());
    };
    match field.place.kind {
        Kind::Number => {
            let max = scan_long(max);
            if scan_long(answer) <= max {
                Ok(())
            } else {
                Err(format!("Enter no higher than {max}").into_bytes())
            }
        }
        Kind::Text => {
            if answer.to_ascii_lowercase() <= max.to_ascii_lowercase() {
                Ok(())
            } else {
                Err(quoted(b"Enter no higher than \"", max, MAX_HELP - 23))
            }
        }
    }
}

/// `lead`, then `value` cut to `cut` bytes, then a closing quote.
fn quoted(lead: &[u8], value: &[u8], cut: usize) -> Vec<u8> {
    let mut msg = lead.to_vec();
    msg.extend_from_slice(&value[..value.len().min(cut)]);
    msg.push(b'"');
    msg
}

/// What [`match_choices`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChoiceMatch {
    /// Choices the entry could stand for.
    pub(crate) count: usize,
    /// The first of them.
    pub(crate) first: Option<usize>,
    /// The entry: the choice itself when exactly one matched, else blanks removed.
    pub(crate) answer: Vec<u8>,
    /// The "Choices:  ..." report, when asked for, for a multiple-choice entry that is not
    /// exactly one choice; empty otherwise.
    pub(crate) report: Vec<u8>,
}

/// Looks the entry (blanks removed) up among the field's choices (as `FSD.C`'s
/// `chkalt`): a choice matches when the entry is a prefix of it, any case. With
/// `report_width` the candidates are listed in that much room.
pub(crate) fn match_choices(
    field: &Field,
    answer: &[u8],
    report_width: Option<usize>,
) -> ChoiceMatch {
    let choices = &field.spec.choices;
    if choices.is_empty() {
        return ChoiceMatch {
            answer: answer.to_vec(),
            ..ChoiceMatch::default()
        };
    }
    let answer: Vec<u8> = answer
        .iter()
        .copied()
        .filter(|&b| !b.is_ascii_whitespace() && b != 0x0B)
        .collect();
    let initial = |text: &[u8]| text.first().map_or(0, u8::to_ascii_uppercase);
    let mut found = ChoiceMatch::default();
    let mut report = b"Choices:  ".to_vec();
    let room = report_width.map_or(0, |w| w.saturating_sub(3));
    let mut full = false;
    for (i, choice) in choices.iter().enumerate() {
        if !starts_with_any_case(choice, &answer) {
            continue;
        }
        if initial(&answer) == initial(choice) {
            found.first.get_or_insert(i);
            found.answer.clone_from(choice);
            found.count += 1;
        }
        if report_width.is_some() && !full {
            if report.len() + choice.len() + 2 <= room {
                report.extend_from_slice(choice);
                report.extend_from_slice(b"  ");
            } else {
                report.extend_from_slice(b"...");
                full = true;
            }
        }
    }
    if found.count != 1 {
        found.answer = answer;
        if report_width.is_some() && field.spec.multichoice {
            found.report = report;
        }
    }
    found
}

/// The first choice that is `text`, any case (as `FSD.C`'s `ck4alt`, whole word).
pub(crate) fn exact_choice(field: &Field, text: &[u8]) -> Option<usize> {
    if text.is_empty() {
        return None;
    }
    field
        .spec
        .choices
        .iter()
        .position(|c| c.eq_ignore_ascii_case(text))
}

/// The first choice that starts with `text`, any case (as `FSD.C`'s `ck4alt`, prefix).
pub(crate) fn prefix_choice(field: &Field, text: &[u8]) -> Option<usize> {
    if text.is_empty() {
        return None;
    }
    field
        .spec
        .choices
        .iter()
        .position(|c| starts_with_any_case(c, text))
}

fn starts_with_any_case(text: &[u8], prefix: &[u8]) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// `sscanf(s, "%ld", &n)` with a 32-bit `long`: an optional sign, then digits up to the
/// first non-digit, wrapping on overflow; 0 without digits.
fn scan_long(s: &[u8]) -> i32 {
    let (neg, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n = digits
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .fold(0u32, |n, d| {
            n.wrapping_mul(10).wrapping_add(u32::from(d - b'0'))
        });
    (if neg { n.wrapping_neg() } else { n }) as i32
}

#[cfg(test)]
mod tests {
    use super::{match_choices, max_ok, min_ok, scan_long, type_ok};
    use crate::form::Field;
    use crate::spec::parse;
    use crate::template::{Kind, Place};

    fn field(spec: &[u8], kind: Kind) -> Field {
        Field {
            spec: parse(spec).remove(0),
            place: Place {
                kind,
                width: 10,
                ..Place::default()
            },
            ..Field::default()
        }
    }

    #[test]
    fn scan_long_reads_like_sscanf() {
        for (s, n) in [("42abc", 42), ("-17", -17), ("+5", 5), ("x1", 0), ("-", 0)] {
            assert_eq!(scan_long(s.as_bytes()), n);
        }
        assert_eq!(scan_long(b"4294967297"), 1);
    }

    #[test]
    fn types() {
        let number = field(b"N", Kind::Number);
        assert!(type_ok(&number, b"-12"));
        assert!(!type_ok(&number, b"-"));
        assert!(!type_ok(&number, b"1a"));
        assert!(type_ok(&field(b"T", Kind::Text), b"a b"));
        assert!(!type_ok(&field(b"T(MULTICHOICE)", Kind::Text), b"X"));
    }

    #[test]
    fn minimum_and_maximum_messages() {
        let n = field(b"N(MIN=10, MAX=250)", Kind::Number);
        assert_eq!(min_ok(&n, b"9"), Err(b"Enter at least 10".to_vec()));
        assert_eq!(
            max_ok(&n, b"251"),
            Err(b"Enter no higher than 250".to_vec())
        );
        assert_eq!(max_ok(&n, b"250"), Ok(()));
        let s = field(b"S(MIN=3)", Kind::Text);
        let msg = b"Enter at least 3 character(s)".to_vec();
        assert_eq!(min_ok(&s, b"ab"), Err(msg));
        let t = field(b"T(MIN=bb MAX=dd)", Kind::Text);
        assert_eq!(min_ok(&t, b"BA"), Err(b"Enter at least \"bb\"".to_vec()));
        assert_eq!(
            max_ok(&t, b"e"),
            Err(b"Enter no higher than \"dd\"".to_vec())
        );
    }

    #[test]
    fn choices_by_prefix_with_a_report() {
        let f = field(b"C(ALT=Red ALT=Rose ALT=Blue MULTICHOICE)", Kind::Text);
        let m = match_choices(&f, b"r o", None);
        assert_eq!((m.count, m.first, m.answer), (1, Some(1), b"Rose".to_vec()));
        let m = match_choices(&f, b"r", Some(80));
        assert_eq!((m.count, m.first), (2, Some(0)));
        assert_eq!(m.report, b"Choices:  Red  Rose  ");
        let m = match_choices(&f, b"", Some(20));
        assert_eq!(m.count, 0);
        assert_eq!(m.report, b"Choices:  Red  ...");
    }
}
