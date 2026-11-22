//! The calculator's arithmetic: exact decimal numbers and the reading of what is typed.
//!
//! The library half of the `vergence-calc` app (tools milestone 1, docs/tools-milestone-1.md),
//! kept apart from the window so it is tested on its own. The window is main.rs, on
//! vergence-widgets, vergence-ui and vergence-look; this half uses no other crate.
//!
//! Start reading: [`dec`] (exact decimal numbers, rounded to 34 digits half to even), then
//! [`expr`] (an expression as typed, worked out), then main.rs (the keypad window).
//!
//! Rule: answers are exact decimals, never binary floating point, so 0.1 + 0.2 is 0.3.

pub mod dec;
pub mod expr;
