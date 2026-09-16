# The omnibox

Cmd+L opens an address bar over the canvas: type a URL or a search, get ranked results from what you have open and where you have been, press Enter. Modelled on the omnibox in `~/Code/glass` (Electron), which already solved the ranking and the tab-to-search flow. This is that design in Rust, in this codebase's shape: pure logic in `infiniterm-core` with tests, the overlay as wiring in `infiniterm-ui`.

Not the command palette. The palette matches a fixed list of commands by fuzzy score; the omnibox takes free text, decides whether it is an address or a query, and mixes results from several sources. They share nothing but the fact that both are overlays.

## What it does

Cmd+L opens it. Two starting states:

- No browser card focused: the field is empty, the placeholder is `search or enter address`. Enter opens a new browser card in the free slot nearest the focused card, the same placement any new card gets.
- A browser card focused: the field holds that card's current URL, selected, so typing replaces it. Enter navigates that card in place. This is Chrome's behaviour and it is why the field is prefilled at all.

Escape closes it and changes nothing. A click outside does the same.

Inside the field: arrows move the selection, Enter opens the selected result, Tab enters a site search when one is offered, Backspace on an empty field leaves that site search. The typed substring is highlighted in the results, as the palette already does.

## Results

Sections, in this order, empty ones omitted:

1. **Best match**: the history entry whose scheme-stripped URL starts with what you typed, promoted out of its own section. This is also what drives the inline completion in the field.
2. **Search**: what you typed, read as an address if it parses as one and as a query otherwise.
3. **Suggestions**: Google's completions. Off by default, see Settings.
4. **History**: visited pages, ranked by visit count with recency as the tiebreak, so a page you open daily beats a redirect URL you saw once.
5. **Cards**: the browser cards already open on the canvas. Choosing one focuses it rather than navigating anywhere. In glass this is "switch to tab"; here it is a card, so it is `set_focus` plus `reveal_focused`.

A result carries an action: `Navigate(url)`, `Search(query)` or `FocusCard(id)`.

## Address or search

`parse_address` replaces `Model::normalise_url`, which only prepends `https://` to anything non-empty:

- an explicit scheme (`https://`, `file://`, `about:`) is kept
- `localhost`, `localhost:8087`, `127.0.0.1:3000` get `http://`, not `https://`, which is the bug that sent the screencast's browser card to a TLS error
- a bare host with a dot and no whitespace (`news.ycombinator.com/newest`) gets `https://`
- anything else is a search against the configured engine

The old function stays as a thin call into the new one until the prompt paths are gone.

## Tab to search

A static registry of engines keyed by host: Google, GitHub, YouTube, Wikipedia, Stack Overflow, MDN, npm, crates.io, docs.rs. Typing a prefix of an engine's name, host, or first host label offers `Tab to search GitHub`. Tab enters that scope: the field clears, a chip shows which site owns it, and every query goes to that engine's search URL. Backspace on an empty field or Escape leaves the scope; Escape from an unscoped field closes the omnibox.

Engines are a list in settings so a site can be added without a build. Deriving them from history or OpenSearch is a later slice.

## History

Its own store, `history.json` beside `workspace.json`, not Chromium's. The CEF profile does keep a real `History` SQLite in `<data>/browser/`, but reading it means a SQLite dependency and a lock fight with the browser process that owns it. We already see every navigation: `browsers.rs` receives an address change per frame and writes it to the card, and the page title arrives the same way.

One JSON array, at most 1000 entries, `{url, title, visit_count, visited_at}`. Deduped by URL; a revisit bumps the count and the timestamp. Written debounced through the same write-then-rename the save file uses. A missing or corrupt file starts empty, like every other file this app reads.

`about:blank` and any internal URL is never recorded.

Nothing here leaves the machine, and there is no history UI in this slice. Clearing it is deleting the file until there is a command for it.

## Suggestions

The one network call, and the only part of this that talks to anything but the page you asked for. Google's `suggestqueries` endpoint, the compact `client=firefox` shape.

Off unless `browser.suggestions` is true. The README's footprint section says the app reaches the network only through browser cards, and that stays true for anyone who does not turn this on. The setting's doc line says plainly that each keystroke goes to Google.

Fetched by shelling out to `curl --max-time 1.5`, the way this app already shells out to `git`, `ps`, `lsof` and `open`, rather than taking an HTTP client dependency for one endpoint. Failure, timeout and garbage all mean no suggestions and never an error.

The local sources must never wait for it. The engine runs in two phases, as glass's does: local providers answer immediately and the overlay draws; the network provider lands later and replaces the results for the same query id. A response for an older query id is dropped.

## Where the code goes

```
infiniterm-core/src/omni/
  mod.rs        OmniAction, OmniResult, OmniSection, the query context
  address.rs    parse_address, the search template
  engines.rs    the tab-to-search registry, offer and scope resolution
  providers.rs  search, history, cards; each a pure fn over the context
  rank.rs       sections in order, best-match promotion, inline completion
  history.rs    the frecency store: record, set_title, list
  suggest.rs    parsing the suggest response (pure); the fetch is an Effect
infiniterm-ui/src/omnibox.rs   the overlay: field, sections, chip, key handling
```

The model holds the omnibox state next to the palette's: query, selection, scope, the last response's query id. `Model::overlay_open()` gains it, so the wheel and the key path treat it like every other modal. `card.omnibox` is a registered command like everything else, bound to Cmd+L, so it reaches the palette and the shortcuts panel without being listed twice.

The network fetch is an `Effect::FetchSuggestions { query_id, q }`, answered by the backend thread and delivered as an event, which is how every other off-thread answer already reaches the model.

## Settings

- `browser.searchEngine`: the template, `https://www.google.com/search?q=%s` by default.
- `browser.suggestions`: false. Turning it on sends what you type to Google.
- `browser.engines`: the tab-to-search list, seeded with the nine above.

Each needs a `settings_doc.rs` entry or the test fails, which is the point of that test.

## ift

`ift omni <term>` prints the ranked sections for a term against the real history file, without the GUI, the way glass's `glass omni` does. It is how ranking gets inspected and how a regression gets a repro that is not a screenshot.

## Not in this slice

Bookmarks, and the three surfaces they want (Cmd+D, a bar, a popup). Engines derived from history or OpenSearch. A history window. Paste-and-go. Find in page.

## Tests

Every module above is a pure function with a test, which is the rule in this repo anyway:

- `parse_address`: scheme kept, localhost to http, bare host to https, query to search, empty to nothing
- `engines`: the prefix offer ("git" offers GitHub, "y" offers YouTube), scope resolution, the built URL
- `providers`: the history matcher ignores the query string, the card provider matches title and URL
- `rank`: section order, empty sections dropped, best match promoted out of history, completion only when the entry is not what was typed
- `history`: dedupe, count bump, the 1000 cap, a corrupt file reading as empty
- `suggest`: the response parser, including every malformed shape

The model gets a Harness test that Cmd+L on a browser card prefills and navigates in place, and that Cmd+L elsewhere makes a card.
