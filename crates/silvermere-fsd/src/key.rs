//! Keystroke codes for [`Form::key`](crate::Form::key): a byte 0x20-0xFF is itself;
//! control keys are their ASCII code; the PC's extended keys are `0x100 | scan code`.

/// ^F: insert a blank.
pub const CTRL_F: u16 = 0x06;
/// ^G: save and leave.
pub const CTRL_G: u16 = 0x07;
/// Backspace.
pub const BS: u16 = 0x08;
/// Tab: next field.
pub const TAB: u16 = 0x09;
/// ^L: redisplay (left to the host).
pub const CTRL_L: u16 = 0x0C;
/// Return: next field.
pub const CR: u16 = 0x0D;
/// ^O: quit.
pub const CTRL_O: u16 = 0x0F;
/// ^U: previous field.
pub const CTRL_U: u16 = 0x15;
/// Escape: quit.
pub const ESC: u16 = 0x1B;
/// The DEL character.
pub const RUBOUT: u16 = 0x7F;
/// Shift-Tab: previous field.
pub const BAKTAB: u16 = 0x10F;
/// Home.
pub const HOME: u16 = 0x147;
/// Cursor up.
pub const CRSRUP: u16 = 0x148;
/// Cursor left.
pub const CRSRLF: u16 = 0x14B;
/// Cursor right.
pub const CRSRRT: u16 = 0x14D;
/// End.
pub const END: u16 = 0x14F;
/// Cursor down.
pub const CRSRDN: u16 = 0x150;
/// The Delete key.
pub const DEL: u16 = 0x153;
/// Ctrl-End: last field.
pub const CTRLEND: u16 = 0x175;
/// Ctrl-PgDn: last field.
pub const CTRLPGDN: u16 = 0x176;
/// Ctrl-Home: first field.
pub const CTRLHOME: u16 = 0x177;
/// Ctrl-PgUp: first field.
pub const CTRLPGUP: u16 = 0x184;
