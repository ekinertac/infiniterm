//! A small Language Server Protocol client, for completion (#238, #602). It
//! speaks JSON-RPC over a server's stdin and stdout (rust-analyzer,
//! typescript-language-server, pyright, gopls: external programs the user
//! installs), keeps the documents it is told about in sync with full-text
//! changes, and asks `textDocument/completion`. Nothing here draws or
//! blocks: the server's replies are read on a thread and the editor takes
//! them in its idle step.
//!
//! Off unless `editor.lsp.enabled`. A server starts lazily, on the first
//! completion request for a language and project (`client_for`), and stops
//! when the last editor holding its `Arc` lets go (`Drop` kills it), so a
//! closed Rust tab costs no rust-analyzer. With the setting off no thread
//! and no process exists.
//!
//! Called by `infiniterm-ui/src/editor_body.rs`. Related: `format.rs` (finding
//! a program), `infiniterm-core/src/complete.rs` (the popup this feeds).
//! Pure apart from the process; the framing and the item conversion are
//! tested without one, and the whole client against a scripted server.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

/// How long a server gets to answer `initialize` before it counts as dead:
/// rust-analyzer on a large workspace takes a while, but a completion popup
/// waiting on it is no use, so the requests simply stay unanswered until it
/// is ready.
pub const INIT_TIMEOUT: Duration = Duration::from_secs(60);

/// The server command for `language` (the badge name), when none is set.
pub fn default_server(language: &str) -> Option<&'static str> {
    Some(match language {
        "rust" => "rust-analyzer",
        "javascript" | "typescript" | "tsx" => "typescript-language-server --stdio",
        "python" => "pyright-langserver --stdio",
        "go" => "gopls",
        _ => return None,
    })
}

/// The `languageId` an LSP server expects for a badge name.
pub fn language_id(language: &str) -> &str {
    match language {
        "tsx" => "typescriptreact",
        "shell" => "shellscript",
        other => other,
    }
}

// ----- framing -----

/// One message as `Content-Length: n\r\n\r\n<json>`.
pub fn encode(msg: &Value) -> Vec<u8> {
    let body = msg.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// The next framed message, `Ok(None)` at end of input.
pub fn read_message(r: &mut impl BufRead) -> std::io::Result<Option<Value>> {
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_string());
    let mut len = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            len = v.trim().parse::<usize>().ok();
        }
    }
    let len = len.ok_or_else(|| bad("no Content-Length"))?;
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| bad(&e.to_string()))
}

// ----- positions and uris -----

/// A `file://` URI for an absolute path, percent-encoding what a URI cannot hold.
pub fn file_uri(path: &str) -> String {
    let mut out = String::from("file://");
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// A char column of `line` as LSP counts it: UTF-16 code units.
pub fn utf16_col(line: &str, col_chars: usize) -> usize {
    line.chars().take(col_chars).map(char::len_utf16).sum()
}

/// The folder a language server should treat as the project: the nearest
/// ancestor holding a marker file, else the file's own folder.
pub fn project_root(path: &str) -> String {
    const MARKERS: &[&str] = &[
        ".git",
        "Cargo.toml",
        "package.json",
        "tsconfig.json",
        "go.mod",
        "pyproject.toml",
        "setup.py",
    ];
    let dir = std::path::Path::new(path)
        .parent()
        .unwrap_or(std::path::Path::new("/"));
    // Cargo.toml nearest first is right for a crate in a workspace too: the
    // server finds the workspace root itself from there.
    dir.ancestors()
        .find(|d| MARKERS.iter().any(|m| d.join(m).exists()))
        .unwrap_or(dir)
        .to_string_lossy()
        .into_owned()
}

// ----- items -----

/// One completion the server offered, ready for the popup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    /// What replaces the typed word: the server's text edit or insert text
    /// with any snippet syntax flattened, else the label.
    pub insert: String,
    pub detail: String,
    pub doc: String,
    /// What the server sorts it by.
    pub sort: String,
}

/// A snippet's placeholders flattened to the text a plain insert needs:
/// `foo(${1:a}, $2)$0` is `foo(a, )`.
pub fn flatten_snippet(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else {
                out.push(c);
            }
            continue;
        }
        match chars.peek() {
            Some(d) if d.is_ascii_digit() => {
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    chars.next();
                }
            }
            Some('{') => {
                chars.next();
                let mut depth = 1;
                let mut inner = String::new();
                for n in chars.by_ref() {
                    match n {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    inner.push(n);
                }
                // `1:name` keeps `name`; `1|a,b|` keeps `a`; `1` keeps nothing.
                if let Some((_, text)) = inner.split_once(':') {
                    out.push_str(&flatten_snippet(text));
                } else if let Some((_, rest)) = inner.split_once('|') {
                    out.push_str(rest.trim_end_matches('|').split(',').next().unwrap_or(""));
                }
            }
            _ => out.push('$'),
        }
    }
    out
}

/// The items of a `textDocument/completion` result (a list or an
/// `{ items }` object), in the server's sort order.
pub fn parse_items(result: &Value) -> Vec<Item> {
    let list = result
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| result.as_array());
    let mut items: Vec<Item> = list
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let label = v.get("label")?.as_str()?.to_string();
            let snippet = v.get("insertTextFormat").and_then(Value::as_u64) == Some(2);
            let text = v
                .pointer("/textEdit/newText")
                .or_else(|| v.get("insertText"))
                .and_then(Value::as_str)
                .unwrap_or(&label);
            let insert = if snippet {
                flatten_snippet(text)
            } else {
                text.to_string()
            };
            let doc = match v.get("documentation") {
                Some(Value::String(s)) => s.clone(),
                Some(d) => d
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                None => String::new(),
            };
            Some(Item {
                sort: v
                    .get("sortText")
                    .and_then(Value::as_str)
                    .unwrap_or(&label)
                    .to_string(),
                detail: v
                    .get("detail")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                insert,
                doc,
                label,
            })
        })
        .collect();
    items.sort_by(|a, b| a.sort.cmp(&b.sort));
    items
}

// ----- the client -----

#[derive(Default)]
struct Shared {
    next_id: u64,
    /// Answers taken off the wire and not yet taken by the editor.
    results: HashMap<u64, Vec<Item>>,
    pending: HashSet<u64>,
    /// `initialize` has been answered and `initialized` sent.
    ready: bool,
    /// The server's stdout closed.
    dead: bool,
    /// Messages sent before the server was ready, sent once it is.
    queue: Vec<Value>,
    /// Documents the server has been told about, by uri.
    open: HashSet<String>,
}

pub struct Client {
    stdin: Arc<Mutex<ChildStdin>>,
    child: Mutex<Option<Child>>,
    shared: Arc<Mutex<Shared>>,
    pub command: String,
    pub root: String,
}

fn write(stdin: &Mutex<ChildStdin>, msg: &Value) {
    if let Ok(mut s) = stdin.lock() {
        let _ = s.write_all(&encode(msg));
        let _ = s.flush();
    }
}

impl Client {
    /// Starts `command` (words, no shell) in `root` and sends `initialize`.
    /// The reply is read on a thread; until it comes, messages wait in a queue.
    pub fn start(command: &str, root: &str) -> Result<Arc<Client>, String> {
        let (program, args) =
            crate::format::split_command(command, "").ok_or("no server command")?;
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let path_var = std::env::var("PATH").unwrap_or_default();
        let exe =
            crate::format::find_program(&program, std::path::Path::new(root), &path_var, &home)
                .ok_or_else(|| format!("{program} not found"))?;
        let mut child = Command::new(&exe)
            .args(&args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{program}: {e}"))?;
        let stdin = Arc::new(Mutex::new(child.stdin.take().ok_or("no stdin")?));
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let shared = Arc::new(Mutex::new(Shared {
            next_id: 1,
            ..Default::default()
        }));
        let init_id = 0;
        write(
            &stdin,
            &json!({
                "jsonrpc": "2.0", "id": init_id, "method": "initialize",
                "params": {
                    "processId": std::process::id(),
                    "rootUri": file_uri(root),
                    "capabilities": {"textDocument": {
                        "synchronization": {"didSave": false},
                        "completion": {"completionItem": {
                            "snippetSupport": false,
                            "documentationFormat": ["plaintext"]
                        }}
                    }},
                    "workspaceFolders": [{"uri": file_uri(root), "name": "root"}],
                }
            }),
        );
        let (r_stdin, r_shared) = (stdin.clone(), shared.clone());
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stdout);
            while let Ok(Some(msg)) = read_message(&mut reader) {
                handle(&r_stdin, &r_shared, init_id, msg);
            }
            if let Ok(mut s) = r_shared.lock() {
                s.dead = true;
            }
        });
        Ok(Arc::new(Client {
            stdin,
            child: Mutex::new(Some(child)),
            shared,
            command: command.to_string(),
            root: root.to_string(),
        }))
    }

    pub fn is_dead(&self) -> bool {
        self.shared.lock().map(|s| s.dead).unwrap_or(true)
    }

    pub fn is_ready(&self) -> bool {
        self.shared.lock().map(|s| s.ready).unwrap_or(false)
    }

    /// Sends now when the server is ready, else queues it for then.
    fn send(&self, msg: Value) {
        let Ok(mut s) = self.shared.lock() else {
            return;
        };
        if s.ready {
            drop(s);
            write(&self.stdin, &msg);
        } else {
            s.queue.push(msg);
        }
    }

    /// Tells the server the text of `uri` as of `version`: opens it the first
    /// time, replaces the whole text after that.
    pub fn sync(&self, uri: &str, language: &str, version: u64, text: &str) {
        let first = self
            .shared
            .lock()
            .map(|mut s| s.open.insert(uri.to_string()))
            .unwrap_or(false);
        if first {
            self.send(json!({
                "jsonrpc": "2.0", "method": "textDocument/didOpen",
                "params": {"textDocument": {
                    "uri": uri, "languageId": language_id(language),
                    "version": version, "text": text
                }}
            }));
        } else {
            self.send(json!({
                "jsonrpc": "2.0", "method": "textDocument/didChange",
                "params": {
                    "textDocument": {"uri": uri, "version": version},
                    "contentChanges": [{"text": text}]
                }
            }));
        }
    }

    pub fn close(&self, uri: &str) {
        let was_open = self
            .shared
            .lock()
            .map(|mut s| s.open.remove(uri))
            .unwrap_or(false);
        if was_open {
            self.send(json!({
                "jsonrpc": "2.0", "method": "textDocument/didClose",
                "params": {"textDocument": {"uri": uri}}
            }));
        }
    }

    /// Asks for the completions at a position (`line` 0-based, `col` in
    /// UTF-16 units); the answer is taken later with `take_completion`.
    pub fn request_completion(&self, uri: &str, line: usize, col: usize) -> u64 {
        let id = {
            let Ok(mut s) = self.shared.lock() else {
                return 0;
            };
            let id = s.next_id;
            s.next_id += 1;
            s.pending.insert(id);
            id
        };
        self.send(json!({
            "jsonrpc": "2.0", "id": id, "method": "textDocument/completion",
            "params": {
                "textDocument": {"uri": uri},
                "position": {"line": line, "character": col}
            }
        }));
        id
    }

    /// The answer to request `id` once it has come, taken (once).
    pub fn take_completion(&self, id: u64) -> Option<Vec<Item>> {
        self.shared.lock().ok()?.results.remove(&id)
    }

    /// Whether `id` is still waiting for its answer.
    pub fn is_pending(&self, id: u64) -> bool {
        self.shared
            .lock()
            .map(|s| s.pending.contains(&id))
            .unwrap_or(false)
    }
}

/// One message from the server: the answer to `initialize`, to a completion
/// request, or a request of its own (progress, configuration, registering a
/// capability) which is answered with null so the server does not wait on us.
fn handle(stdin: &Mutex<ChildStdin>, shared: &Mutex<Shared>, init_id: u64, msg: Value) {
    let id = msg.get("id").and_then(Value::as_u64);
    if msg.get("method").is_some() {
        if let Some(id) = msg.get("id") {
            write(stdin, &json!({"jsonrpc": "2.0", "id": id, "result": null}));
        }
        return;
    }
    let Some(id) = id else { return };
    if id == init_id {
        write(
            stdin,
            &json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
        );
        let queued = {
            let Ok(mut s) = shared.lock() else { return };
            s.ready = true;
            std::mem::take(&mut s.queue)
        };
        for m in queued {
            write(stdin, &m);
        }
        return;
    }
    if let Ok(mut s) = shared.lock() {
        if s.pending.remove(&id) {
            let items = msg.get("result").map(parse_items).unwrap_or_default();
            s.results.insert(id, items);
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.lock().ok().and_then(|mut c| c.take()) {
            // A polite goodbye first; the kill is what actually ends it. The
            // wait is on a thread, so closing a tab never waits on a server.
            write(
                &self.stdin,
                &json!({"jsonrpc": "2.0", "id": 9_999_999, "method": "shutdown"}),
            );
            write(&self.stdin, &json!({"jsonrpc": "2.0", "method": "exit"}));
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

/// The running clients, weakly: a client lives as long as some editor holds
/// its `Arc`, so the last tab of a language closing stops its server.
type Registry = Mutex<HashMap<(String, String), Weak<Client>>>;

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// The running client for `language` in `root`, started if there is none.
pub fn client_for(language: &str, root: &str, command: &str) -> Result<Arc<Client>, String> {
    let key = (language.to_string(), root.to_string());
    let mut reg = registry().lock().map_err(|_| "lsp registry poisoned")?;
    if let Some(c) = reg
        .get(&key)
        .and_then(Weak::upgrade)
        .filter(|c| !c.is_dead())
    {
        return Ok(c);
    }
    let client = Client::start(command, root)?;
    reg.insert(key, Arc::downgrade(&client));
    reg.retain(|_, w| w.strong_count() > 0);
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_framed_and_read_back_even_with_unicode_and_a_second_one_behind() {
        let a = json!({"jsonrpc": "2.0", "method": "x", "params": {"s": "héllo 🙂"}});
        let b = json!({"id": 2});
        let mut wire = encode(&a);
        wire.extend(encode(&b));
        let mut r = std::io::BufReader::new(&wire[..]);
        assert_eq!(read_message(&mut r).unwrap(), Some(a));
        assert_eq!(read_message(&mut r).unwrap(), Some(b));
        assert_eq!(read_message(&mut r).unwrap(), None);
        // A header with extra fields, and no length at all.
        let wire = b"Content-Type: x\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(
            read_message(&mut std::io::BufReader::new(&wire[..])).unwrap(),
            Some(json!({}))
        );
        assert!(read_message(&mut std::io::BufReader::new(&b"X: 1\r\n\r\n"[..])).is_err());
    }

    #[test]
    fn uris_and_columns_follow_the_protocol() {
        assert_eq!(file_uri("/a b/é.rs"), "file:///a%20b/%C3%A9.rs");
        // 🙂 is two UTF-16 units; "é" one.
        assert_eq!(utf16_col("é🙂x", 3), 4);
        assert_eq!(utf16_col("abc", 99), 3);
    }

    #[test]
    fn a_snippet_is_flattened_to_the_text_a_plain_insert_needs() {
        assert_eq!(flatten_snippet("foo(${1:a}, $2)$0"), "foo(a, )");
        assert_eq!(flatten_snippet("x${1|up,down|}"), "xup");
        assert_eq!(flatten_snippet("a \\$ b"), "a $ b");
        assert_eq!(flatten_snippet("${1:${2:inner}}"), "inner");
        assert_eq!(flatten_snippet("cost $"), "cost $");
    }

    #[test]
    fn items_are_read_from_a_list_or_an_object_and_sorted_by_the_servers_key() {
        let result = json!({"items": [
            {"label": "zeta", "sortText": "1"},
            {"label": "alpha", "detail": "fn()", "documentation": {"value": "Does it."},
             "insertText": "alpha($0)", "insertTextFormat": 2, "sortText": "0"},
            {"label": "edit", "textEdit": {"newText": "edited", "range": {}}, "sortText": "0b"},
            {"nolabel": true},
        ]});
        let items = parse_items(&result);
        let got: Vec<_> = items
            .iter()
            .map(|i| (i.label.as_str(), i.insert.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![("alpha", "alpha()"), ("edit", "edited"), ("zeta", "zeta")]
        );
        assert_eq!(items[0].detail, "fn()");
        assert_eq!(items[0].doc, "Does it.");
        assert_eq!(parse_items(&json!([{"label": "x"}])).len(), 1);
        assert!(parse_items(&json!(null)).is_empty());
    }

    #[test]
    fn the_project_root_is_the_nearest_folder_with_a_marker() {
        let root = std::env::temp_dir().join(format!("ift-lsp-root-{}", std::process::id()));
        std::fs::create_dir_all(root.join("crates/a/src")).unwrap();
        std::fs::write(root.join("crates/a/Cargo.toml"), "").unwrap();
        let file = root
            .join("crates/a/src/lib.rs")
            .to_string_lossy()
            .into_owned();
        assert_eq!(project_root(&file), root.join("crates/a").to_string_lossy());
        let loose = root.join("loose/x.rs").to_string_lossy().into_owned();
        std::fs::create_dir_all(root.join("loose")).unwrap();
        // No marker above it inside the temp tree: its own folder (or a
        // marker further up the real tree, which is also a folder).
        assert!(project_root(&loose).starts_with('/'));
        let _ = std::fs::remove_dir_all(&root);
    }

    // The whole client against a scripted server: the handshake, the queue
    // that holds the document until `initialized`, a completion answered
    // after a server-initiated request, and the server dying with the client.
    #[cfg(unix)]
    #[test]
    fn the_client_speaks_to_a_scripted_server() {
        use std::os::unix::fs::PermissionsExt;
        let Some(python) = [
            "/usr/bin/python3",
            "/opt/homebrew/bin/python3",
            "/usr/local/bin/python3",
        ]
        .iter()
        .find(|p| std::path::Path::new(p).is_file()) else {
            return; // no python here: the framing tests above still ran
        };
        let dir = std::env::temp_dir().join(format!("ift-lsp-fake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("server.py");
        std::fs::write(
            &script,
            r#"#!/usr/bin/env python3
import sys, json
def read():
    n = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line: return None
        line = line.decode().strip()
        if not line: break
        if line.startswith("Content-Length:"): n = int(line.split(":")[1])
    return json.loads(sys.stdin.buffer.read(n))
def send(m):
    b = json.dumps(m).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(b) + b); sys.stdout.buffer.flush()
text = {}
while True:
    m = read()
    if m is None: break
    meth = m.get("method")
    if meth == "initialize":
        send({"jsonrpc": "2.0", "id": m["id"], "result": {"capabilities": {"completionProvider": {}}}})
    elif meth == "textDocument/didOpen":
        d = m["params"]["textDocument"]; text[d["uri"]] = d["text"]
    elif meth == "textDocument/didChange":
        text[m["params"]["textDocument"]["uri"]] = m["params"]["contentChanges"][0]["text"]
    elif meth == "textDocument/completion":
        send({"jsonrpc": "2.0", "id": 77, "method": "window/workDoneProgress/create", "params": {}})
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": m["id"], "result": {"items": [
            {"label": "seen:" + text[uri].strip(), "insertText": "ins($0)", "insertTextFormat": 2}]}})
    elif meth == "exit": break
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let command = format!("{python} {}", script.display());
        let client = Client::start(&command, &dir.to_string_lossy()).expect("starts");
        let uri = file_uri(&dir.join("a.rs").to_string_lossy());
        // Sent before the server has answered `initialize`: queued, then sent.
        client.sync(&uri, "rust", 1, "first");
        client.sync(&uri, "rust", 2, "second");
        let id = client.request_completion(&uri, 0, 3);
        let mut got = None;
        for _ in 0..200 {
            if let Some(items) = client.take_completion(id) {
                got = Some(items);
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let items = got.expect("the server answered");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].label, "seen:second",
            "the change reached it before the request"
        );
        assert_eq!(items[0].insert, "ins()");
        assert!(client.is_ready() && !client.is_dead());
        assert!(!client.is_pending(id));
        client.close(&uri);
        // The registry hands the same client back while someone holds it.
        let root = dir.to_string_lossy().into_owned();
        let a = client_for("fake", &root, &command).unwrap();
        let b = client_for("fake", &root, &command).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        drop((a, b, client));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_server_is_an_error_not_a_panic() {
        let err = Client::start("definitely-not-a-server-xyz --stdio", "/tmp")
            .err()
            .expect("fails");
        assert!(err.contains("not found"), "{err}");
        assert_eq!(default_server("markdown"), None);
        assert!(default_server("rust").is_some());
        assert_eq!(language_id("tsx"), "typescriptreact");
    }
}
