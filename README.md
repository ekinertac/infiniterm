# infini-rust

The native port of [infiniterm](https://github.com/ekinertac/infiniterm): terminal, editor, diff and browser cards on an infinite zoomable canvas, with agent state from Claude Code hooks. Rust, gpui for the window, CEF for the browser card, `alacritty_terminal` for the grid.

Why a port: the Tauri app runs xterm.js in a webview, and a browser card there is either an iframe that cannot log in anywhere or a native view nothing can paint over or scale. The port draws every card itself. Its browser card is Chromium with the Claude in Chrome extension loaded, painted as a texture, so Claude Code can drive a browser that lives inside the canvas.

Status, 2026-09-15: the four-crate workspace exists and every pure module of the Tauri app is ported with its tests (483 of 498 reference cases, plus native checks; `cargo test --offline`). The save file round-trips the real `workspace.json` byte for byte. The backend (PTY, unix socket for `ift` and hooks, process inspection, git, files) runs with no window and answers `ift` in a test. The gpui canvas draws: cards, groups, workspaces, the palette, prompts, the shortcuts panel, every key command of the reference, alignment guides on a drag, and real shells in the cards (htop, fastfetch, links). Terminals select, copy, blink and dim like the reference. Not there yet: the flood target (26 flooding cards paint at 22 fps against the reference's 45 to 60), persistence gates and the config watcher, editor, diff, transcript and browser cards. `HANDOVER.md` has the plan and the state; `make` lists the commands; `tools/drive/` scripts the GUI for testing.

`spikes/` holds the five standalone spikes and their measurements. A CEF frame in a gpui window costs about 1 ms; 25 terminals hold 120 fps under continuous zoom. The extension runs and Google sign-in passes. The spikes stay outside the root workspace.

Single binary plus the CEF framework, no Electron, no account, no telemetry. macOS first; Linux and Windows are kept possible, not promised.

Private, unreleased.
