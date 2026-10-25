# A file service for `ift join` (proposal, #560)

Status: step 5, PICTURES, BUILT (2026-10-25): no new request. gpui decodes a picture from a path on this Mac, so a host picture is copied through `stat` + `read` into `<data>/host-pictures/` (`host_pictures.rs`: `fetch`, named `<fnv hash of the host path>-<mtime>-<size>.<ext>`, written to `.part` and renamed, older copies of the same host path removed) and the painters get the cached path; a changed file is a NEW path, so gpui's per-path asset cache cannot serve the old picture. An editor card on a host image (`start_host_picture`) says "Loading from the host..." until the copy is in, draws it, and its disk poll/watch refetches on a changed mtime (`DiskAnswer::Picture`); a missing picture is text in the card. The canvas background list (`background_show.rs`) in a viewer: `host_pictures::is_host_entry` says an entry (`~/x`, an absolute path, anything with a `/` or an extension) is the host's, a bundled name is the app's own; the host's `~` is the host's home (`home` request); each entry is fetched on its own thread, the list grows as copies arrive (the rotation restarts then), a failed one is skipped and not waited for (`needs_frame` asks for frames only while some are on the way). shortcut: a background that changes on the host is seen at the next settings change or restart, and nothing prunes the cache folder except a replaced copy; add an age sweep if a rotation of hundreds is ever pointed at a host.

Status: #556's UPLOAD is on the file connection (2026-10-25): one new request, `put_drop { id, name }` with the bytes as `D` frames like a `write`, answered by `FileSource::put_drop` (`LocalSource` -> `put_drop_in`): the host tidies drop folders older than `KEEP_DAYS` in `~/.infiniterm/drops`, makes `<id>/`, writes `<name>` atomically and replies `{path}`. Id and name are made plain path components on the host (`host_name`), so the request can write nowhere but the drops folder; it is the only request that makes a folder (a general `mkdir` or `remove` was not wanted). `host_drop::send_path_via` / `send_image_via` use it for a file up to `SERVICE_MAX` (64 MiB: it goes whole in one request and sits in memory at both ends) and fall back to the old ssh `cat` for `unsupported` (an older host's ift), `lost` (no connection) and `too_large`; any other failure is reported, not retried over ssh. A FOLDER still goes as a tar over ssh (links and modes survive; per-file writes would lose them).

Status: #559 BUILT (2026-10-25): "open settings" and "open keybindings" in a viewer open the host's files, replacing the paused host-opens-cards design. `Model::open_config_pair` in a viewer (`read_only && snapshot_seen`) does not use this Mac's `config_path`: with `host_config_dir` unknown it pushes `Effect::AskHostConfig`, the app asks `source.home()` on a thread (`ask_host_config`, answer taken in `drain_canvas`) and calls `Model::host_config_known`, which keeps the folder and opens the pair on the host's paths (`<dir>/settings.default.json` + `settings.json`, joined as text). The cards are ordinary viewer-born editor cards: they go up as part of the viewer's canvas, the host's merge makes them, and their bodies read and write through `HostSource` (the defaults file is read-only by name, `is_generated`). A save lands in the host's `settings.json`; the host's watcher publishes `S` back to every viewer (#433). Until the answer arrives a notice says the host is being asked. Step 3 BUILT (2026-10-25): the FILE TREE and `watch`. `HostDirs` (`infiniterm-ui/src/host_dirs.rs`) is the tree's lister: local, a folder is listed on the spot as before; on a host the lister answers an unlisted folder with nothing, asks `list` on a thread, and `tick` (from `EditorBody::idle`) hands the entries to the tree (`Tree::set_listing`). The root says "Loading from the host..." in one line over the rows (`tree_status`, counted in `tree_head_rows` so clicks stay aligned) until it is listed, and a failed root says why in the same line ("Could not list this folder on the host: ...") and as a notice, once per message. A subfolder that is `not_found` lists as empty and its parent's next listing drops it; any other failure keeps the old entries. `ift -n .` (an editor card with a root and no file) now shows the host's tree. `watch { path }` is the one new request pair (`watch`, `unwatch`): the reply comes once the watch is set, the watch's id IS the request's id, and `W` events carry it. The helper's watch is a thread that compares a fingerprint every 400 ms (a file's size and mtime, or a folder's entry names and kinds), so only a change crosses the link; shortcut: swap in OS file events if a host ever carries hundreds. `HostSource::watch` returns a `Watch` (events channel, `poll()` says changed/closed, dropping it sends `unwatch`); a lost connection closes every watch, which the owner sees as `closed` and answers by listing or reading once and setting a new one. `LocalSource::watch` is `unsupported`, and so is an older host's answer: those cards poll every `DISK_POLL_MS` as before. The open file and every shown folder are watched; the 2 s poll runs only for what has no watch. Opening a file from the tree opens a tab whose body takes the card's source, so it reads from the host. The tree moves between tabs with `put_tree`, which tells the new body's `HostDirs` the folders are already listed (`adopt`).

Status: step 4 (b) BUILT (2026-10-25): the TRANSCRIPT and DIFF cards in a viewer. `TranscriptBody` takes a source (`set_source`; "Loading from the host..." until the first answer): a host's session file is asked about every 2 s on a thread, its mtime by `stat` first, the whole file read and parsed (`parse_transcript`, in the thread) only when it moved; the error text shows only while the list is empty, so a lost link keeps the turns. shortcut: the whole file is read each time it moves; read from the old size on if a session of tens of MB over a slow link makes that hurt. The DIFF card (an `EditorBody` in diff mode) does every step through its source: `diff_refresh` -> `git_changes`, `diff_show` -> `git_show(repo, rel)` (new request `git_show`: HEAD's text by repo-relative name, because the file may be gone from the disk and `git_head` takes a path) plus `read` of the file, blame -> `git_blame`, and its 2 s poll is a `stat` of the shown file. On this Mac's disk every step is immediate as before; for a host the thread sends a `DiffAnswer` that `diff_idle` takes (`diff_take_answers`) and the same `diff_apply_*` functions the local path calls apply. A card started on a file or the list waits for the first list (`DiffView.start`) and says "Loading from the host..."; a git error from the host is the buffer's text and a notice. Step 4 (a) BUILT (2026-10-25): git on the host for the editor's gutter. Three requests, answered by `LocalSource` through the same `git.rs` code, so there is no new git logic: `git_head { path }` (one request does `repo_of` and `git show HEAD:rel`: the reply is `{repo, rel}` and the text as `D` chunks; `not_found` outside a repository, an empty text for a file HEAD does not have), `git_changes { path }` (`Changes` as JSON, for the diff card) and `git_blame { repo, path }`. The trait methods default to `unsupported`, so an older host's helper (which answers `unsupported` to the op) and a source with no git just give no gutter. The editor asks `source.git_head` for local and host alike; a host's is asked on a thread, one at a time, at load and then every `HOST_GIT_POLL_MS` (10 s, since a commit is rare and each ask is a `git show` over the link; shortcut: watch the repo's HEAD to make it instant). `set_git_head` drops the gutter cache when HEAD changes (the cache is keyed by buffer version, which a commit does not move). A trap found on the way: git names the repo root with symlinks resolved (`/private/var` for `/var` on a Mac), so the file's path is made relative by canonicalising when the plain prefix fails. The diff card and blame in a viewer are step (b); `git_changes` and `git_blame` have a client and a server and tests but no body uses them yet. Step 3 BUILT (2026-10-25): the FILE TREE and `watch`. `HostDirs` (`infiniterm-ui/src/host_dirs.rs`) is the tree's lister: local, a folder is listed on the spot as before; on a host the lister answers an unlisted folder with nothing, asks `list` on a thread, and `tick` (from `EditorBody::idle`) hands the entries to the tree (`Tree::set_listing`). The root says "Loading from the host..." in one line over the rows (`tree_status`, counted in `tree_head_rows` so clicks stay aligned) until it is listed, and a failed root says why in the same line ("Could not list this folder on the host: ...") and as a notice, once per message. A subfolder that is `not_found` lists as empty and its parent's next listing drops it; any other failure keeps the old entries. `ift -n .` (an editor card with a root and no file) now shows the host's tree. `watch { path }` is the one new request pair (`watch`, `unwatch`): the reply comes once the watch is set, the watch's id IS the request's id, and `W` events carry it. The helper's watch is a thread that compares a fingerprint every 400 ms (a file's size and mtime, or a folder's entry names and kinds), so only a change crosses the link; shortcut: swap in OS file events if a host ever carries hundreds. `HostSource::watch` returns a `Watch` (events channel, `poll()` says changed/closed, dropping it sends `unwatch`); a lost connection closes every watch, which the owner sees as `closed` and answers by listing or reading once and setting a new one. `LocalSource::watch` is `unsupported`, and so is an older host's answer: those cards poll every `DISK_POLL_MS` as before. The open file and every shown folder are watched; the 2 s poll runs only for what has no watch. Opening a file from the tree opens a tab whose body takes the card's source, so it reads from the host. The tree moves between tabs with `put_tree`, which tells the new body's `HostDirs` the folders are already listed (`adopt`).

Step 2 BUILT too (2026-10-24): `write` (the bytes ride `D` frames of the request's id, then the `Q`; `LocalSource::write` checks `expect_mtime_ms` just before a temp-file-and-rename write and refuses with `changed_on_host` carrying the file's mtime now, in the error's `mtime_ms`), and the EDITOR card on a source: `EditorBody` loads, saves and polls a host's file on threads (`host_locked` while loading or after a read that failed for any reason but not-found, so a stand-in buffer can never be saved over a file nobody read; a refused save keeps the changes, says so, takes the host's mtime and the next save overwrites on purpose). A joined window builds editor bodies for host cards; a host's folder (no tree), pictures, the git gutter, diffs and transcripts wait for their steps. Step 1 BUILT (2026-10-24, #560): `file_proto.rs` (frames, requests), `file_source.rs` (`FileSource`, `LocalSource`), `file_server.rs` (the host's loop, `ift proxy files`), `backend/file_client.rs` (`HostSource`), requests home, stat, exists, list and read, and the PAGE card on it (`PageBody` takes a source; a host page loads on a thread with a "Loading from the host" line, polls the mtime, keeps its text when the host is lost, and a link in it asks the host to open it). `AppView::file_source()` hands a joined window one `HostSource`, connected at the first request. Everything below the first table is still the plan for the later steps; the changes against it are noted in the commit history of the step PRs. Covers: why a joined window needs it, the helper, the wire messages, where the file-source seam sits in the code, and the order cards switch in. Read with `join-viewer.md` (how a viewer works) and `join-serve-and-login.md` (the host without an app).

## The problem

In a joined window every card is a card of the HOST's canvas, and a card's content is read from disk by the machine that draws it. The viewer draws, so it looks for the host's path on its own disk. A terminal card works because its bytes come over the card's own connection. An editor, diff, transcript or page card has no such connection: the viewer shows a placeholder ("editor on host"), and #557 (Cmd+click on a path) made that visible, because the host now opens the card and the viewer cannot fill it.

Where the disk is read today (found by reading, 2026-10-24):

| What | Where | Calls |
|---|---|---|
| Editor load, save, 2 s disk poll | `infiniterm-ui/src/editor_body.rs` | `file_read`, `file_write`, `file_mtime` (`infiniterm-core/src/files.rs`) |
| File tree | `editor_body.rs`, `infiniterm-editor/src/explorer.rs` | `dir_list` (one call per folder, asked once) |
| Git gutter and blame | `editor_body.rs` | `git_show_head`, `repo_of`, `git_blame` (`infiniterm-core/src/git.rs`, they run `git`) |
| Diff card | `infiniterm-ui/src/editor_body/diff.rs` | `git_changes`, `git_blame`, `file_read`, `file_mtime`, and one direct `git` call |
| Page card | `infiniterm-ui/src/page_body.rs` | `file_read`, `file_mtime` |
| Transcript card | `infiniterm-ui/src/transcript_body.rs`, `infiniterm-core/src/transcript.rs` | `file_mtime`, `transcript_read` (`std::fs`) |
| Picture in an editor card | `editor_body.rs` | gpui's asset cache by path |
| Remote-path checks | `links_fs.rs` | `resolve`, `path_kinds` (#557 already answers these on the host for a click) |

Drafts (`draft_*`) stay local to whoever is typing: they are the viewer's unsaved text, keyed by card id, and must survive a lost connection.

The tree has no file operations (no create, rename or delete). The service does not need them yet.

## Shape (decided in #560)

- One extra persistent ssh per join, used only for files. A helper on the host, started by the viewer, speaks a framed request/reply protocol on stdin and stdout. Not scp or sftp: the tree, the gutter and the diff card make many small requests.
- Separate from the card connections and the canvas connection. A big read must not delay a keystroke, and a shared ssh stalls typing under a flood (measured in #423).
- Needs nothing new on the host: `ift` is already there for any host you can join. It works with the host's app closed, because it does not use `j.sock`.

## The helper: `ift proxy files`

`ssh host ift proxy files`. It is plumbing, like `ift proxy join`: in neither USAGE nor the completion. The viewer starts it the way it starts a card connection (`remote::connect_command`, same ssh arguments, same shared-login `ControlPath` when the host is a password host, same back-off). It has no state beyond its watches. It does not touch the canvas, the model or `j.sock`.

Handshake, as the join protocol does it: the viewer sends the line `ift-files <major>`, the helper answers `ift-files <major> ok` or `ift-files <major> error <sentence>`. A host whose `ift` does not know `proxy files` answers with a shell error where the line should be, and the viewer says "update infiniterm there", as it does for join. Major 1.

## Frames

Binary, because a file is bytes:

```
len:u32 BE | kind:u8 | id:u32 BE | jlen:u32 BE | json (jlen bytes) | bytes (the rest)
```

`len` counts everything after itself. One frame is at most 8 MiB; a read is cut into chunks of 256 KiB. `kind`:

- `Q` request (viewer to host), `id` chosen by the viewer, never reused while open.
- `R` the final reply to `id`.
- `D` a data chunk of a reply to `id` (read, search matches), sent before its `R`.
- `E` an error reply to `id`: `{ "code": "...", "message": "..." }`.
- `W` an event with no request: a watch fired. `id` is the watch's id.

Replies to different ids may interleave. The helper runs each request on its own thread (a `git blame` must not hold a `stat`). The viewer matches by id and keeps a table of open ids.

Error codes: `not_found`, `denied`, `not_a_directory`, `is_a_directory`, `too_large`, `changed_on_host`, `io` (any other, with the OS message), `unsupported` (a request this helper's major does not know).

## Requests

Paths are the host's, absolute. `~` is expanded by the helper. A relative path is refused (`io`); the viewer resolves against the card's cwd first.

| Request | JSON | Reply |
|---|---|---|
| `home` | none | `{ "home": "/Users/x", "config_dir": "..." }` (so the viewer can name `~/.config/infiniterm/settings.json` of the host, #559) |
| `stat` | `{ "path" }` | `{ "kind": "file\|dir\|other", "size", "mtime_ms", "symlink": bool }` |
| `exists` | `{ "paths": [..] }` | `{ "kinds": ["file"\|"dir"\|null, ..] }` (the batch `path_kinds` is today) |
| `list` | `{ "path" }` | `{ "entries": [{ "name", "is_dir" }] }`, sorted as `dir_list` sorts, `.git` left out |
| `read` | `{ "path", "offset"?, "len"? }` | `D` chunks, then `{ "size", "mtime_ms" }` |
| `write` | `{ "path", "expect_mtime_ms"?, "expect_size"? }` and the bytes in the same frame (one frame, up to 8 MiB; larger is chunked as `D` frames before the `Q` is completed, see below) | `{ "mtime_ms" }` |
| `watch` | `{ "path" }` (built: not recursive; one watch per shown folder and one for the open file) | `{ "watch": id }` where id is the request's own id; then `W` events with that id, `{ "paths": [..] }` |
| `unwatch` | `{ "watch": id }` (sent when a `Watch` is dropped, not waited for) | `{}` |
| `git_repo_of` | `{ "path" }` | `{ "repo": "/abs" }` or `not_found` |
| `git_head_text` | `{ "repo", "path" }` | `D` chunks of `git show HEAD:path`, then `{}` |
| `git_changes` | `{ "path" }` | the `Changes` value `git_changes` returns today, as JSON |
| `git_blame` | `{ "repo", "path" }` | `{ "lines": [..] }` as `git_blame` returns |
| `git_diff` | what the diff card's direct `git` call asks | its output |
| `search` | `{ "root", "pattern", "options" }` | `D` chunks of matches as they are found, then `{ "done": true, "truncated": bool }` |
| `cancel` | `{ "id" }` | `{}` (a search or a read the viewer no longer wants) |

`write` is whole-file, to a temp file in the same folder and then `rename`, through the same symlink-aware `write_atomically` a local save uses. If `expect_mtime_ms` or `expect_size` is given and the file differs, the reply is `E changed_on_host` and nothing is written. A write bigger than one frame is sent as `D` frames carrying the same id, then the `Q`; the helper holds them in a temp file until the `Q` arrives and drops them if the connection ends first.

The git requests run the host's `git` against the host's repository, with the same code as `git.rs` (the helper links `infiniterm-core`). No new git logic.

## Where the seam sits

A trait in core, `infiniterm-core/src/file_source.rs`:

```
pub trait FileSource: Send + Sync {
    fn read(&self, path: &str) -> Result<FileData, FileError>;     // text or bytes, size, mtime
    fn stat(&self, path: &str) -> Result<Stat, FileError>;
    fn list(&self, path: &str) -> Result<Vec<DirEntry>, FileError>;
    fn write(&self, path: &str, bytes: &[u8], expect: Option<Stamp>) -> Result<Stamp, FileError>;
    fn git_head_text(&self, repo: &str, path: &str) -> Result<String, FileError>;
    fn git_repo_of(&self, path: &str) -> Result<String, FileError>;
    fn git_changes(&self, path: &str) -> Result<Changes, FileError>;
    fn git_blame(&self, repo: &str, path: &str) -> Result<Vec<BlameLine>, FileError>;
    fn watch(&self, path: &str, recursive: bool) -> Result<Watch, FileError>;
}
```

Two implementations:

- `LocalSource` (`file_source.rs`): the functions that exist today, unchanged (`files.rs`, `git.rs`). Every card that is not a viewer's keeps this, so the macOS app, `ift serve` on the host and the Linux and Windows ports behave as now.
- `HostSource` (`backend/file_client.rs`): one per join, a client of the persistent connection. It owns the id table, the reconnect with the card links' back-off (`LostLink`'s numbers), and turns a lost connection into `FileError::Lost` for every request in flight.

A card holds an `Arc<dyn FileSource>`, chosen when its body is built: in a viewer, `HostSource`; everywhere else, `LocalSource`. The viewer builds editor, diff, transcript and page bodies for host cards instead of placeholders (`reconcile_editors` and `reconcile_browsers` are skipped in a viewer today). A browser card stays a placeholder: its page loads on the machine that draws it by design, and the model already refuses it in a remote window.

The editor crate (`infiniterm-editor`) stays free of this: the buffer is text and does not read files. The explorer asks for a folder through a callback the body gives it, which is `source.list`.

## The cost to say out loud: these calls are synchronous today

`EditorBody::load`, the 2 s poll, `dir_list` for a folder and the gutter's `git_show_head` run on the UI thread, because a local disk answers in microseconds. A call over ssh is milliseconds to seconds, so a viewer cannot call the same way. The change is to make the bodies request and then poll: a body in state `Loading { request }`, a result channel drained in the poll tick, and an unchanged-looking card until the data arrives (a one-line "loading from the host" in the card). The local path can stay synchronous behind the same trait (`LocalSource` answers at once); the bodies handle both by always asking through the `Pending` handle `FileSource` returns (an already-resolved one for local). This is the bulk of the work and the reason the first cards below are the small ones.

## Order of the work

Each step is a PR with its own tests, and nothing before step 1 changes any local behaviour.

1. **The helper and the trait, with one reader.** `ift proxy files` (stat, exists, list, read, home), the framing and the handshake, `LocalSource`, `HostSource` with reconnect, and the page card on it (a read and an mtime: the smallest body). Tests: the frame codec (round trip, a chunked read, a frame cut in two, an oversize frame refused), the helper against a stand-in ssh, a lost connection failing the requests in flight and reconnecting. The same contract suite runs against `LocalSource` and `HostSource`, so they cannot drift.
2. **Editor and transcript read and save.** `write` with the change check; a refusal opens the conflict flow an external edit opens today (`disk_stamp` differs). Open offline: the buffer and the draft stay, the save says the host is unreachable.
3. **The file tree and watch.** `list` per folder, `watch` replacing the 2 s poll for the open file and the shown folders.
4. **Git: the gutter, blame and the diff card.** The git requests.
5. **Search in files** and **pictures** (`read` bytes into gpui's image path instead of an asset path).
6. **#559, open settings in a joined window.** `home` gives the host's config folder; `settings.json` and `keybindings.json` open as editor cards whose source is `HostSource` and whose path is the host's; saving writes the host's file; the host's watcher already publishes `S` and `K` back to every viewer (#433). The viewer's own empty config folder is never opened. Until step 2 lands, the command shows a notice instead of empty cards.
7. **#556's upload moves** to this connection (`cat` for a file today goes over a card's ssh and is one new login per drop; here it is one `write` per file). No behaviour change.

#557 stays as it is: the host still creates the card. What changes is that the card's content now loads in the viewer.

## What this does not do, on purpose

- No language servers, no remote terminals through this connection, no general file manager (out of scope in #560).
- No path restrictions and no read-only mode: a viewer has full control of the host, as in #423, and ssh is the only way in. The helper runs as the user who logged in and can read and write what that user can.
- No caching beyond what a body already keeps in its buffer. A cache needs invalidation, and `watch` plus the mtime check on save already give the correctness that matters.

## Questions for review

1. The helper name `ift proxy files` and its frame layout. Is a binary frame fine, or would you rather have newline JSON with base64 bodies (simpler to read in a log, about a third larger on the wire)?
2. A viewer card on `HostSource` for `~/.config/infiniterm/settings.json` of the host: should the viewer's own settings editor (the palette's "open settings", which writes the viewer's own empty folder today) always be the host's in a viewer, or only after a choice? The proposal says always, since the settings are the host's (#433).
3. One persistent files connection per join, started at the first need (a card that needs it) or at join? The proposal says at first need, and closed with the window.
4. Step order: is step 1 with the page card the right first PR, or the editor (what #557 shows first)?
