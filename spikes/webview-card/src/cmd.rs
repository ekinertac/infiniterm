// The spike's control language: one command per line, read from a file the
// spike polls, so every test runs with no keyboard or mouse input (Ekin may
// be at the Mac). Pure parsing with tests; main.rs carries the commands out.
// Related: main.rs (the app), docs/agent/webview-spike.md (the scenarios).

#[derive(Debug, PartialEq)]
pub enum Cmd {
    /// `web <id> <x> <y> <w> <h> <url>`: a webview card at a world rect.
    Web { id: String, rect: [f32; 4], url: String },
    /// `plain <id> <x> <y> <w> <h> <rrggbb>`: a gpui-drawn card, the stand-in
    /// for a terminal that may overlap a browser card.
    Plain { id: String, rect: [f32; 4], color: u32 },
    Move { id: String, x: f32, y: f32 },
    Size { id: String, w: f32, h: f32 },
    Zoom(f32),
    /// `anim <ms>`: oscillate the zoom between 0.5 and 1.5 (two cycles) and
    /// log what the native sync cost per step.
    Anim(u64),
    Pan(f32, f32),
    /// `zoommode page|bounds|mag`: how a webview follows the canvas zoom.
    ZoomMode(String),
    /// `cover hole|snap|hide|none`: what a covered or dragged card does.
    Cover(String),
    Dim { id: String, alpha: f32 },
    Front(String),
    Close(String),
    Eval { id: String, js: String },
    Snap(String),
    /// `hit <x> <y>`: which AppKit view a click at this window point reaches.
    Hit(f32, f32),
    /// `click <x> <y>`: a synthetic left click sent to this app's window only
    /// (not through the HID tap), to see which view takes it.
    Click(f32, f32),
    /// `key <id> <chars> <mods>`: a synthetic key event sent to the window.
    Key { id: String, chars: String, cmd: bool },
    Focus(String),
    Url { id: String, url: String },
    Devtools(String),
    /// `inactivedim on|off`: dim every card but the selected one (0.8).
    InactiveDim(bool),
    /// `pinch <magnification>`: the same code path a trackpad pinch takes.
    Pinch(f32),
    /// `select <id>`: make a card the focused one, as a click on it does.
    Select(String),
    /// `find <id> <text>`: WKWebView's own find in page.
    Find { id: String, text: String },
    Windows,
    Info,
    Quit,
}

pub fn parse(line: &str) -> Result<Cmd, String> {
    let line = line.trim();
    let mut it = line.splitn(2, ' ');
    let verb = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    let f = |s: &str| s.parse::<f32>().map_err(|_| format!("not a number: {s}"));
    let words: Vec<&str> = rest.split_whitespace().collect();
    let need = |n: usize| {
        if words.len() >= n {
            Ok(())
        } else {
            Err(format!("{verb} needs {n} arguments"))
        }
    };
    match verb {
        "web" | "plain" => {
            need(6)?;
            let rect = [f(words[1])?, f(words[2])?, f(words[3])?, f(words[4])?];
            let id = words[0].to_string();
            if verb == "web" {
                let url = rest.splitn(6, ' ').nth(5).unwrap_or("").to_string();
                Ok(Cmd::Web { id, rect, url })
            } else {
                let color = u32::from_str_radix(words[5].trim_start_matches('#'), 16)
                    .map_err(|_| format!("not a colour: {}", words[5]))?;
                Ok(Cmd::Plain { id, rect, color })
            }
        }
        "move" => {
            need(3)?;
            Ok(Cmd::Move { id: words[0].into(), x: f(words[1])?, y: f(words[2])? })
        }
        "size" => {
            need(3)?;
            Ok(Cmd::Size { id: words[0].into(), w: f(words[1])?, h: f(words[2])? })
        }
        "zoom" => {
            need(1)?;
            Ok(Cmd::Zoom(f(words[0])?))
        }
        "anim" => {
            need(1)?;
            Ok(Cmd::Anim(words[0].parse().map_err(|_| format!("not a number: {}", words[0]))?))
        }
        "pan" => {
            need(2)?;
            Ok(Cmd::Pan(f(words[0])?, f(words[1])?))
        }
        "zoommode" => {
            need(1)?;
            Ok(Cmd::ZoomMode(words[0].into()))
        }
        "cover" => {
            need(1)?;
            Ok(Cmd::Cover(words[0].into()))
        }
        "dim" => {
            need(2)?;
            Ok(Cmd::Dim { id: words[0].into(), alpha: f(words[1])? })
        }
        "front" => {
            need(1)?;
            Ok(Cmd::Front(words[0].into()))
        }
        "close" => {
            need(1)?;
            Ok(Cmd::Close(words[0].into()))
        }
        "eval" => {
            need(2)?;
            let js = rest.splitn(2, ' ').nth(1).unwrap_or("").to_string();
            Ok(Cmd::Eval { id: words[0].into(), js })
        }
        "snap" => {
            need(1)?;
            Ok(Cmd::Snap(words[0].into()))
        }
        "hit" => {
            need(2)?;
            Ok(Cmd::Hit(f(words[0])?, f(words[1])?))
        }
        "click" => {
            need(2)?;
            Ok(Cmd::Click(f(words[0])?, f(words[1])?))
        }
        "key" => {
            need(2)?;
            Ok(Cmd::Key {
                id: words[0].into(),
                chars: words[1].into(),
                cmd: words.get(2) == Some(&"cmd"),
            })
        }
        "focus" => {
            need(1)?;
            Ok(Cmd::Focus(words[0].into()))
        }
        "url" => {
            need(2)?;
            Ok(Cmd::Url { id: words[0].into(), url: rest.splitn(2, ' ').nth(1).unwrap_or("").into() })
        }
        "devtools" => {
            need(1)?;
            Ok(Cmd::Devtools(words[0].into()))
        }
        "inactivedim" => {
            need(1)?;
            Ok(Cmd::InactiveDim(words[0] == "on"))
        }
        "pinch" => {
            need(1)?;
            Ok(Cmd::Pinch(f(words[0])?))
        }
        "select" => {
            need(1)?;
            Ok(Cmd::Select(words[0].into()))
        }
        "find" => {
            need(2)?;
            Ok(Cmd::Find { id: words[0].into(), text: rest.splitn(2, ' ').nth(1).unwrap_or("").into() })
        }
        "windows" => Ok(Cmd::Windows),
        "info" => Ok(Cmd::Info),
        "quit" => Ok(Cmd::Quit),
        "" => Err("empty".into()),
        other => Err(format!("unknown command {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_takes_the_whole_url() {
        assert_eq!(
            parse("web a 10 20 300 200 https://x.test/a b").unwrap(),
            Cmd::Web { id: "a".into(), rect: [10.0, 20.0, 300.0, 200.0], url: "https://x.test/a b".into() }
        );
    }

    #[test]
    fn plain_reads_a_hex_colour() {
        assert_eq!(
            parse("plain b 0 0 5 5 #ff8800").unwrap(),
            Cmd::Plain { id: "b".into(), rect: [0.0, 0.0, 5.0, 5.0], color: 0xff8800 }
        );
    }

    #[test]
    fn eval_keeps_the_script_as_one_string() {
        assert_eq!(
            parse("eval a document.title + ' x'").unwrap(),
            Cmd::Eval { id: "a".into(), js: "document.title + ' x'".into() }
        );
    }

    #[test]
    fn gesture_commands_parse() {
        assert_eq!(parse("inactivedim on").unwrap(), Cmd::InactiveDim(true));
        assert_eq!(parse("pinch 0.2").unwrap(), Cmd::Pinch(0.2));
        assert_eq!(parse("select a").unwrap(), Cmd::Select("a".into()));
    }

    #[test]
    fn click_takes_two_numbers() {
        assert_eq!(parse("click 3 4.5").unwrap(), Cmd::Click(3.0, 4.5));
    }

    #[test]
    fn errors_name_the_problem() {
        assert!(parse("zoom").unwrap_err().contains("needs 1"));
        assert!(parse("zoom x").unwrap_err().contains("not a number"));
        assert!(parse("nope").unwrap_err().contains("unknown"));
        assert!(parse("").is_err());
    }
}
