//! Echo, steering.
//!
//! A shell, because the work lives in the library where it can be tested.
//! This one moves the mouse; `echo-watch` beside it decides everything and
//! moves nothing, which is what any figure out of this one is measured
//! against.

fn main() {
    echo::app::start(echo::app::watching_only());
}
