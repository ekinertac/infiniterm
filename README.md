# infini-rust

The native port of [infiniterm](https://github.com/ekinertac/infiniterm): terminal, editor, diff and browser cards on an infinite zoomable canvas, with agent state from Claude Code hooks. Rust, gpui for the window, CEF for the browser card, `alacritty_terminal` for the grid.

Why a port: the Tauri app runs xterm.js in a webview, and a browser card there is either an iframe that cannot log in anywhere or a native view nothing can paint over or scale. The port draws every card itself. Its browser card is Chromium with the Claude in Chrome extension loaded, painted as a texture, so Claude Code can drive a browser that lives inside the canvas.

Status, 2026-09-15: the stack is decided and measured; the port itself has not started. `spikes/` holds the five spikes with their numbers (a CEF frame in a gpui window costs about 1 ms; 25 terminals hold 120 fps under a continuous zoom; the extension runs and Google sign-in passes). `HANDOVER.md` is the plan.

Single binary plus the CEF framework, no Electron, no account, no telemetry. macOS first; Linux and Windows are kept possible, not promised.

Private, unreleased.
