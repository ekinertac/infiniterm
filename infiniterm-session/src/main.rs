//! `iftd`'s entry point, and the one place that knows it is a unix program.
//!
//! The daemon itself is `daemon.rs`: a fork, a setsid, a unix socket and a
//! ring of raw bytes. None of that has a Windows spelling, and ConPTY is
//! itself the second emulator the whole design exists to route around (see
//! docs/windows-handoff.md, "No daemon"). So on Windows this binary exists
//! and refuses, rather than the crate being dropped from the workspace:
//! workspace members cannot be cfg'd, and `cargo test` at the root builds
//! every one of them.
//!
//! The app never reaches this refusal. `terminal.backend` cannot be
//! `daemon` on Windows at all — `Panes::start` falls back to local shells
//! and says so — so a person only sees this by running `iftd` by hand.

#[cfg(unix)]
mod daemon;

#[cfg(unix)]
fn main() {
    daemon::run();
}

#[cfg(windows)]
fn main() {
    eprintln!("iftd is macOS only; on Windows a card owns its shell directly");
    std::process::exit(2);
}
