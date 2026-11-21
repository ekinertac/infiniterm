# App surface: a separate process drawing into a card (2026-10-09)

Question: can a small app in its own process draw into a card well enough that typing, clicking and zooming feel native? Ekin wants an ecosystem of small native apps that run in a card and standalone, not tied to gpui. A process boundary is the only sound one here: Rust has no stable ABI and gpui has no plugin interface.

Status: partial. The child side works and is verified headless. The host side (gpui painting the surface in a card), the standalone window check, and every number on screen are not done. See "Not done".

## What is here

- `src/ui.rs`: the probe UI, a counter and a text field. One function, used in both modes.
- `src/main.rs`: no argument opens a standalone eframe window. `--surface <socket>` runs the surface mode.
- `src/surface.rs`: surface mode. egui drawn offscreen with wgpu (Metal, `Bgra8Unorm`), read back, copied into one IOSurface. The host's input comes in as egui events.
- `src/protocol.rs`: the socket messages, one JSON object per line.
- `src/bin/drive.rs`: a headless host. It plays the card's role with no window: spawns the child, reads the IOSurface by id, writes PNGs, clicks and types, and times each input.

The spike is excluded from the workspace (`[workspace]` in its Cargo.toml). Nothing here lands in the product.

## Stack

- eframe and egui 0.31.1, wgpu 24.0.5 (through eframe's re-export).
- io-surface 0.16.1 and core-foundation 0.10, both already in the app's lockfile.
- Surface id is `IOSurfaceGetID`. The host calls `IOSurfaceLookup`. The surface is created with `kIOSurfaceIsGlobal`, which lookup needs.
- The host side uses gpui 0.2.2 `paint_surface(bounds, CVPixelBuffer)`. The surface wraps as a CVPixelBuffer. This is the same kind of texture paint the browser card does, but not yet run.

## Measured (headless, 2026-10-09, `drive` against the release build)

Frames: 1520 x 840 device pixels for a 760 x 420 card at 2x. The PNGs confirm the pixel path and the input path.

| step | result |
|---|---|
| first frame after the size message | 43 ms |
| click on "+" to the frame that shows it | 2.4 ms |
| key "A" to the frame that shows its letter | 4.4 ms |
| composed text "é😀" to frame | 47 ms (one run; not repeated) |

What the PNG shows after the run: the counter reads 1 (the click worked), the field reads `aé😀` (the key and the composed text both arrived once), and the count says 3 chars.

These times are the child's round trip only. They leave out the socket hop into the host, the host's paint, and the display's refresh. The real key-to-pixel number needs the host and a screen.

## Looks

The text is sharp at 2x. The emoji is monochrome. egui's bundled fonts have no colour emoji. A native text stack would draw it in colour. This matters for the "feels native" bar and is a real limitation of using egui for text.

## Problems found and fixed on the way

- A socket path under the scratchpad is over the 104-byte macOS limit. The driver binds a relative `host.sock`.
- My first driver version measured a frame that came from the pointer move, not the click (10 µs). The driver now settles on a quiet frame before each measurement.
- My first guess at the "+" position was off (40, 74 points). The real place is (50, 40), read from the first frame.

## Not done

1. Standalone window: not run. It opens a window on Ekin's Mac, so it waits for his go.
2. Host paint in gpui: not built. This is the `paint_surface` path. It needs a gpui host, either a spike crate or a scratch build with a surface card kind.
3. Zoom sharpness at 50%, 100% and 200%. Needs the host. Plan: the host sends a new `size` with the new scale when zoom settles, as the browser card does in half steps. Compare that with scaling one bitmap.
4. The emoji panel and dead keys through the host's `ime.rs` path. The child takes `text` messages already; the host side is not written.
5. Tearing. The child writes the one surface while the host can read it. A second surface (ping-pong) or a fence is needed before a claim is made. Not measured.
6. Idle cost. The child wakes once a second when nothing happens. Not measured with `top`.
7. Readback cost by itself. The 43 ms first frame and the per-frame cost are not split out yet.

## Recommendation (provisional)

Nothing rules the process boundary out yet. The child side is simple: a plain egui app, one function, two run modes. The open risk is the host. Whether text stays sharp at zoom, and whether input feels immediate, depends on the gpui paint path and the real window. Those need the next step, and they need a go from Ekin before anything opens or types on his Mac.

## Notes for whoever continues

- `io-surface` is deprecated in favour of `objc2-io-surface`. The spike uses it because it is already in the lockfile. A product version should move.
- `image` is a dev tool here, for the PNG dumps only.
- Run the driver with a short output path, or the socket name is the only thing relative.
