//! The join file service end to end (#560): a `HostSource` over a stand-in ssh
//! that runs the real `ift proxy files` as the host's helper, with files in a
//! scratch folder. The ssh and the helper's process are real; only the network
//! is missing.

mod common;
use common::{profile_dir, stand_in_ssh, Scratch};

use infiniterm_core::backend::file_client::HostSource;
use infiniterm_core::backend::remote::RemoteHost;
use infiniterm_core::file_source::{Code, EntryKind, FileSource};

fn host(ssh: &std::path::Path) -> RemoteHost {
    let mut h = RemoteHost::new("test@server");
    h.ssh = ssh.display().to_string();
    h.ift = env!("CARGO_BIN_EXE_ift").to_string();
    h
}

#[test]
fn a_host_source_reads_the_hosts_files_through_the_real_helper() {
    let work = Scratch::new("files-ok");
    let server = Scratch::new("files-srv");
    let ssh = stand_in_ssh(&work.0, &server.0);
    let docs = server.0.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("README.md"), "# Hello from the host\n").unwrap();
    let big: Vec<u8> = (0..700_000).map(|i| (i % 253) as u8).collect();
    std::fs::write(docs.join("big.bin"), &big).unwrap();

    let src = HostSource::new(host(&ssh));
    let path = |n: &str| docs.join(n).to_string_lossy().into_owned();
    let stat = src.stat(&path("README.md")).unwrap();
    assert_eq!((stat.kind, stat.size), (EntryKind::File, 22));
    assert_eq!(src.read(&path("README.md")).unwrap().text(), "# Hello from the host\n");
    assert!(src.read(&path("big.bin")).unwrap().bytes == big, "a read of several chunks is whole");
    let names: Vec<_> = src
        .list(&docs.to_string_lossy())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["big.bin", "README.md"]);
    assert_eq!(src.read(&path("nope.md")).unwrap_err().code, Code::NotFound);
    assert!(!src.home().unwrap().home.is_empty());
}

#[test]
fn a_host_whose_ift_has_no_file_helper_is_told_to_update_it() {
    let work = Scratch::new("files-old");
    // `proxy files` on an older ift is not an answer at all: a shell error.
    let ssh = work.0.join("ssh");
    std::fs::write(
        &ssh,
        format!(
            "#!/bin/sh\nfor last; do :; done\ncase \"$last\" in\n*\"proxy files\"*) echo 'ift: unknown command proxy files' >&2; exit 2;;\nesac\nPATH='{}':\"$PATH\" exec sh -c \"$last\"\n",
            profile_dir().display()
        ),
    )
    .unwrap();
    std::process::Command::new("chmod").arg("+x").arg(&ssh).status().unwrap();
    let src = HostSource::new(host(&ssh));
    let err = src.read("/etc/hosts").unwrap_err();
    assert_eq!(err.code, Code::Lost);
    assert!(err.message.contains("unknown command proxy files"), "{err}");
}
