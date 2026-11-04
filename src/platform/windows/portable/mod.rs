//! The parts of the Windows shell that make no system calls: colors, the
//! tray glyph, dropdown row text and proxy settings parsing. They build on
//! every platform so their tests run anywhere.

pub mod color;
pub mod glyph;
pub mod menu_text;
pub mod wininet;
