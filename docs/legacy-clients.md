# Legacy clients

Notes for the terminal clients MajorMUD players used (plain telnet with ANSI, MegaMud and
the like): where silvermere behaves as the original did on the wire, and where it differs
on purpose. T-062 owns the rest of this file; sections are appended by the cards that add
behaviour.

## Full-screen character editor (T-063)

Character creation and `TRAIN STATS` open the original's full-screen form (`EDITCHA1`)
when the terminal is ANSI with at least 80 columns and 23 rows.
Smaller terminals and ANSI-off sessions get the line-mode editor (T-022, T-047). A client
that never answers NAWS is taken to be 80x24, which is what MBBSEmu assumed, so it gets the
form. Both editors run the same checks and the same finish routine,
so they always produce the same character from the same answers.

The form is driven one key at a time. The server switches the connection to character
mode while the form is open and back to line mode when it closes.

| Key | Effect |
|---|---|
| letters, digits (digits only in a stat) | the first key replaces the field's value; each key is echoed |
| Backspace, DEL | deletes the last character typed |
| Enter, down arrow | checks the field, then moves to the next one; a refusal shows on row 23 and the cursor stays |
| up arrow | checks the field, then moves to the previous one |
| Space on hair length, hair colour, eye colour | the next value, redrawn in upper case |
| Space on Exit | cycles SAVE, EDIT, QUIT |
| Enter on Exit | SAVE or QUIT leaves the form; EDIT goes back to the first field with the answers kept |

The cursor positions are the original's: given name `4;20`, family name `5;20`, stats rows
9-14 at column 34, hair length `16;22`, hair colour `17;22`, eye colour `18;22`, Exit
`20;14`. The CP still to spend is redrawn at `20;33` after the family name and after each
accepted stat. Every move is an absolute `ESC[row;colf`.

Deliberate differences from the original under MBBSEmu:

- **Arrow keys never leave the form.** Earlier notes recorded the arrows as
  exiting the editor; the 2026-10-14 capture showed up and down moving between fields
  instead, and that is what silvermere does. Left, right and every other escape sequence
  are swallowed, so a stray key from a client macro cannot close the form or land in a
  field. Original: an unknown sequence could swallow the next few keys.
- **The start field is always the same**: the given name for a new character, the family
  name for `TRAIN STATS` (the first name is locked for an existing character,
  so the original skipped it). Under MBBSEmu the start
  field varied between sessions.
- A refused answer stays in the field and the cursor goes back to its start, so typing
  replaces it. The capture never showed a refusal, so this part of the original's
  behaviour is inferred.
- **EDIT** on the Exit field redraws Exit as `SAVE` and focuses the first field the cursor
  visits, with every answer kept. The capture never chose EDIT, so this redraw is
  inferred too.
- `Your family name can only be 18 characters long!` never appears on the form: the field
  is 18 characters wide and the checks strip blanks before they test the length.
  The line-mode editor can still show it.

Bots and scripts should keep using the line-mode editor: connect with a window narrower
than 80 columns (NAWS 79x24, as the `creation-line-mode` capture did). Bots never run
`TRAIN STATS`.
