#![cfg(unix)] // ift serve is unix-only
//! The app's takeover from a running `ift serve` (#438), with a real serve
//! process: `stop_running_serve` is what the app calls before it binds its
//! sockets. The app itself is not started; the function is.
//!
//! The data dir is scratch and set for this whole process before anything
//! reads it. One test function: the steps share one serve at a time and the
//! environment is process-wide.

mod common;
use common::{profile_dir, Scratch};

use infiniterm_core::headless::{live_serve, stop_running_serve, ServeStop};
use std::process::{Command, Stdio};
use std::time::Duration;

fn ift(data: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ift"))
        .args(args)
        .env("INFINITERM_DATA_DIR", data)
        .env("HOME", data)
        .env(
            "PATH",
            format!(
                "{}:{}",
                profile_dir().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn the_app_asks_a_running_serve_to_save_and_go_and_is_never_fooled_by_a_stale_file() {
    let work = Scratch::new("takeover");
    let data = work.0.clone();
    let pid_file = data.join("serve.pid");

    // Nothing running, and a stale pid file naming an unrelated live process:
    // nothing is signalled.
    assert_eq!(
        stop_running_serve(&pid_file, Duration::from_millis(200)),
        ServeStop::NotRunning
    );
    let mut bystander = Command::new("sleep").arg("30").spawn().unwrap();
    std::fs::write(&pid_file, bystander.id().to_string()).unwrap();
    assert_eq!(
        stop_running_serve(&pid_file, Duration::from_millis(200)),
        ServeStop::NotRunning
    );
    assert!(bystander.try_wait().unwrap().is_none(), "the bystander lives");
    let _ = bystander.kill();
    let _ = bystander.wait();
    let _ = std::fs::remove_file(&pid_file);

    // A real serve: the app's call saves the canvas and waits for it to go.
    let out = ift(&data, &["serve", "--detach"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(live_serve(&pid_file).is_some(), "serve is running");
    assert_eq!(
        stop_running_serve(&pid_file, Duration::from_secs(10)),
        ServeStop::Stopped
    );
    assert!(live_serve(&pid_file).is_none());
    assert!(data.join("workspace.json").is_file(), "the canvas was saved");
    assert!(!data.join("j.sock").exists(), "the sockets are free for the app");

    // A serve that does not stop (here: suspended, so the signal stays
    // pending) is reported with its pid, not waited for forever.
    let out = ift(&data, &["serve", "--detach"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let pid = live_serve(&pid_file).expect("serve is running again");
    let _ = Command::new("kill").args(["-STOP", &pid.to_string()]).status();
    assert_eq!(
        stop_running_serve(&pid_file, Duration::from_millis(600)),
        ServeStop::Stuck(pid)
    );
    // Cleanup: let it die (the pending TERM now arrives) and wait for it.
    let _ = Command::new("kill").args(["-CONT", &pid.to_string()]).status();
    for _ in 0..100 {
        if live_serve(&pid_file).is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
}
