//! The field specification: names and options.

use crate::MAX_ANSWER;

/// One field of the specification.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FieldSpec {
    pub(crate) name: Vec<u8>,
    /// `MIN=`: a number, a minimum length (text, 1-2 digits) or a lowest string.
    pub(crate) min: Option<Vec<u8>>,
    /// `MAX=`: a number or a highest string.
    pub(crate) max: Option<Vec<u8>>,
    /// The `ALT=` values, in order.
    pub(crate) choices: Vec<Vec<u8>>,
    /// `MULTICHOICE`: only a choice is an answer, and keys pick one.
    pub(crate) multichoice: bool,
    /// A `MIN=` not starting with `-`: no minus sign can be typed.
    pub(crate) no_negative: bool,
}

/// Parses a specification (as the library's `fspscn` and `chkops`): names end at white space
/// or `(`; options run to `)`. Stops at a NUL.
pub(crate) fn parse(spec: &[u8]) -> Vec<FieldSpec> {
    let end = spec.iter().position(|&b| b == 0).unwrap_or(spec.len());
    let spec = &spec[..end];
    let mut fields = Vec::new();
    let mut i = 0;
    loop {
        while spec.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if i >= spec.len() {
            return fields;
        }
        let start = i;
        while i < spec.len() && !spec[i].is_ascii_whitespace() && spec[i] != b'(' {
            i += 1;
        }
        let mut field = FieldSpec {
            name: spec[start..i].to_vec(),
            ..FieldSpec::default()
        };
        if spec.get(i) == Some(&b'(') {
            let opts_start = i + 1;
            i = opts_start;
            while i < spec.len() && spec[i] != b')' {
                i += 1;
            }
            field.read_options(&spec[opts_start..i]);
            i += 1;
        }
        fields.push(field);
    }
}

impl FieldSpec {
    /// Reads the words of an option list. An option is a word whose first letter is as
    /// written and the rest any case; the first `MIN=` and `MAX=` count.
    fn read_options(&mut self, opts: &[u8]) {
        for word in opts.split(u8::is_ascii_whitespace) {
            if let Some(value) = option(word, b"MIN=") {
                self.min.get_or_insert(value);
            } else if let Some(value) = option(word, b"MAX=") {
                self.max.get_or_insert(value);
            } else if let Some(value) = option(word, b"ALT=") {
                self.choices.push(value);
            } else if word.len() == b"MULTICHOICE".len() && option(word, b"MULTICHOICE").is_some() {
                self.multichoice = true;
            }
        }
        self.no_negative = self.min.as_ref().is_some_and(|min| !min.starts_with(b"-"));
    }
}

/// The value after `token` if `word` starts with it, cut to [`MAX_ANSWER`].
fn option(word: &[u8], token: &[u8]) -> Option<Vec<u8>> {
    let head = word.get(..token.len())?;
    (head[0] == token[0] && head.eq_ignore_ascii_case(token))
        .then(|| word[token.len()..word.len().min(token.len() + MAX_ANSWER)].to_vec())
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn names_and_options() {
        let fields = parse(b"A() B(MIN=10, MAX=250) C(ALT=x Alt=y aLT=z MULTICHOICE)  D\0E");
        let names: Vec<&[u8]> = fields.iter().map(|f| f.name.as_slice()).collect();
        assert_eq!(names, [&b"A"[..], b"B", b"C", b"D"]);
        assert_eq!(fields[1].min.as_deref(), Some(&b"10,"[..]));
        assert_eq!(fields[1].max.as_deref(), Some(&b"250"[..]));
        assert!(fields[1].no_negative);
        // the first letter of an option is as written
        assert_eq!(fields[2].choices, [b"x".to_vec(), b"y".to_vec()]);
        assert!(fields[2].multichoice);
        assert!(!fields[0].multichoice && fields[0].min.is_none());
    }

    #[test]
    fn options_are_whole_words() {
        let fields = parse(b"A(MIN=-5 MULTICHOICES XALT=1)");
        assert!(!fields[0].no_negative);
        assert!(!fields[0].multichoice);
        assert!(fields[0].choices.is_empty());
    }
}
