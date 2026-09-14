# infini-rust

The native port of [infiniterm](https://github.com/ekinertac/infiniterm): terminal, editor, diff and browser cards on an infinite zoomable canvas, with agent state from Claude Code hooks. Rust, gpui for the window, CEF for the browser card, `alacritty_terminal` for the grid.

Why a port: the Tauri app runs xterm.js in a webview, and a browser card there is either an iframe that cannot log in anywhere or a native view nothing can paint over or scale. The port draws every card itself. Its browser card is Chromium with the Claude in Chrome extension loaded, painted as a texture, so Claude Code can drive a browser that lives inside the canvas.

Status, 2026-09-15: Phase 0 is complete. The root workspace has four crates, with the viewport module in `infiniterm-core` and placeholders for the terminal, browser, and UI. All 12 viewport tests pass with `cargo test --offline`. The workspace has no external dependencies yet. Phase 1 starts with the pure geometry modules; `HANDOVER.md` is the plan.

`spikes/` holds the five standalone spikes and their measurements. A CEF frame in a gpui window costs about 1 ms; 25 terminals hold 120 fps under continuous zoom. The extension runs and Google sign-in passes. The spikes stay outside the root workspace.

Single binary plus the CEF framework, no Electron, no account, no telemetry. macOS first; Linux and Windows are kept possible, not promised.

Private, unreleased.
