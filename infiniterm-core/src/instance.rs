//! Who owns the single-instance endpoint, recorded beside the save file.
//!
//! `transport::is_live` answers "is something listening", which on unix is
//! the whole question. On Windows it is not: a terminated process is not
//! reaped while any of its threads is stuck in the kernel, and CEF leaves
//! one that is, so the pipe an instance was serving goes on ACCEPTING
//! CONNECTIONS for about forty seconds after the app has gone (measured
//! 2026-09-25). For those forty seconds a relaunch was told the endpoint was
//! held and exited, which reads as "infiniterm will not start".
//!
//! So the owner writes its pid here when it binds and removes the file when
//! it quits, and a would-be second instance asks two questions instead of
//! one: is something listening, and does the process that claimed it still
//! exist. Both must be true to refuse.
//!
//! WHY THIS IS NOT THE MISTAKE THAT WAS MADE BEFORE. An earlier attempt
//! asked the PIPE which process served it (`GetNamedPipeServerProcessId`)
//! and let a second instance onto the same save file, because a dying
//! instance's handle lingers BESIDE a live one's and a client can land on
//! either; landing on the dead one answered "nobody is here" while somebody
//! was. This asks a file, not a handle, and the file always names the
//! instance that bound most recently — the live one, when there is a live
//! one. The dangerous direction is answering "free" while an instance runs,
//! and a running instance's pid is in the file and is alive.
//!
//! UNIX KEEPS THE PLAIN CONNECT and none of this runs there. A unix socket
//! file goes when the process does, so there is nothing to disambiguate,
//! and adding a second source of truth to a rule that already works is how
//! the rule stops working. Only `holds_the_endpoint` is compiled
//! everywhere, so the rule can be read and tested on either machine.
//!
//! Related: `transport.rs` (the endpoint itself), `hooks.rs` (which binds),
//! `infiniterm-ui/src/runtime.rs` (`another_instance_holds_the_socket`), and
//! `infiniterm-ui/src/main.rs` (the quit that calls `release`).
use std::path::{Path, PathBuf};

/// The claim file, beside the save file rather than beside the pipe: a pipe
/// name is not a path on Windows, and the data dir is what
/// `INFINITERM_DATA_DIR` already isolates per instance.
#[cfg(windows)]
pub fn claim_path(data: &Path) -> PathBuf {
    data.join("instance.pid")
}

/// Record this process as the endpoint's owner. Called after `bind`.
#[cfg(windows)]
pub fn claim(data: &Path) {
    let _ = std::fs::create_dir_all(data);
    let _ = std::fs::write(claim_path(data), std::process::id().to_string());
}

/// Give the claim up. Called on the way out, before the process ends.
///
/// Best effort on purpose: a crash never reaches this, which is what the
/// liveness check in `holds_the_endpoint` is for.
#[cfg(windows)]
pub fn release(data: &Path) {
    let _ = std::fs::remove_file(claim_path(data));
}

/// The pid in the claim file, or `None` when there is no readable claim.
#[cfg(windows)]
pub fn owner(data: &Path) -> Option<u32> {
    std::fs::read_to_string(claim_path(data))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Whether a running instance holds the endpoint, given whether something
/// answers there and who claims to own it.
///
/// The pure half, so the rule can be read and tested without a pipe. Nothing
/// listening means free, whatever the file says — a claim left by a crash is
/// not an instance. Something listening with no claim, or a claim whose
/// process is gone, is the lingering endpoint of a process that has already
/// ended.
pub fn holds_the_endpoint(
    listening: bool,
    owner: Option<u32>,
    alive: impl Fn(u32) -> bool,
) -> bool {
    listening && owner.is_some_and(alive)
}

/// Whether a process exists and has not exited.
///
/// `HasExited` is the distinction that matters and the reason this is not
/// just "does the pid resolve": a terminated process that has not been
/// reaped still opens, still appears in the process list, and still answers
/// on its pipes. `GetExitCodeProcess` reporting anything but `STILL_ACTIVE`
/// is the honest answer.
///
/// A pid we may not open (another user's, or one raised above us) counts as
/// ALIVE. Refusing to start is the safe direction: the cost is a message,
/// and the cost of the other answer is two instances on one save file.
#[cfg(windows)]
pub fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess};
    // PROCESS_QUERY_LIMITED_INFORMATION, which a process we own always
    // grants and which is enough for the exit code.
    const QUERY: u32 = 0x1000;

    let handle = unsafe { OpenProcess(QUERY, 0, pid) };
    if handle.is_null() {
        // Only "no such process" is proof of death; anything else is a
        // permission answer and counts as alive, per the doc above.
        return std::io::Error::last_os_error().raw_os_error()
            != Some(ERROR_INVALID_PARAMETER as i32);
    }
    let mut code: u32 = 0;
    let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
    unsafe { CloseHandle(handle) };
    ok != 0 && code == STILL_ACTIVE as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_listening_is_free_whatever_the_file_says() {
        assert!(!holds_the_endpoint(false, Some(42), |_| true));
        assert!(!holds_the_endpoint(false, None, |_| true));
    }

    // The case this file exists for: the pipe still answers, its owner is
    // gone. Before, that refused to launch for forty seconds.
    #[test]
    fn a_lingering_endpoint_whose_owner_has_gone_is_free() {
        assert!(!holds_the_endpoint(true, Some(42), |_| false));
    }

    // A claim removed on a clean quit, while the pipe has not caught up.
    #[test]
    fn a_lingering_endpoint_with_no_claim_is_free() {
        assert!(!holds_the_endpoint(true, None, |_| true));
    }

    // And the case that must never regress: a real instance is running.
    #[test]
    fn a_listening_endpoint_with_a_live_owner_is_held() {
        assert!(holds_the_endpoint(true, Some(42), |pid| pid == 42));
    }

    #[cfg(windows)]
    #[test]
    fn a_claim_round_trips_and_release_removes_it() {
        let dir = std::env::temp_dir().join(format!("ift-claim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        claim(&dir);
        assert_eq!(owner(&dir), Some(std::process::id()));
        release(&dir);
        assert_eq!(owner(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // This process is the one case we can assert about without a fixture.
    #[cfg(windows)]
    #[test]
    fn this_process_is_alive_and_a_pid_that_cannot_exist_is_not() {
        assert!(process_alive(std::process::id()));
        // Odd pids are never valid on Windows (they are multiples of four),
        // so this one cannot resolve to anything.
        assert!(!process_alive(u32::MAX - 2));
    }
}
