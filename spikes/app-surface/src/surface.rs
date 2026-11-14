// Surface mode: no window. Each frame egui is drawn offscreen with wgpu into a
// BGRA texture, read back, and copied into an IOSurface that the host paints
// as a card body. Input arrives on the socket as protocol::FromHost and goes
// in as egui events, the same events a window would produce.
//
// Known gaps, all measured or listed in NOTES.md: one IOSurface, so the host
// can read a frame while the next one is written (no ping-pong, no fence);
// the readback is a GPU to CPU copy per frame; the surface id is shared
// with IOSurfaceLookup, which needs kIOSurfaceIsGlobal.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::ptr;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use eframe::egui_wgpu;
use egui_wgpu::wgpu;
use io_surface::IOSurface;

use crate::protocol::{self, FromHost, Mods, ToHost};
use crate::ui::Probe;

/// 'BGRA', the IOSurface pixel format that matches the texture we render to.
const PIXEL_FORMAT_BGRA: i64 = 0x4247_5241;
/// Frame timings go to stderr every this many frames when APP_SURFACE_LOG is set.
const LOG_EVERY: u64 = 120;

struct Target {
    w: u32,
    h: u32,
    scale: f32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_bpr: u32,
    surface: IOSurface,
}

pub fn run(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let stream = UnixStream::connect(path)?;
    let reader = stream.try_clone()?;
    let mut out = stream;

    let (tx, rx) = mpsc::channel::<FromHost>();
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            match serde_json::from_str::<FromHost>(&line) {
                Ok(msg) => {
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
                Err(e) => eprintln!("app-surface: bad message ({e}): {line}"),
            }
        }
        // The host closed the socket: the card is gone, so is the probe.
        std::process::exit(0);
    });

    let (device, queue) = pollster::block_on(async {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..Default::default()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .expect("a Metal adapter");
        adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .expect("a Metal device")
    });

    let format = wgpu::TextureFormat::Bgra8Unorm;
    let mut renderer = egui_wgpu::Renderer::new(&device, format, None, 1, true);
    let ctx = egui::Context::default();
    let mut probe = Probe::default();

    // The first message is the card's size. Nothing is drawn before it.
    let mut target = None;
    let mut seq = 0u64;
    let mut wait = Duration::from_secs(1);
    let mut stats = Stats::default();
    let log = std::env::var_os("APP_SURFACE_LOG").is_some();

    loop {
        let first = if target.is_none() {
            rx.recv().ok()
        } else {
            match rx.recv_timeout(wait) {
                Ok(m) => Some(m),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };
        let mut batch: Vec<FromHost> = first.into_iter().collect();
        while let Ok(m) = rx.try_recv() {
            batch.push(m);
        }

        let mut events = Vec::new();
        for msg in batch {
            match msg {
                FromHost::Size { w, h, scale } => {
                    let same = target
                        .as_ref()
                        .is_some_and(|t: &Target| t.w == w && t.h == h && t.scale == scale);
                    if !same && w > 0 && h > 0 {
                        let t = new_target(&device, format, w, h, scale);
                        send(&mut out, &ToHost::Surface {
                            id: t.surface.get_id(),
                            w,
                            h,
                            bpr: unsafe { io_surface::IOSurfaceGetBytesPerRow(t.surface.obj) } as u32,
                        });
                        target = Some(t);
                    }
                }
                FromHost::Pointer { x, y, button, down, mods } => {
                    let pos = egui::pos2(x, y);
                    events.push(egui::Event::PointerMoved(pos));
                    if let Some(b) = button.as_deref().and_then(button_of) {
                        events.push(egui::Event::PointerButton {
                            pos,
                            button: b,
                            pressed: down,
                            modifiers: modifiers(mods),
                        });
                    }
                }
                FromHost::Scroll { dx, dy, mods } => events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(dx, dy),
                    modifiers: modifiers(mods),
                }),
                FromHost::Key { key, down, mods } => {
                    if let Some(k) = egui::Key::from_name(&key) {
                        events.push(egui::Event::Key {
                            key: k,
                            physical_key: None,
                            pressed: down,
                            repeat: false,
                            modifiers: modifiers(mods),
                        });
                    } else {
                        eprintln!("app-surface: unknown key name {key:?}");
                    }
                }
                FromHost::Text { s } => events.push(egui::Event::Text(s)),
            }
        }

        let Some(t) = target.as_mut() else { continue };
        let started = Instant::now();
        let (pixels, delay) = draw(&ctx, &device, &queue, &mut renderer, &mut probe, t, events);
        let drawn = Instant::now();
        copy_into_surface(t, &pixels);
        wait = delay.min(Duration::from_secs(1));

        seq += 1;
        send(&mut out, &ToHost::Frame { id: t.surface.get_id(), seq });

        stats.record(started, drawn, Instant::now());
        if log && seq % LOG_EVERY == 0 {
            eprintln!("app-surface: {}", stats.summary());
            stats = Stats::default();
        }
    }
    Ok(())
}

/// Draw one frame and return the BGRA pixels (tight rows) and when egui next
/// wants a frame. A pointer move or key arrives as an event, so most frames
/// are input-driven; the delay covers the caret blink and hover animations.
fn draw(
    ctx: &egui::Context,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut egui_wgpu::Renderer,
    probe: &mut Probe,
    t: &mut Target,
    events: Vec<egui::Event>,
) -> (Vec<u8>, Duration) {
    let points = egui::vec2(t.w as f32 / t.scale, t.h as f32 / t.scale);
    let mut viewports = egui::ViewportIdMap::default();
    viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo {
            native_pixels_per_point: Some(t.scale),
            inner_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, points)),
            ..Default::default()
        },
    );
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, points)),
        viewport_id: egui::ViewportId::ROOT,
        viewports,
        events,
        ..Default::default()
    };
    let output = ctx.run(raw, |ctx| probe.ui(ctx));
    let delay = output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|v| v.repaint_delay)
        .unwrap_or(Duration::from_secs(1));

    let prims = ctx.tessellate(output.shapes, output.pixels_per_point);
    for (id, delta) in &output.textures_delta.set {
        renderer.update_texture(device, queue, *id, delta);
    }
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [t.w, t.h],
        pixels_per_point: t.scale,
    };

    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let extra = renderer.update_buffers(device, queue, &mut enc, &prims, &screen);
    {
        let mut pass = enc
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("app-surface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.11,
                            g: 0.11,
                            b: 0.12,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            })
            .forget_lifetime();
        renderer.render(&mut pass, &prims, &screen);
    }
    for id in &output.textures_delta.free {
        renderer.free_texture(id);
    }
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &t.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &t.readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(t.padded_bpr),
                rows_per_image: Some(t.h),
            },
        },
        wgpu::Extent3d {
            width: t.w,
            height: t.h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(extra.into_iter().chain(std::iter::once(enc.finish())));

    let slice = t.readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);
    let tight = (t.w * 4) as usize;
    let mut pixels = Vec::with_capacity(tight * t.h as usize);
    {
        let mapped = slice.get_mapped_range();
        for row in 0..t.h as usize {
            let start = row * t.padded_bpr as usize;
            pixels.extend_from_slice(&mapped[start..start + tight]);
        }
    }
    t.readback.unmap();
    (pixels, delay)
}

fn copy_into_surface(t: &Target, pixels: &[u8]) {
    let tight = (t.w * 4) as usize;
    let surf_bpr = unsafe { io_surface::IOSurfaceGetBytesPerRow(t.surface.obj) };
    unsafe {
        io_surface::IOSurfaceLock(t.surface.obj, 0, ptr::null_mut());
        let base = io_surface::IOSurfaceGetBaseAddress(t.surface.obj) as *mut u8;
        for row in 0..t.h as usize {
            ptr::copy_nonoverlapping(
                pixels.as_ptr().add(row * tight),
                base.add(row * surf_bpr),
                tight,
            );
        }
        io_surface::IOSurfaceUnlock(t.surface.obj, 0, ptr::null_mut());
    }
}

fn new_target(device: &wgpu::Device, format: wgpu::TextureFormat, w: u32, h: u32, scale: f32) -> Target {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("app-surface target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    // wgpu wants each row copied to a multiple of 256 bytes.
    let padded_bpr = (w * 4).div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("app-surface readback"),
        size: (padded_bpr * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    Target {
        w,
        h,
        scale,
        texture,
        view,
        readback,
        padded_bpr,
        surface: make_surface(w, h),
    }
}

fn make_surface(w: u32, h: u32) -> IOSurface {
    // kIOSurfaceIsGlobal: IOSurfaceLookup finds a surface by id from another
    // process only when it is global.
    let pairs: Vec<(CFString, CFType)> = unsafe {
        vec![
            (
                CFString::wrap_under_get_rule(io_surface::kIOSurfaceWidth),
                CFNumber::from(w as i64).as_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(io_surface::kIOSurfaceHeight),
                CFNumber::from(h as i64).as_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(io_surface::kIOSurfaceBytesPerElement),
                CFNumber::from(4i64).as_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(io_surface::kIOSurfacePixelFormat),
                CFNumber::from(PIXEL_FORMAT_BGRA).as_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(io_surface::kIOSurfaceIsGlobal),
                CFBoolean::true_value().as_CFType(),
            ),
        ]
    };
    io_surface::new(&CFDictionary::from_CFType_pairs(&pairs))
}

fn send(out: &mut UnixStream, msg: &ToHost) {
    // A closed socket means the host is gone; the reader thread exits the process.
    let _ = out.write_all(protocol::encode(msg).as_bytes());
}

fn button_of(name: &str) -> Option<egui::PointerButton> {
    match name {
        "primary" => Some(egui::PointerButton::Primary),
        "secondary" => Some(egui::PointerButton::Secondary),
        "middle" => Some(egui::PointerButton::Middle),
        _ => None,
    }
}

fn modifiers(m: Mods) -> egui::Modifiers {
    egui::Modifiers {
        alt: m.alt,
        ctrl: m.ctrl,
        shift: m.shift,
        mac_cmd: m.cmd,
        command: m.cmd,
    }
}

/// Frame cost split into draw (CPU record + GPU + readback) and copy into the
/// IOSurface, printed every LOG_EVERY frames.
#[derive(Default)]
struct Stats {
    frames: u64,
    draw_us: u128,
    copy_us: u128,
}

impl Stats {
    fn record(&mut self, started: Instant, drawn: Instant, done: Instant) {
        self.frames += 1;
        self.draw_us += drawn.duration_since(started).as_micros();
        self.copy_us += done.duration_since(drawn).as_micros();
    }

    fn summary(&self) -> String {
        let n = self.frames.max(1) as u128;
        format!(
            "{} frames, draw {} us, copy {} us on average",
            self.frames,
            self.draw_us / n,
            self.copy_us / n
        )
    }
}
