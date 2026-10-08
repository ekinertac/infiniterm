// Every feature, once. Rendered twice: as the landing-style page
// (src/pages/features.astro) and as a docs page (reference/features.mdx),
// so the two cannot drift apart. `text` may hold `code` in backticks;
// `href` is a path under the site, usually a guide section.
// Kept by hand: check it against CHANGELOG.md at every release.
export interface Feature { text: string; href?: string }
export interface FeatureGroup { title: string; items: Feature[] }

/** The text as HTML: escaped, with `code` spans. */
export function featureHtml(text: string): string {
  const esc = text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  return esc.replace(/`([^`]+)`/g, '<code>$1</code>');
}

export const featureGroups: FeatureGroup[] = [
  {
    title: 'Agents and card states',
    items: [
      { text: 'Four border colours: working, waiting on you, failed, done.', href: 'guide/card-states/' },
      { text: 'Claude Code, Codex, OpenCode and Pi report through hooks, installed with one command each.', href: 'guide/install/' },
      { text: 'Any command in zsh reports too: a build that runs 5 seconds goes violet, then green or red.', href: 'guide/card-states/' },
      { text: 'Each workspace tab wears one dot per card, lit in that card\'s colour.', href: 'guide/canvas/#groups-and-workspaces' },
      { text: 'Done means unseen: a green card goes grey once you have looked at it.' },
      { text: 'No notifications and no focus stealing. You look when you are ready.' },
      { text: 'Transcript cards show a Claude Code or Pi session as turns, following along while it runs.', href: 'guide/canvas/' },
      { text: '`ift ls --agents` lists every card that runs an agent, with the session id `claude --resume` takes.', href: 'guide/ift-and-sessions/' },
    ],
  },
  {
    title: 'The canvas',
    items: [
      { text: 'One zoomable canvas per workspace. `Cmd 2` fits every card, `Cmd 1` the one you are on.', href: 'guide/canvas/' },
      { text: 'A new card takes the next free slot of a grid; nothing moves on its own. Canvas: tidy puts cards back on the grid when you ask.' },
      { text: 'Drag a card by its edge or label; it snaps to the grid\'s slots, and a drop on another card swaps the two.', href: 'guide/canvas/#moving-and-resizing' },
      { text: 'Split a card to the right or below, grow it into a gap, or pick a size from a menu.' },
      { text: 'Reading mode: `Cmd 1` on a fitted terminal card zooms to 150% at its bottom, and `Cmd Up` / `Cmd Down` pan inside it.', href: 'guide/canvas/#reading-mode' },
      { text: '`Cmd Ctrl Alt` + arrow slides a card into the next gap; `Cmd Alt Shift` + arrow moves a whole selection one block.', href: 'guide/canvas/#moving-and-resizing' },
      { text: 'Select several cards with a drag or `Cmd` + click, then move, close, fit or group them together.' },
      { text: '`Cmd Alt` + arrow moves to the neighbour; `Ctrl Tab` walks the cards you used, most recent first.' },
      { text: 'Right-click opens the native Mac menu on a terminal, a card, the canvas, a tab, an editor and a browser page, with each shortcut at the right edge.', href: 'guide/canvas/#the-right-click-menu' },
      { text: 'Groups with a frame and a name; workspaces with tabs you can reorder.' },
      { text: '`Cmd Z` undoes a move, a swap, a resize or a closed card. Undo never closes a card you opened.' },
      { text: 'A closed terminal is parked for a minute first, so `Cmd Z` brings its program back mid-work.', href: 'guide/terminal/#closing-a-card' },
      { text: 'Protect a card (`Cmd Shift L`) so `Cmd W` refuses it and its shell restarts when it exits.' },
      { text: 'Mask a card (`Cmd Shift H`) while someone reads your screen.' },
      { text: 'The interface size is separate from the zoom (`Cmd Shift =` and `-`).' },
    ],
  },
  {
    title: 'Terminal cards',
    items: [
      { text: 'Shells run in their own small daemon, so they survive a quit, an update and a crash of the app.', href: 'guide/ift-and-sessions/#sessions-that-outlive-the-window' },
      { text: '`ift attach 7` takes over a card\'s shell from any terminal, also over ssh from a phone.' },
      { text: 'Find in the whole scrollback (`Cmd F`), with every match highlighted.', href: 'guide/terminal/#find' },
      { text: 'Visual mode (`Cmd Shift C`) to select and copy with the keyboard.', href: 'guide/terminal/#visual-mode' },
      { text: 'Select on the zsh command line with Shift and arrows, as in any Mac text field.' },
      { text: '`Cmd` + click opens a link or a file path. A file dropped from the Finder pastes its quoted path.' },
      { text: '`Cmd =` and `Cmd -` change the font size in every terminal at once.' },
      { text: 'The kitty keyboard protocol, so Shift+Enter is a line break in Claude Code and Pi.' },
      { text: 'Programs in a card can ask for macOS permissions (Photos, camera, folders).', href: 'guide/terminal/#privacy-prompts' },
    ],
  },
  {
    title: 'Editor, diff and browser cards',
    items: [
      { text: 'An editor with syntax highlighting for 17 languages, a file tree, find and replace, multiple cursors and tabs. No LSP, on purpose.', href: 'guide/editor/' },
      { text: '`ift file.rs` in a terminal card edits the file over that card and waits, so `EDITOR=ift` works for `git commit`.' },
      { text: 'JSON files with a `$schema` complete their keys and values; setting names complete in `settings.json`.', href: 'guide/editor/#json-with-a-schema' },
      { text: '`ift diff` opens your changes against HEAD, with a blame gutter on `Cmd B`.' },
      { text: 'Browser cards are Chromium with tabs and the Claude in Chrome extension, beside the agent that drives them.' },
      { text: 'An address bar (`Cmd L`) that takes a URL or a search, with your history.' },
      { text: 'Page cards show Markdown read-only; the "Start here" card is one.' },
    ],
  },
  {
    title: 'ift, the command line',
    items: [
      { text: 'Open files, folders and diffs from any shell.', href: 'guide/ift-and-sessions/' },
      { text: '`ift send`, `ift read`, `ift close` and `ift run` drive another card by its number.', href: 'guide/ift-and-sessions/#driving-another-card' },
      { text: '`ift ls`, `ift sessions` and `ift commands` print tables, or tab-separated rows into a pipe.' },
      { text: '`ift usage 30` shows which commands and gestures you used in 30 days, from a local log.' },
      { text: 'zsh completion for every verb.' },
    ],
  },
  {
    title: 'Servers',
    items: [
      { text: '`ift connect user@host` opens a second window whose terminal cards run on that server over ssh.', href: 'guide/ift-and-sessions/#servers' },
      { text: 'The shells stay on the server when the window closes; connecting again brings the same cards back.' },
      { text: '`--install` puts `ift` and `iftd` on a Linux server through the ssh connection, checksum checked.' },
      { text: 'Agent states and `ift` work inside a server\'s cards too.' },
      { text: 'Each server gets its own colour, settings and Dock icon.' },
    ],
  },
  {
    title: 'Look and settings',
    items: [
      { text: '522 themes, previewed live as you move through the list.', href: 'guide/configuration/#themes' },
      { text: 'Settings as flat dotted keys, with every default documented beside your file. Changes apply on save.', href: 'guide/configuration/#settings' },
      { text: '`settings.json` is checked before it applies: a mistake keeps your old settings and the status bar says why.', href: 'guide/configuration/' },
      { text: 'Every binding can be changed; chords follow the physical key on any keyboard layout.', href: 'guide/configuration/#keybindings' },
      { text: 'A binding can carry a `when`, as in VS Code, so it applies only in a locked editor, a browser card or an empty slot.', href: 'guide/configuration/#bindings-that-apply-only-sometimes' },
      { text: 'A see-through window, background pictures that rotate, see-through cards, rounded corners, the gap between cards.', href: 'guide/configuration/#the-window-and-the-canvas' },
      { text: 'A title bar and Dock colour per window.' },
      { text: 'Snippets: plain files in a folder, pasted into the card with `Cmd Ctrl S`.', href: 'guide/configuration/#snippets' },
      { text: 'Any file infiniterm writes can be a symlink into your dotfiles; the link stays.' },
    ],
  },
  {
    title: 'The app',
    items: [
      { text: 'Native, in Rust. No Electron, no account, no telemetry.', href: 'guide/configuration/#what-the-app-touches' },
      { text: 'Signed and notarized, and it updates itself; the update check is a plain GET.' },
      { text: 'Install with one `curl` line, Homebrew or a DMG.', href: 'guide/install/' },
      { text: 'Free for personal use, $29 per person for work, on the honour system.', href: 'guide/install/#registering-a-licence' },
    ],
  },
];
