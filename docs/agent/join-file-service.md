# A file service for `ift join` (proposal, #560)

Status: step 1 BUILT (2026-10-24, #560): `file_proto.rs` (frames, requests), `file_source.rs` (`FileSource`, `LocalSource`), `file_server.rs` (the host's loop, `ift proxy files`), `backend/file_client.rs` (`HostSource`), requests home, stat, exists, list and read, and the PAGE card on it (`PageBody` takes a source; a host page loads on a thread with a "Loading from the host" line, polls the mtime, keeps its text when the host is lost, and a link in it asks the host to open it). `AppView::file_source()` hands a joined window one `HostSource`, connected at the first request. Everything below the first table is still the plan for the later steps; the changes against it are noted in the commit history of the step PRs. Covers: why a joined window needs it, the helper, the wire messages, where the file-source seam sits in the code, and the order cards switch in. Read with `join-viewer.md` (how a viewer works) and `join-serve-and-login.md` (the host without an app).

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
| `watch` | `{ "path", "recursive": bool }` | `{ "watch": id }`; then `W` events `{ "paths": [..] }` coalesced over 100 ms |
| `unwatch` | `{ "watch" }` | `{}` |
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
