#![cfg(unix)] // headless hosts are unix-only
//! `headless::Host` against a real `iftd` and a real join socket (#438): a
//! canvas held with no window, a viewer's edit applied by the model's own
//! merge, and the cards still there for the next host.
//!
//! The data dir is a short temp folder for the whole process (a unix socket
//! path holds 104 bytes), set before anything reads it, so the real data dir
//! and the installed app's sockets are never touched. One test function: the
//! steps share the sessions the first host leaves behind.

use infiniterm_core::backend::join::{
    encode_canvas_message, encode_update, read_canvas_message, KIND_LAYOUT, KIND_REVISION,
    KIND_UPDATE,
};
use infiniterm_core::headless::{Host, HostIo};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
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

/// Ticks until `pred` or the deadline.
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

/// A viewer's canvas connection: what it writes up, and the messages a thread
/// reads down, so the test can tick the host between looks.
struct Viewer {
    up: UnixStream,
    down: std::sync::mpsc::Receiver<(u8, String)>,
}

impl Viewer {
    fn send(&mut self, bytes: &[u8]) {
        self.up.write_all(bytes).unwrap();
    }
}

fn canvas(path: &std::path::Path) -> Viewer {
    let mut s = UnixStream::connect(path).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    writeln!(s, "ift-join 2 canvas").unwrap();
    // The answer line byte by byte: a buffered reader would take the first
    // canvas messages that follow it into a buffer that is then dropped (the
    // rare CI failure of this test, at the read that never got its message).
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    while s.read(&mut b).unwrap() == 1 && b[0] != b'\n' {
        line.push(b[0]);
    }
    assert_eq!(String::from_utf8_lossy(&line), "ift-join 2 ok");
    let (tx, down) = std::sync::mpsc::channel();
    let mut reader = s.try_clone().unwrap();
    reader.set_read_timeout(None).unwrap();
    std::thread::spawn(move || {
        while let Ok(Some(message)) = read_canvas_message(&mut reader) {
            if tx.send(message).is_err() {
                return;
            }
        }
    });
    Viewer { up: s, down }
}

/// The next layout and its revision, ticking the host while waiting: it only
/// publishes when ticked, and this test is the only thing ticking it.
fn next_layout(host: &mut Host, viewer: &mut Viewer) -> (String, u64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut layout = None;
    let mut now = 2_000_000.;
    while Instant::now() < deadline {
        host.tick(now);
        now += 50.;
        while let Ok(message) = viewer.down.recv_timeout(Duration::from_millis(50)) {
            match message {
                (KIND_LAYOUT, text) => layout = Some(text),
                (KIND_REVISION, text) => {
                    return (layout.take().unwrap(), text.trim().parse().unwrap())
                }
                _ => {}
            }
        }
    }
    panic!("no layout arrived within 10 s");
}

#[test]
fn a_host_with_no_window_holds_the_canvas_serves_a_viewer_and_hands_the_cards_on() {
    let dir = std::env::temp_dir().join(format!("ifth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("INFINITERM_DATA_DIR", &dir);
    put_iftd_on_path();
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();

    // The hook socket's listener lives as long as the process (the app has
    // one host per process), so a second host in this test needs its own.
    let io = |dir: &std::path::Path, home: &std::path::Path, socket: &str| HostIo {
        socket: dir.join(socket),
        join_socket: dir.join("j.sock"),
        read_layout: Box::new({
            let p = dir.join("workspace.json");
            move || std::fs::read_to_string(&p).ok()
        }),
        write_layout: Box::new({
            let p = dir.join("workspace.json");
            move |t| std::fs::write(&p, t).map_err(|e| e.to_string())
        }),
        home: home.to_path_buf(),
    };

    // 1. A host starts on an empty canvas; a card is made on it (as `ift` or
    //    a viewer would); the host starts its shell and saves the canvas.
    let mut host = Host::start(io(&dir, &home, "i.sock")).expect("the host starts");
    let home_str = home.to_string_lossy().to_string();
    let first = host.model.open_terminal_at(&home_str).unwrap();
    assert!(
        tick_until(&mut host, |h| h
            .model
            .card(&first)
            .is_some_and(|c| c.pane_id.is_some() && c.session.is_some())),
        "the card got a shell and learned its session"
    );
    let session = host.model.card(&first).unwrap().session.clone().unwrap();
    assert!(
        tick_until(&mut host, |_| dir.join("workspace.json").is_file()
            && std::fs::read_to_string(dir.join("workspace.json"))
                .unwrap()
                .contains(&session)),
        "the canvas file names the card's session"
    );

    // 2. A viewer joins: it gets the canvas with that card; its edit (a new
    //    card) is applied by the host, which starts a shell for it.
    let mut viewer = canvas(&dir.join("j.sock"));
    let (layout, rev) = loop {
        let got = next_layout(&mut host, &mut viewer);
        if got.0.contains(&first) {
            break got;
        }
    };
    assert!(layout.contains(&session));
    let mut edited: serde_json::Value = serde_json::from_str(&layout).unwrap();
    let mut made = edited["cards"][0].clone();
    made["id"] = "viewer-made-card".into();
    made["session"] = serde_json::Value::Null;
    made["rect"]["x"] = (made["rect"]["x"].as_f64().unwrap() + 2000.).into();
    edited["cards"].as_array_mut().unwrap().push(made);
    viewer.send(&encode_canvas_message(
        KIND_UPDATE,
        &encode_update(rev, &edited.to_string(), &[]),
    ));
    assert!(
        tick_until(&mut host, |h| h
            .model
            .card("viewer-made-card")
            .is_some_and(|c| c.pane_id.is_some())),
        "the host started a shell for the card a viewer made"
    );

    // 3. The host stops; the sessions keep running; a second host on the same
    //    folder adopts them instead of starting new shells.
    let second_session = {
        assert!(tick_until(&mut host, |h| h
            .model
            .card("viewer-made-card")
            .is_some_and(|c| c.session.is_some())));
        host.model
            .card("viewer-made-card")
            .unwrap()
            .session
            .clone()
            .unwrap()
    };
    host.stop();
    drop(host);
    std::thread::sleep(Duration::from_millis(300));
    let mut again = Host::start(io(&dir, &home, "i2.sock")).expect("the next host starts");
    assert!(
        tick_until(&mut again, |h| h.model.cards.len() == 2
            && h.model.cards.iter().all(|c| c.pane_id.is_some())),
        "both cards got a pane"
    );
    let sessions: Vec<_> = again
        .model
        .cards
        .iter()
        .filter_map(|c| c.session.clone())
        .collect();
    assert!(
        sessions.contains(&session) && sessions.contains(&second_session),
        "the same two sessions, adopted not replaced: {sessions:?}"
    );

    // 4. A viewer's close reaches the host through the model's own close.
    let mut viewer = canvas(&dir.join("j.sock"));
    let (layout, rev) = next_layout(&mut again, &mut viewer);
    let mut edited: serde_json::Value = serde_json::from_str(&layout).unwrap();
    edited["cards"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["id"] != "viewer-made-card");
    // The close is named (#588): a layout that merely lacks the card closes
    // nothing.
    viewer.send(&encode_canvas_message(
        KIND_UPDATE,
        &encode_update(rev, &edited.to_string(), &["viewer-made-card".to_string()]),
    ));
    assert!(
        tick_until(&mut again, |h| h.model.card("viewer-made-card").is_none()),
        "the host closed the card the viewer closed"
    );

    // Leave nothing running: end every session this test started.
    for card in again.model.cards.clone() {
        if let Some(p) = card.pane_id {
            again.backend.pty.kill(p);
        }
    }
    again.stop();
    // By pid, from the process table, and only daemons whose socket is under
    // this test's own folder.
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
