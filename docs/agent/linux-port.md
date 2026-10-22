# Linux desktop app (#464)

What this covers: whether the gpui app runs on Linux, how to test it on a server with no screen, and every macOS-only piece of the ui crate with a plan for Linux.

Read it before touching a `cfg(target_os = "macos")` / `cfg(not(target_os = "macos"))` gate in `infiniterm-ui`, `not_macos.rs`, `keycode.rs`, or anything that runs `open`, `osascript` or reads `/Applications`.

Related: `docs/windows-handoff.md` on the `windows-merge-master` branch (the same seam, built for Windows first), `join-serve-and-login.md` (`ift serve`, which already runs on Linux), `daemon-and-backends.md`.

## Spike result, 2026-10-21

The app draws on a Linux server with no screen. Droplet: Ubuntu 24.04, 8 vCPU, 16 GB, Xvfb at 1920x1080x24, Mesa lavapipe (`llvmpipe (LLVM 20.1.2)`, Vulkan 1.4) as gpui's renderer. Checked with screenshots and `ift read`:

- The window opens (1600x1000, no decorations because Xvfb has no window manager), the welcome Page card draws, the title bar and status bar draw.
- A terminal card made with `ift run card.new.terminal` runs bash under `iftd` (the daemon backend), colours and the cursor draw, `ift send` and `ift read` work.
- `xdotool type` reaches the shell. Super is Cmd: Super+T made a card, Super+2 fit every card.
- A relaunch adopts the running `iftd` sessions with their screens.

What it took: master does not build on Linux (`core-graphics-types` fails first, because the ui crate depends on the Cocoa crates unconditionally). The `windows-merge-master` branch (5abe95f) builds with `cargo build -p infiniterm-ui --no-default-features` after one change: `keycode.rs` has `install`, `dom_code` and `last_dead` for macOS and Windows only, so Linux needs its own three (the spike used no-op stand-ins). Nothing else in that branch's seam failed on `unix && !macos`.

What is wrong at runtime, found by the spike:

1. The first frame is never shown. The window stays black until the first input event (a mouse move is enough), on every launch. 2026-10-22: only with NO window manager. With openbox running on the same Xvfb the first frame draws at once, so it is not an app bug and was not fixed; a test run starts a window manager (openbox) before the app. The window manager also draws its own title bar above the app's (gpui's X11 window asks for server-side decorations); that is a later polish item.
2. The default terminal font `ui-monospace, Menlo, monospace` resolves to a PROPORTIONAL font on Linux: the cell is measured from its `M` and the text is spread wide. `fc-match monospace` on the same box names DejaVu Sans Mono, so the font gpui picked is not the one the fontconfig alias gives; why was not looked into. Linux needs its own default (`DejaVu Sans Mono` worked) or a lookup through `fc-match monospace`. Fixed 2026-10-22: gpui takes ONE family name and draws a name that is not installed in the platform's default face, and the old `family_of` took the first named family (Menlo) without asking. `fonts::terminal_family_of` now picks the first INSTALLED named family, then for `monospace` the first installed entry of a per-platform list (`PLATFORM_MONOSPACE`), from gpui's `all_font_names`, read once when the window opens (`fonts::remember_installed`). Checked on the droplet: the default list draws DejaVu Sans Mono. `family_of` kept its one-argument signature because the Windows branch's `browser_stub.rs` calls it with no window at hand.
3. Nerd Font icons draw as boxes: `icon_font` is a stub off macOS, so the bundled `Symbols Nerd Font Mono` is not registered.
4. Themes come from `<data>/themes/`, which `tools/bundle.sh` fills on a Mac. A Linux build needs its own way to ship them (the spike copied one by hand).
5. Idle CPU is about 36% of one core with a focused terminal card, all of it in llvmpipe threads (the cursor blink redraws the window in software). That is the price of lavapipe, not a bug; a real GPU will not show it. Do not measure performance on the droplet.
6. On a first launch the welcome card was clipped by the window's right edge after the window was resized with xdotool. 2026-10-22: a first launch on a window that is not resized frames the card as on the Mac; nothing in master refits the view when the window changes size, on any platform, so this is the app's behaviour everywhere and was left alone.

Not tested: Wayland, a real GPU, IME (`ime.rs` compiles because it is gpui's generic `EntityInputHandler`), clipboard, colour emoji (no colour emoji font was installed; ⚠️ drew in monochrome).

## How to repeat it

The driver does all of this (2026-10-22): `tools/drive/linux/box.sh up` (droplet and packages), then `tools/drive/linux/smoke.sh` (copy this checkout, build, check the first frame, a shell in a card and the Turkish Q chord, screenshots in `.scratchpad/linux-shots/`), then `box.sh down`. Each step by hand: `box.sh sync`, `box.sh build`, `box.sh r start`, `box.sh r chord super t`, `box.sh r ift ls`, `box.sh shot <name>`. `box.sh` copies the working tree with rsync, so uncommitted changes are what gets built. `LINUX_HOST=user@host` drives any Linux machine instead. Run with no verb, each script prints its verbs.

Three things the first driver run found, each now handled in `remote.sh`: a fresh droplet runs its own apt at boot and a second apt corrupts the cache (wait for `cloud-init status --wait`); a process started over ssh keeps the session open unless it and every subshell around it has no ssh descriptor (`setsid`, stdin from `/dev/null`, redirects on the whole subshell); under `set -e` and `pipefail`, `lsof` on a missing socket ends the script silently, so "not running" is answered explicitly. A later run lost its ssh connection during the silent package install ("Operation timed out"), so `box.sh` sends keepalives.

The commands underneath, for a machine the driver does not reach:

```
doctl --context ekinertac compute droplet create ift-linux-spike --image ubuntu-24-04-x64 --size s-8vcpu-16gb --region fra1 --ssh-keys 23752258 --wait
apt-get install -y gcc g++ libasound2-dev libfontconfig-dev libglib2.0-dev libssl-dev libvulkan1 libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev libzstd-dev make cmake clang lld llvm git curl pkg-config build-essential mesa-vulkan-drivers vulkan-tools xvfb imagemagick xdotool fonts-dejavu-core x11-utils openbox x11-xkb-utils
git clone https://github.com/ekinertac/infiniterm.git && cd infiniterm      # master builds on Linux since #464's keycode PR
cargo build -p infiniterm-ui --no-default-features && cargo build -p infiniterm-cli -p infiniterm-session -p infiniterm-hook
Xvfb :99 -screen 0 1920x1080x24 &
DISPLAY=:99 openbox &                          # without a window manager the first frame stays black
DISPLAY=:99 INFINITERM_DATA_DIR=/tmp/ift-spike PATH=$PWD/target/debug:$PATH infiniterm &
DISPLAY=:99 import -window root shot.png
```

The package list is most of zed's `script/linux` (the set for gpui; libgit2, libva, sqlite, pipewire and the portal were left out and were not needed) plus Mesa, Xvfb, ImageMagick and xdotool. Package install, rustup and a first debug build of the ui crate took about 15 minutes together on 8 vCPU. `iftd` is found on PATH. `xdotool windowactivate` fails without a window manager; `windowfocus` and plain `type` / `key` work. A CHORD is sent held, `xdotool keydown super minus; sleep 0.1; xdotool keyup minus super`: `xdotool key` releases the key before the app asks the X server which key is down, and the chord falls back to gpui's layout name (see the keycode paragraph below). `INFINITERM_KEYLOG=1` prints each chord with its physical code (`[chord] cmd+= (key "-", code Some("Equal"))`). The `default` context of doctl is the employer's production: always pass `--context ekinertac`, and delete the droplet afterwards (`droplet delete` has no `--wait`; poll `droplet list`).

## The macOS-only pieces and a plan for each

The Windows branch already put the seam in: Cocoa crates under `[target.'cfg(target_os = "macos")'.dependencies]`, browser cards behind a `browser` feature (on by default), and `not_macos.rs` answering seven modules under the same names. Linux uses that seam; the plan below is what each piece needs on Linux on top of it.

| Piece | On macOS | Linux plan |
|---|---|---|
| `keycode.rs` | NSEvent monitor gives the physical key code and the real Shift flag | Done on X11 (2026-10-22), see the paragraph below. Native Wayland still gets gpui's layout names. |
| Cmd key | `platform` is Cmd | gpui maps Super to `platform`, which the spike confirmed. Desktops take many Super chords themselves (GNOME: Super+number, Super+A). Decide: Super as is, or the Windows branch's `keyboard.commandModifier` (left Ctrl for the app, right Ctrl for the terminal); `roles` is `cfg(windows)` today and would need an xkb read of which Ctrl is down. Ekin's choice. |
| `ime.rs`, `composing.rs` | the macOS text input system | Compile as they are (gpui's generic input handler). gpui's Linux backends speak XIM and Wayland text-input. Dead keys: `last_dead` returns false until tested with a compose key. |
| `native_menu.rs` | NSMenu for the right-click menus | Done (2026-10-22): the app draws the menu, see the paragraph below. Serves Windows too. |
| `title_drag.rs` | AppKit drag of a transparent title bar | Drag and double-click done (2026-10-22), see the paragraph below. Client-side decorations (the app's bar as the only bar, with window buttons and resize edges) not yet. |
| `fullscreen.rs` | `toggleFullScreen:` patch | gpui's `toggle_fullscreen` on the window. |
| `icon_font.rs` | CoreText registration of the bundled Nerd Font | Load the font into gpui's text system with `add_fonts` and check that cosmic-text's fallback finds it (on the Mac it did not, which is why CoreText is used there). |
| `instance_icon.rs` | Dock icon and badge | The window icon through the `.desktop` file; no badge (a remote or joined window keeps its title and chip). |
| `finder_service.rs` | "Open in infiniterm" Finder service | None. A `.desktop` file with `MimeType=inode/directory` is the Linux equivalent, later. |
| `middle_drag.rs` | adds `otherMouseDragged:` to gpui's view | Not needed if gpui's X11/Wayland backends report middle drags; test with `xdotool mousedown 2`. |
| `system_reduces_motion` | NSWorkspace | `false` now; later the `org.gnome.desktop.interface enable-animations` setting. |
| Default fonts (`config.rs`) | `ui-monospace, Menlo` | A Linux default: `DejaVu Sans Mono, Liberation Mono, monospace`, or the family `fc-match monospace` names. |
| CEF, `app_protocol.rs`, the helper | browser cards | First version without them (`--no-default-features`). CEF has Linux builds; adding them is a later step with its own spike. |
| `osascript` dialogs (`ssh_login` askpass, the takeover's "Stuck") | dialogs before any window exists | `zenity` or `kdialog` when present, else the message on stderr; askpass also has `ssh-askpass` packages. |
| `open` (links, `open -n` in `ift connect` / `ift join`) | macOS `open` | `xdg-open` for links and files; for a second app, run the binary with the same environment. |
| Updater, `ift install`, `/Applications` paths | bundle swap, `codesign`, `spctl` | Off for the first version (the updater already refuses anything outside an Applications folder). Packaging (AppImage or .deb) decides the paths. |
| Themes and assets | copied into the bundle by `tools/bundle.sh` | Ship `assets/themes` beside the binary or under `share/infiniterm/`, and look there. |

## The physical key on Linux (2026-10-22)

gpui's Linux keystroke has the Mac's problem word for word: `Keystroke::from_xkb` names the key through the layout, clears Shift on a symbol, and drops the key code. There is no local event monitor to hook, so `keycode.rs` reads the key the other way round: the first thing every `key_down` does is `keycode::note_key_down()`, which on Linux sends one `QueryKeymap` on an X connection of our own (`x11rb`, already in the tree as gpui's dependency) and keeps the one key down that the scan code table names (`pressed_scan`; modifiers are not in the table, so a chord leaves one key; two named keys at once is ambiguous and answers `None`). Shift comes from the same bitmap (`shift_down`). A Linux evdev code is the set 1 scan code for every key in the Windows table, and an X11 key code is the evdev code plus 8, so Windows and Linux share `set1_dom_code`; its tests now run on every platform, the Mac included. Elsewhere `note_key_down` does nothing.

Checked on the droplet with `setxkbmap tr`: Super plus the key US prints `=` on (Turkish Q prints `-` there) logged `cmd+= (key "-", code Some("Equal"))` and `terminal.fontSize` went from 14 to 15; with Shift it was `cmd+shift+=` although gpui reported `_` with Shift cleared; Turkish letters typed into a card unchanged.

Limits, stated: (1) native Wayland: gpui picks Wayland whenever `WAYLAND_DISPLAY` is set, our connection then reaches XWayland, which sees no keys while a Wayland window has focus, so every query answers `None` and the keymap gets gpui's layout names (the Turkish Q `Cmd+=` bug, on Wayland only); a fix needs the key code from gpui's Wayland client or the X11 backend forced. (2) The query runs when the app handles the key, not when the server got it: a press released before that answers `None`, the same fallback. On the droplet (software rendering, so a slow main thread) holds of 40 ms and more were named 20 of 20 times, 20 ms 8 of 10; human presses last 70 to 150 ms. (3) Dead keys need nothing: gpui composes them itself with xkbcommon's compose state, so `last_dead()` is `false` on Linux. Checked with `setxkbmap us -variant intl`: the dead key arrived as a keystroke with no character (`key: "'", key_char: None`), the terminal sent nothing for it, the next key arrived as `é`, and the card read `café`. (4) Super is Cmd (`roles` is the identity off Windows); desktops take many Super chords themselves, and whether Linux gets the Windows `keyboard.commandModifier` split is Ekin's choice.

The `app` job in `.github/workflows/linux.yml` runs `cargo clippy -p infiniterm-ui --no-default-features -- -D warnings` on Ubuntu for every PR that touches the ui or editor crates, so a Mac-only call added without a gate is caught; the Mac's `make check` compiles none of the Linux code.

## The title bar on Linux (2026-10-22)

A press on the bare part of the app's title bar (a tab or the "+" stop the press) is kept as `AppView::title_press`; once the pointer has travelled `MOVE_SLOP_PX` (4) with the button held, `window.start_window_move()` hands the move to the window manager (X11 `_NET_WM_MOVERESIZE`, Wayland `xdg_toplevel.move`). A double-click is `window.zoom_window()`, maximise or restore; gpui's `titlebar_double_click` is macOS only. The move starts on motion, not on the press, because the window manager takes the pointer for the move: started on the first press it ate the second click and a double-click never reached the app (the driver's first run of check 5 failed exactly that way). The rule is `not_macos.rs::title_drag` (`starts_move`, tested); the Mac path in `overlays.rs` is the untouched `title_drag.rs` one under its own cfg, and Windows does what it did (its caption is the system's). `smoke.sh` check 5 drags the bar and asserts the window moved, then double-clicks it and asserts it fills the screen; a single click was checked to leave the window where it was.

Not done: client-side decorations. gpui asks for them with `WindowOptions::window_decorations`; on X11 it falls back to the window manager's frame when no compositor runs (openbox on Xvfb has none, so the driver sees two bars: the window manager's and the app's), and on GNOME Wayland there are no server-side decorations at all, so there the app's bar would be the only one and needs window buttons (close, minimise, maximise) and resize edges (`window.start_window_resize`). That is the next title bar step, and how the buttons look is a choice for Ekin; until he picks, Linux keeps the system frame (two bars where a window manager draws one).

## The right-click menu off macOS (2026-10-22)

gpui has no native context menu on Linux or Windows, and the Mac's `native_menu::pop_up` is a blocking call that returns the chosen row, which a menu the app draws cannot be. So `menus.rs::show_menu` builds the same rows as before and hands them to `present_menu`: on macOS the `NSMenu` as it was, elsewhere `Model::menu` (an `OpenMenu`: the rows, a `MenuNav` and the window point it opened at). Both end in `finish_menu`, which runs the command on the tab's workspace and puts out a lit tree row. Walking the menu is pure (`infiniterm-core/src/menu_nav.rs`, tested on every platform against the real terminal menu): a stack of open submenus and one selected row, separators and disabled rows never selected, moves wrap, hovering a submenu opens it without selecting inside it. `menu_nav::place` and `place_submenu` keep it in the window (upward near the bottom edge, a submenu to the left near the right edge). `menu_view.rs` draws it over the whole window (the root element is `relative` so a tab menu from the title bar and a card menu share window pixels): one panel per open level, the shortcut on the right through `shortcuts::chord_keys`, rows measured in the chrome font. While it is up it is an overlay (`overlay_open`, so the wheel and chords are gated) without a dim sheet, `input.rs` hands it EVERY key before anything else (arrows, Enter, Escape; any other key closes it and is swallowed, so a Cmd+W aimed at the menu cannot close the card behind it), and a press of any button outside the panels closes it and goes no further. The browser page's own menu (`browsers.rs::show_page_menu`) still calls `pop_up`: its rows carry actions, not commands, and browser cards are not built on Linux; on a Windows build with CEF it is still the stub. `smoke.sh` check 4 right-clicks a terminal card, screenshots the menu, chooses Close by keyboard and asserts the card is gone.

## Order

1. Base the Linux work on the Windows branch's seam, not a second seam. Done: #513 put it on master.
2. `keycode.rs` Linux functions, the Linux default fonts, the first-frame fix: the app is then usable on X11. Done: the fonts in #515, the keycode with this paragraph's PR, the first frame was the missing window manager.
3. A `tools/drive/linux/` driver (the commands above as a script on a droplet) so each change is checked with a screenshot. Done: `box.sh`, `remote.sh`, `smoke.sh`.
4. Context menu drawn by gpui (done), title bar drag (done), the icon font.
5. Packaging, then CEF.
