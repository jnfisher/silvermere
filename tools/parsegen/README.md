# parsegen

Development oracles, standard library only, not shipped.

## `parsegen.py`: the command words

Generates the oracle for `parse_command` (T-024),
`crates/silvermere-engine/tests/fixtures/command_words.json`: every letter sequence the game
tells apart as the first word of a command, with the `CMD_` code of the command it runs, plus
one longer and one wrong-letter sibling for each. The generator runs only in the private
workspace; the fixture is committed, and its `regenerate` field gives the command line.

- The enumeration is a breadth-first walk over prefixes: for each prefix it records the code,
  learns which letters the game tells apart at the next position, and expands those plus one
  wrong letter. A prefix the game never reads past is a leaf, pinned by one unexpanded longer
  sibling.
- `crates/silvermere-engine/tests/parse_command.rs` feeds every entry through the Rust parser,
  so a table mistake fails loudly.

## `expneeded.py`: experience per level

Writes `crates/silvermere-rules/tests/fixtures/expneeded.json`, the experience each level needs
for a range of class/race modifiers, computed in plain Python integers with the game's 32-bit
arithmetic (see the script's docstring).
