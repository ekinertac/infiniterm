//! A host with no canvas file but running sessions (#439: a Linux server that
//! never had an app) builds its canvas from them. Real `iftd`s, a scratch data
//! dir set for the whole process (a unix socket path holds 104 bytes), and the
//! installed app's sockets never touched. One test function.

use infiniterm_core::headless::{Host, HostIo};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn put_iftd_on_path() {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().unwrap().parent().unwrap();
    assert!(
        profile.join("iftd").is_file(),
        "run `cargo build -p infiniterm-session` first"
    );
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<PathBuf> = std::env::split_paths(&existing).collect();
    paths.insert(0, profile.to_path_buf());
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
}

fn tick_until(host: &mut Host, mut pred: impl FnMut(&mut Host) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut now = 1_000_000.;
    while Instant::now() < deadline {
        host.tick(now);
        if pred(host) {
            return true;
        }
        now += 100.;
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn io(dir: &std::path::Path, home: &std::path::Path, socket: &str) -> HostIo {
    let file = dir.join("workspace.json");
    let (r, w) = (file.clone(), file);
    HostIo {
        socket: dir.join(socket),
        join_socket: dir.join("j.sock"),
        read_layout: Box::new(move || std::fs::read_to_string(&r).ok()),
        write_layout: Box::new(move |t| std::fs::write(&w, t).map_err(|e| e.to_string())),
        home: home.to_path_buf(),
    }
}

#[test]
fn a_host_with_no_canvas_file_makes_a_card_for_each_running_session() {
    let dir = std::env::temp_dir().join(format!("ifts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("INFINITERM_DATA_DIR", &dir);
    put_iftd_on_path();
    let home = dir.join("home");
    let work = dir.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    // Two shells running, as a server's would be: started by a first host,
    // which then stops and leaves them.
    let mut first = Host::start(io(&dir, &home, "a.sock")).unwrap();
    let a = first
        .model
        .open_terminal_at(&home.to_string_lossy())
        .unwrap();
    let b = first
        .model
        .open_terminal_at(&work.to_string_lossy())
        .unwrap();
    assert!(tick_until(&mut first, |h| [&a, &b].iter().all(|id| h
        .model
        .card(id)
        .is_some_and(|c| c.session.is_some()))));
    let mut sessions: Vec<String> = [&a, &b]
        .iter()
        .map(|id| first.model.card(id).unwrap().session.clone().unwrap())
        .collect();
    sessions.sort();
    first.stop();
    drop(first);
    // The canvas file is gone: this server never had one.
    let _ = std::fs::remove_file(dir.join("workspace.json"));
    std::thread::sleep(Duration::from_millis(300));

    let mut host = Host::start(io(&dir, &home, "b.sock")).unwrap();
    assert_eq!(host.model.cards.len(), 2, "a card for each running session");
    let mut got: Vec<String> = host
        .model
        .cards
        .iter()
        .filter_map(|c| c.session.clone())
        .collect();
    got.sort();
    assert_eq!(got, sessions, "each card names its session");
    // In the folder each shell was started in.
    let mut cwds: Vec<String> = host.model.cards.iter().map(|c| c.cwd.clone()).collect();
    cwds.sort();
    let mut want = vec![
        home.to_string_lossy().to_string(),
        work.to_string_lossy().to_string(),
    ];
    want.sort();
    assert_eq!(cwds, want);
    // The first tick adopts them: no new shell is started.
    assert!(tick_until(&mut host, |h| h
        .model
        .cards
        .iter()
        .all(|c| c.pane_id.is_some())));
    let mut after: Vec<String> = host
        .model
        .cards
        .iter()
        .filter_map(|c| c.session.clone())
        .collect();
    after.sort();
    assert_eq!(after, sessions, "adopted, not replaced");
    // And the canvas is saved for the next start.
    host.flush();
    assert!(dir.join("workspace.json").is_file());

    // Leave nothing running: by pid, only daemons under this test's folder.
    for card in host.model.cards.clone() {
        if let Some(p) = card.pane_id {
            host.backend.pty.kill(p);
        }
    }
    host.stop();
    let needle = format!("--socket {}", dir.display());
    if let Ok(out) = std::process::Command::new("ps")
        .args(["-axww", "-o", "pid=,command="])
        .output()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if line.contains(&needle) {
                if let Some(pid) = line.split_whitespace().next() {
                    let _ = std::process::Command::new("kill").arg(pid).status();
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
