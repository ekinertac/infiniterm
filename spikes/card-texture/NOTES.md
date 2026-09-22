# A terminal card as a texture (2026-09-22)

Question: at far zoom, can a card's text be rendered to a bitmap once per output change and painted like the browser card's CEF texture, instead of painting every glyph every frame? Ekin asked after the glyph budget landed: bars are fast but they are not text.

Headless spike, no window: CoreText into a BGRA bitmap the size a card really is at his fit-all, with a card's worth of Claude-shaped output (box drawing, prose, tool lines, indented code, five colours). Release build, Mac mini M4.

## Result

A full card is 151 x 87 cells. At his fit-all (19 px font x 0.507 = 9.63 px) that is a 728 x 1006 texture, 2.9 MB.

| | cost | when |
|---|---|---|
| glyphs through gpui (today) | ~7.5 ms per card | EVERY frame (12 cards = 85 to 100 ms, measured 2026-09-19) |
| CoreText into a bitmap | 4.0 ms per card (13.3 ms the first time, the font's glyph cache) | once per output change |
| bars (today's far mode) | 0.5 ms per card | every frame |
| painting a ready texture | a blit | every frame |

At other sizes: 6 px font 453 x 627, 1.1 MB, 4.4 ms; 14 px 1058 x 1462, 6.2 MB, 4.9 ms. The time barely moves with the size, so it is glyph count, not pixels.

Steady state at fit-all with one card streaming, rate-limited the way `FAR_REFRESH_MS` already limits far repaints (4 Hz): 4 ms every 250 ms, plus twelve blits a frame. Worst case, twelve cards all streaming: 48 ms per 250 ms, 19% of one core. Today it is 90 ms EVERY frame.

Zooming is where it really pays: a pinch re-renders nothing, it scales the textures it has.

## Looks

`/tmp/card-texture-fitall.png` (9.6 px) and `/tmp/card-texture-tiny.png` (6 px) against `/tmp/card-texture-bars.png`. At 9.6 px the text reads, in colour, and at 6 px it is still shapes with the right ink where the code and the prose are. Bars are a grey pattern. The answer to "could it look better than blocks" is yes, plainly.

## What it settled

1. No new crate in the tree: `core-text 21`, `core-graphics 0.24`, `core-foundation 0.10` and `image 0.25` are already in the app's `Cargo.lock` as gpui's own dependencies.
2. gpui paints any BGRA buffer: `window.paint_image(bounds, corners, Arc<RenderImage>, 0, false)`, which is exactly what `browser_body.rs` does with CEF's frames at every zoom. The mechanism is proven in this codebase.
3. Rendering a row as one `CTLine` with a colour run per span is enough; the texture is stretched to the card's rect, so its internal grid only has to agree with itself. The per-cell placement rule the glyph painter needs (2026-09-19's drift trap) does not apply here.
4. 4 ms is small enough that the texture can be re-rendered on the far refresh tick rather than on a timer of its own.

## What it did NOT settle, and the traps

- **The atlas leaks unless you drop.** `window.paint_image` uploads a tile keyed by the image's id and only `window.drop_image` frees it; `RenderImage` has no `Drop`. Nothing in our tree calls `drop_image` today, and `browser_body.rs:344` makes a new `RenderImage` per CEF frame, so a live browser card is probably growing the atlas for as long as it runs. Worth a look by the browser session; a texture terminal must drop the old image every update.
- **Atlas size.** gpui's atlas textures start at 1024 x 1024 and grow to fit, up to 16384. A 728 x 1006 tile all but fills a 1024 square, so twelve cards are twelve textures, about 50 MB of GPU memory on top of 35 MB of CPU buffers. Not measured against a real GPU.
- Colour fidelity against our own palette (the spike used five hand-picked colours), wide characters and emoji, and the cursor and selection overlays, which stay as quads over the texture because they change without output.
- Whether the editor, diff and transcript bodies want the same treatment. They report no cells to the glyph budget yet either.

## Decision: not built (2026-09-22)

Ekin read the numbers and kept bars: "bars: 0.5 ms/card beats all of them". A texture is 8x a bar to produce and carries a cache, an invalidation rule, an atlas tile and a second text path; bars are one rect per word and never go stale. Far zoom is for finding a card, not for reading it.

What would reopen this: wanting to READ a card at fit-all. Then the recommendation was: below `FAR_FONT_PX` a terminal paints its texture, bars as the fallback for the frame before the first one exists, the glyph budget still deciding when far starts. Two to three days.

## How to run

```
cd spikes/card-texture && cargo run --release        # or: cargo run --release -- Menlo
```
