//! The process table and working directories on macOS, read in process with
//! libproc and sysctl instead of spawning `ps` and `lsof` every poll.
//!
//! `inspect::pane_status_poll` used to fork `ps -axo ...` and `lsof -d cwd`
//! every 750 ms. Measured 2026-11-16 on the mini (1,202 processes, 31
//! shells): about 0.04 s of CPU for ps and 0.03 s for lsof per poll, close
//! to 9% of a core per running app, plus the system's own work at every exec.
//!
//! What it reads, and for whom:
//! - every process: pid, ppid, pgid and whether it is in its terminal's
//!   foreground group (`PROC_PIDTBSDINFO`, one call each). That is what
//!   `pane_statuses` walks.
//! - only the processes under a card's shell: the command line
//!   (`KERN_PROCARGS2`) and ps's %cpu (the sum of the threads' decaying
//!   `pth_cpu_usage`, which is what ps itself adds up). Nothing else in the
//!   table is ever read for those, so a thousand other processes cost one
//!   call each, not three.
//! - only the pids a status reports: the working directory
//!   (`PROC_PIDVNODEPATHINFO`), what lsof printed.
//!
//! The caller in `inspect.rs` falls back to ps and lsof when this returns
//! nothing, so a libproc failure costs speed, not the labels. Windows has its
//! own snapshot there; Linux keeps ps.

use super::Proc;
use std::collections::{HashMap, HashSet};
use std::ffi::{c_int, c_void};

/// `<sys/proc_info.h>`, not in the libc crate: the thread ids of a process.
const PROC_PIDLISTTHREADS: c_int = 6;
/// `<sys/proc_info.h>` `PROC_FLAG_CONTROLT`: the process has a controlling
/// terminal. ps prints `+` only for such a process in the foreground group.
const PROC_FLAG_CONTROLT: u32 = 0x80;
/// `<mach/thread_info.h>`: `pth_cpu_usage` is in thousandths of a core.
const TH_USAGE_SCALE: f32 = 1000.;
/// Room for processes started between the size query and the list.
const PID_SLACK: usize = 64;
/// More threads than any process under a card has; a bigger one is cut,
/// which only lowers its %cpu.
const MAX_THREADS: usize = 1024;

/// The table, with command lines and %cpu filled in for everything under
/// `shells`. `None` when libproc gives nothing, so the caller falls back.
pub fn snapshot(shells: &[u32]) -> Option<Vec<Proc>> {
    let mut procs: Vec<Proc> = all_pids()?.into_iter().filter_map(bsd_info).collect();
    if procs.is_empty() {
        return None;
    }
    let under = descendants(&procs, shells);
    for p in procs.iter_mut().filter(|p| under.contains(&p.pid)) {
        p.args = args(p.pid).unwrap_or_default();
        p.cpu = cpu(p.pid);
    }
    Some(procs)
}

/// The working directory of each pid that has one readable.
pub fn cwds(pids: &[u32]) -> HashMap<u32, String> {
    pids.iter()
        .filter_map(|&pid| cwd(pid).map(|d| (pid, d)))
        .collect()
}

/// Every pid below the shells (the shells themselves excluded), by parent
/// links. Bounded like `pane_statuses`, so a malformed table cannot spin.
pub fn descendants(procs: &[Proc], shells: &[u32]) -> HashSet<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in procs {
        children.entry(p.ppid).or_default().push(p.pid);
    }
    let mut out = HashSet::new();
    let mut stack: Vec<u32> = shells.to_vec();
    while let Some(pid) = stack.pop() {
        if out.len() > 10_000 {
            break;
        }
        for &kid in children.get(&pid).map(Vec::as_slice).unwrap_or_default() {
            if kid != pid && out.insert(kid) {
                stack.push(kid);
            }
        }
    }
    out
}

/// ps's `+`: a controlling terminal, and the terminal's foreground group is
/// this process's group.
pub fn is_foreground(flags: u32, pgid: u32, tpgid: u32) -> bool {
    flags & PROC_FLAG_CONTROLT != 0 && pgid == tpgid
}

/// The argv words of a `KERN_PROCARGS2` buffer, joined by single spaces as
/// `parse_ps` joins them: an `int` argc, the executable path, NUL padding,
/// then argc NUL-terminated words.
pub fn parse_procargs2(buf: &[u8]) -> Option<String> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    let mut rest = &buf[4..];
    // The executable path, then the NULs that pad it.
    let path_end = rest.iter().position(|&b| b == 0)?;
    rest = &rest[path_end..];
    let start = rest.iter().position(|&b| b != 0)?;
    rest = &rest[start..];
    let words: Vec<String> = rest
        .split(|&b| b == 0)
        .take(argc.max(0) as usize)
        .map(|w| String::from_utf8_lossy(w).into_owned())
        .collect();
    (!words.is_empty()).then(|| {
        words
            .iter()
            .flat_map(|w| w.split_whitespace())
            .collect::<Vec<_>>()
            .join(" ")
    })
}

fn all_pids() -> Option<Vec<u32>> {
    let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if n <= 0 {
        return None;
    }
    let mut pids = vec![0 as c_int; n as usize + PID_SLACK];
    let bytes = (pids.len() * std::mem::size_of::<c_int>()) as c_int;
    let got = unsafe { libc::proc_listallpids(pids.as_mut_ptr() as *mut c_void, bytes) };
    if got <= 0 {
        return None;
    }
    pids.truncate(got as usize);
    Some(
        pids.into_iter()
            .filter(|&p| p > 0)
            .map(|p| p as u32)
            .collect(),
    )
}

fn bsd_info(pid: u32) -> Option<Proc> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as c_int;
    let got = unsafe {
        libc::proc_pidinfo(
            pid as c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut c_void,
            size,
        )
    };
    (got == size).then(|| Proc {
        pid: info.pbi_pid,
        ppid: info.pbi_ppid,
        pgid: info.pbi_pgid,
        foreground: is_foreground(info.pbi_flags, info.pbi_pgid, info.e_tpgid),
        cpu: 0.,
        args: String::new(),
    })
}

fn args(pid: u32) -> Option<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as c_int];
    let mut size: libc::size_t = 0;
    let ok = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if ok != 0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    let ok = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if ok != 0 {
        return None;
    }
    buf.truncate(size);
    parse_procargs2(&buf)
}

/// ps's %cpu: the decaying usage of every thread, summed.
fn cpu(pid: u32) -> f32 {
    let mut ids = vec![0u64; MAX_THREADS];
    let bytes = (ids.len() * std::mem::size_of::<u64>()) as c_int;
    let got = unsafe {
        libc::proc_pidinfo(
            pid as c_int,
            PROC_PIDLISTTHREADS,
            0,
            ids.as_mut_ptr() as *mut c_void,
            bytes,
        )
    };
    if got <= 0 {
        return 0.;
    }
    ids.truncate(got as usize / std::mem::size_of::<u64>());
    let size = std::mem::size_of::<libc::proc_threadinfo>() as c_int;
    let mut usage = 0i64;
    for id in ids {
        let mut t: libc::proc_threadinfo = unsafe { std::mem::zeroed() };
        let got = unsafe {
            libc::proc_pidinfo(
                pid as c_int,
                libc::PROC_PIDTHREADINFO,
                id,
                &mut t as *mut _ as *mut c_void,
                size,
            )
        };
        if got == size {
            usage += t.pth_cpu_usage as i64;
        }
    }
    usage as f32 * 100. / TH_USAGE_SCALE
}

fn cwd(pid: u32) -> Option<String> {
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as c_int;
    let got = unsafe {
        libc::proc_pidinfo(
            pid as c_int,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            &mut info as *mut _ as *mut c_void,
            size,
        )
    };
    if got != size {
        return None;
    }
    // libc declares the MAXPATHLEN buffer as 32 rows of 32.
    let path = &info.pvi_cdir.vip_path;
    let bytes: Vec<u8> = path
        .iter()
        .flatten()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    let s = String::from_utf8_lossy(&bytes).into_owned();
    s.starts_with('/').then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn procargs(path: &str, argv: &[&str]) -> Vec<u8> {
        let mut b = (argv.len() as i32).to_ne_bytes().to_vec();
        b.extend(path.as_bytes());
        b.extend([0, 0, 0]);
        for w in argv {
            b.extend(w.as_bytes());
            b.push(0);
        }
        // The environment follows argv in the real buffer.
        b.extend(b"HOME=/Users/x\0");
        b
    }

    #[test]
    fn procargs2_gives_argv_without_path_or_environment() {
        let b = procargs("/usr/bin/ssh", &["ssh", "-p", "22", "host"]);
        assert_eq!(parse_procargs2(&b).as_deref(), Some("ssh -p 22 host"));
    }

    #[test]
    fn procargs2_keeps_a_login_shells_dash() {
        let b = procargs("/bin/zsh", &["-zsh"]);
        assert_eq!(parse_procargs2(&b).as_deref(), Some("-zsh"));
    }

    #[test]
    fn procargs2_joins_like_parse_ps() {
        // ps prints words separated by spaces, and parse_ps collapses runs.
        let b = procargs("/bin/sh", &["sh", "-c", "a  b"]);
        assert_eq!(parse_procargs2(&b).as_deref(), Some("sh -c a b"));
    }

    #[test]
    fn a_short_buffer_is_none() {
        assert_eq!(parse_procargs2(&[1, 0]), None);
    }

    #[test]
    fn foreground_needs_a_terminal_and_the_terminals_group() {
        assert!(is_foreground(PROC_FLAG_CONTROLT, 50, 50));
        assert!(!is_foreground(PROC_FLAG_CONTROLT, 50, 51));
        assert!(!is_foreground(0, 50, 50));
    }

    fn p(pid: u32, ppid: u32) -> Proc {
        Proc {
            pid,
            ppid,
            pgid: pid,
            foreground: false,
            cpu: 0.,
            args: String::new(),
        }
    }

    #[test]
    fn descendants_are_everything_below_the_shells_and_nothing_else() {
        let t = [p(1, 0), p(10, 1), p(11, 10), p(12, 11), p(20, 1), p(21, 20)];
        let d = descendants(&t, &[10]);
        assert_eq!(d, [11, 12].into_iter().collect());
    }

    #[test]
    fn this_process_is_in_the_table_with_its_parent() {
        let me = std::process::id();
        let procs = snapshot(&[]).expect("libproc answers on macOS");
        let found = procs
            .iter()
            .find(|p| p.pid == me)
            .expect("this test process");
        assert_eq!(found.ppid, unsafe { libc::getppid() } as u32);
    }

    #[test]
    fn a_child_under_a_shell_gets_its_command_line() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let me = std::process::id();
        let procs = snapshot(&[me]).unwrap();
        let kid = procs
            .iter()
            .find(|p| p.pid == child.id())
            .expect("the child");
        assert_eq!(kid.args, "/bin/sleep 30");
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn the_working_directory_is_this_processs() {
        let me = std::process::id();
        let dirs = cwds(&[me]);
        let want = std::env::current_dir().unwrap();
        assert_eq!(dirs.get(&me).map(std::path::PathBuf::from), Some(want));
    }
}
