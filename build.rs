//! Marks the binary as needing administrator rights.
//!
//! Reading another process's memory needs elevation. Without this, it has to
//! be remembered at every launch — right-click, Run as administrator — and
//! forgetting looks like a bug rather than a missing privilege. With it,
//! Windows shows the UAC prompt on its own.
//!
//! Two details that are easy to get wrong:
//!
//! * The flags go out as `rustc-link-arg-bins`, not `rustc-link-arg`. The
//!   plain form reaches *every* target, which marks the test harness as
//!   requiring elevation too and makes `cargo test` fail to launch.
//! * `/MANIFESTUAC` is used rather than supplying a manifest file. rustc
//!   already contributes its own manifest snippet with `level="asInvoker"`,
//!   and a second snippet naming a different level is a hard linker error
//!   ("Values of attribute level not equal in different manifest snippets").
//!   This flag overrides the level instead of competing with it.
//!
//! Trade-off that remains: a binary marked `requireAdministrator` cannot be
//! started by an ordinary `CreateProcess`, so `cargo run` from a normal
//! terminal fails with "requires elevation". Run it from an elevated terminal,
//! or launch the executable directly. `cargo build`, `test` and `clippy` are
//! unaffected.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // MSVC-only linker flags, and nothing to elevate off Windows anyway.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }

    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:level='requireAdministrator'");
}
