# 25 terminals under a zoom in gpui (2026-09-14)

Question: does gpui hold the frame rate with 25 `alacritty_terminal` grids (80x40) while the zoom animates continuously between 0.35 and 1.15, idle and flooding? The Tauri app's numbers, after its scheduler work: 60 fps idle, 45 to 60 flooding. Release build, Mac mini M4, 4K display at 1x, gpui 0.2.2, alacritty_terminal 0.26.

## Result

| | fps | paint per frame |
|---|---|---|
| idle (`ls` output), zoom animating | 85 to 120 | 5 to 11 ms, the 11 at zoom 0.35 where all 25 cards are on screen |
| 25 x `yes`, zoom animating | 118 to 120 | 3.5 to 4.6 ms shape and paint, plus 10 to 16 ms parsing 200 KiB |

Reshaping every visible line at a new pixel size each frame is not a problem: 1000 lines of 80 columns shape and paint in 5 to 11 ms. The glyph atlas did not become a concern in a two-minute run.

## The number that mattered

The first version used alacritty's own `EventLoop`: one thread per pane, parsing PTY output greedily under a `FairMutex`, the renderer locking each term once per frame. Under flood that gave **1 fps**: 40 to 870 ms per frame waiting on locks, 2 ms shaping. Twenty-five reader threads on ten cores, each holding its lock for a 64 KiB parse of two-byte lines, is the pathological case and `yes` produces it on purpose.

The second version is the Tauri app's shape (`local_pty.rs` credit plus `outputScheduler.ts`), which CLAUDE.md records as the fix that took the webview from 0.2 to 45-60 fps: reader threads only move bytes into a bounded channel of eight 64 KiB chunks (when it fills the reader blocks on send and the child blocks on the kernel PTY buffer), and the main thread parses a fixed budget per frame, 256 KiB round-robin over the panes, before painting. No lock exists. That is the 120 fps row. Parse throughput at 60 fps is about 15 to 20 MB/s across all panes, above the 6 to 11 MB/s the webview managed.

Decision for the port: `infiniterm-term` wraps `Term` and `vte::ansi::Processor` only; the PTY reader stays ours (`local_pty.rs` as it is) and parsing happens on the UI thread under a byte budget. `alacritty_terminal::event_loop` is not used.

## Traps

- Text shaped per row with one `TextRun` per colour change; consecutive cells with equal colours share a run. Shaping cost tracks the number of runs more than the number of characters.
- A binary launched from a shell is not activated and gets no key events; wrap it in a minimal `.app` and `open` it. `target/term-zoom.app` is that, built by hand (Info.plist with an icon key, per the global rule).
- The Mac mini's display is 120 Hz here, so "120 fps" means display-bound; the paint time is the number to compare.

## How to run

```
cargo build --release
cp target/release/term-zoom target/term-zoom.app/Contents/MacOS/
open --stderr "$PWD/run.log" --stdout "$PWD/run.log" target/term-zoom.app
```

Then click into the window: `1` idle, `2` flood, `space` freezes the zoom. `run.log` gets an fps line per second and a breakdown line for any frame over 12 ms.

The code of this spike was deleted on 2026-09-16 once its replacement was in the crates (`infiniterm-browser`, `infiniterm-term`, `infiniterm-ui`); `git log -- spikes/term-zoom` has it.
