// drive: a headless host for the probe, the same role the card plays, with
// no window. It spawns `app-surface --surface <socket>`, sends a size, reads
// the IOSurface the child announces (by its global id, the way the card will)
// and writes PNGs. Then it clicks "+" and types, and times how long each input
// takes to show up as a new frame.
//
// Usage: drive <app-surface binary> <out dir>
// It checks the pixel path and the input path. It does not look at the screen:
// read the PNGs to see whether a click or a key did what it should.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use io_surface::IOSurface;
use serde_json::{json, Value};

const W: u32 = 760;
const H: u32 = 420;
const SCALE: f32 = 2.0;
/// kIOSurfaceLockReadOnly; io-surface 0.16 does not export it.
const LOCK_READ_ONLY: u32 = 0x0000_0001;
/// A frame that arrives this long after the last one counts as settled.
const QUIET: Duration = Duration::from_millis(250);

/// Where the probe's widgets sit, in points, read from frame-0.png at 2x:
/// "+" at device (100, 80), the text field at device (200, 123).
const PLUS: (f32, f32) = (50.0, 40.0);
const FIELD: (f32, f32) = (100.0, 62.0);

#[derive(Clone, Copy)]
struct Seen {
    id: u32,
    seq: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let bin = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out)?;
    // A relative name: macOS caps a socket path at 104 bytes, and the output
    // directory can be longer than that.
    let sock = PathBuf::from("host.sock");
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;

    let mut child: Child = Command::new(&bin).arg("--surface").arg(&sock).spawn()?;
    let (stream, _) = listener.accept()?;
    let mut writer = stream.try_clone()?;
    let frames = read_messages(stream);

    send(&mut writer, json!({"t": "size", "w": W * 2, "h": H * 2, "scale": SCALE}))?;
    let t0 = Instant::now();
    let mut seen = wait_frame(&frames, 0)?;
    println!("first frame after {:?} (seq {})", t0.elapsed(), seen.seq);
    dump(seen.id, &out.join("frame-0.png"))?;

    // Settle: a pointer move with no button produces its own frame.
    send(&mut writer, json!({"t": "pointer", "x": PLUS.0, "y": PLUS.1}))?;
    seen = settle(&frames, seen)?;

    // Click "+". Time from the press leaving the driver to the frame that shows it.
    let click = Instant::now();
    send(&mut writer, json!({"t": "pointer", "x": PLUS.0, "y": PLUS.1, "button": "primary", "down": true}))?;
    send(&mut writer, json!({"t": "pointer", "x": PLUS.0, "y": PLUS.1, "button": "primary", "down": false}))?;
    seen = wait_frame(&frames, seen.seq)?;
    println!("click to frame: {:?} (seq {})", click.elapsed(), seen.seq);
    seen = settle(&frames, seen)?;
    dump(seen.id, &out.join("frame-click.png"))?;

    // Focus the text field, then type: a key and its letter, then composed text
    // (a dead key's é and an emoji, the way the host's ime.rs path would send them).
    send(&mut writer, json!({"t": "pointer", "x": FIELD.0, "y": FIELD.1, "button": "primary", "down": true}))?;
    send(&mut writer, json!({"t": "pointer", "x": FIELD.0, "y": FIELD.1, "button": "primary", "down": false}))?;
    seen = settle(&frames, seen)?;
    let typing = Instant::now();
    send(&mut writer, json!({"t": "key", "key": "A", "down": true}))?;
    send(&mut writer, json!({"t": "text", "s": "a"}))?;
    seen = wait_frame(&frames, seen.seq)?;
    println!("key a to frame: {:?} (seq {})", typing.elapsed(), seen.seq);
    seen = settle(&frames, seen)?;
    let composed = Instant::now();
    send(&mut writer, json!({"t": "text", "s": "é😀"}))?;
    seen = wait_frame(&frames, seen.seq)?;
    println!("composed text to frame: {:?} (seq {})", composed.elapsed(), seen.seq);
    seen = settle(&frames, seen)?;
    dump(seen.id, &out.join("frame-typed.png"))?;

    drop(writer);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&sock);
    Ok(())
}

/// Parse the child's lines on a thread, so the driver can wait with a timeout.
fn read_messages(stream: UnixStream) -> Receiver<Value> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if tx.send(v).is_err() {
                break;
            }
        }
    });
    rx
}

fn send(w: &mut impl Write, v: Value) -> std::io::Result<()> {
    let mut s = v.to_string();
    s.push('\n');
    w.write_all(s.as_bytes())
}

fn as_seen(v: &Value) -> Seen {
    Seen {
        id: v["id"].as_u64().unwrap_or(0) as u32,
        seq: v["seq"].as_u64().unwrap_or(0),
    }
}

/// The next frame with a seq above `after`. Errors if none comes in 10 seconds.
fn wait_frame(frames: &Receiver<Value>, after: u64) -> Result<Seen, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let v = frames.recv_timeout(left).map_err(|_| "no frame within 10 s")?;
        if v["t"] == "frame" && v["seq"].as_u64().unwrap_or(0) > after {
            return Ok(as_seen(&v));
        }
    }
}

/// Keep taking frames until none arrives for QUIET. Returns the last one, so
/// the next measurement starts from a frame that nothing else is still drawing.
fn settle(frames: &Receiver<Value>, mut seen: Seen) -> Result<Seen, Box<dyn std::error::Error>> {
    loop {
        match frames.recv_timeout(QUIET) {
            Ok(v) if v["t"] == "frame" => seen = as_seen(&v),
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => return Ok(seen),
            Err(_) => return Err("the probe closed the socket".into()),
        }
    }
}

/// Lock the IOSurface read-only and write it as a PNG. The rows are BGRA; the
/// PNG wants RGBA.
fn dump(id: u32, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let surface: IOSurface = io_surface::lookup(id);
    let (w, h, bpr) = unsafe {
        (
            io_surface::IOSurfaceGetWidth(surface.obj) as u32,
            io_surface::IOSurfaceGetHeight(surface.obj) as u32,
            io_surface::IOSurfaceGetBytesPerRow(surface.obj),
        )
    };
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    unsafe {
        io_surface::IOSurfaceLock(surface.obj, LOCK_READ_ONLY, std::ptr::null_mut());
        let base = io_surface::IOSurfaceGetBaseAddress(surface.obj) as *const u8;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let s = base.add(y * bpr + x * 4);
                let d = (y * w as usize + x) * 4;
                rgba[d] = *s.add(2);
                rgba[d + 1] = *s.add(1);
                rgba[d + 2] = *s;
                rgba[d + 3] = *s.add(3);
            }
        }
        io_surface::IOSurfaceUnlock(surface.obj, LOCK_READ_ONLY, std::ptr::null_mut());
    }
    image::save_buffer(path, &rgba, w, h, image::ColorType::Rgba8)?;
    println!("wrote {} ({w}x{h}, bytes per row {bpr})", path.display());
    Ok(())
}
