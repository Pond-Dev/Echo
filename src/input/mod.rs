//! Relative mouse output, restricted to the game process in front.

pub mod send;
pub use send::{focused, move_by};
