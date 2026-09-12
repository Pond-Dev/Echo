//! Echo — a combat assistance tool for CS2, built one step at a time.
//!
//! Everything lives here rather than in the binary so that it can be tested
//! without elevation: the executable is marked `requireAdministrator`, and a
//! test harness built from a binary target inherits that mark and then cannot
//! be launched at all. The binary is a thin shell over this crate.
//!
//! The one structural rule so far:
//!
//! * [`process`] knows how to read another process, and nothing about CS2.
//! * [`game`] is the only module that knows CS2 exists.
//!
//! A game update therefore reaches exactly one file, `game::offsets`.

pub mod aim;
pub mod app;
pub mod game;
pub mod input;
pub mod log;
pub mod overlay;
pub mod process;
