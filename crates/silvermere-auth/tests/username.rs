//! Username rules: 1-29 printable ASCII characters (32-126), as a BBS user-ID and
//! MBBSEmu's signup.

use proptest::prelude::*;
use silvermere_auth::account::{USERNAME_MAX, validate_username};

proptest! {
    #[test]
    fn every_accepted_name_is_1_to_29_printable_ascii(name in "\\PC{0,40}|[ -~]{0,40}") {
        if validate_username(&name).is_ok() {
            prop_assert!(!name.is_empty() && name.len() <= USERNAME_MAX);
            prop_assert!(name.bytes().all(|b| (32..=126).contains(&b)));
        }
    }

    #[test]
    fn plain_names_up_to_29_are_accepted(name in "[A-Za-z][A-Za-z0-9]{0,28}") {
        prop_assume!(!name.eq_ignore_ascii_case("new"));
        prop_assert!(validate_username(&name).is_ok());
    }
}
