//! Echo, watching.
//!
//! The same program with its hands behind its back: it attaches, reads,
//! chooses a target, works out the movement, spends the pull, counts every
//! refusal and writes every hit — and never touches the mouse.
//!
//! A separate executable rather than a flag so that measuring without the
//! assist is as easy as measuring with it. A control nobody can be bothered
//! to run is not a control.

fn main() {
    echo::app::start(true);
}
