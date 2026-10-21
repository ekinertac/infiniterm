#!/bin/sh
# The landing page's canvas screenshot (site/public/canvas.png), taken on a
# clean scratch instance so it shows no real directories or card names.
#
# The scene is built with `ift` and the hook binary, not with keystrokes:
# `ift run card.new.terminal` makes a card, `ift send` types into it, `ift name` / `ift group`
# label it, and the hook binary sets agent states (no tokens spent). Plain
# shell commands colour their cards by themselves through the zsh
# integration: the long build goes violet, the failing test run red. The
# only input to the Mac is launching the app and one screencapture.
#
#   tools/drive/hero-shot.sh            build the scene and take the shot
#   KEEP_OPEN=1 tools/drive/hero-shot.sh  leave the instance running after
#
# Everything lives under $HERO_DIR; the real data dir is never touched.
set -e
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
HERO_DIR=${HERO_DIR:-/tmp/ift-hero}
export INFINITERM_DATA_DIR="$HERO_DIR/data"
export INFINITERM_CONFIG_DIR="$HERO_DIR/config"
APP="$ROOT/target/bundle/infiniterm.app"
IFT="$APP/Contents/MacOS/ift"
HOOK="$APP/Contents/MacOS/infiniterm-hook"
# The window the shot is taken at, in points; the capture is at the screen's
# scale, so 2x on a Retina display.
WIN_W=${WIN_W:-1600}
WIN_H=${WIN_H:-1000}

# Several instances of this bundle can run at once (one per HERO_DIR), so
# the one this script started is addressed by the pid it saved, never by name.
PIDFILE="$HERO_DIR.pid"
pid() { cat "$PIDFILE"; }
stop() { [ -f "$PIDFILE" ] && kill "$(pid)" 2>/dev/null; rm -f "$PIDFILE"; sleep 1; }

# --- the demo projects ------------------------------------------------------
demo() {
    # Stop a previous run first: quitting saves its canvas, which would land
    # in the fresh folder.
    stop
    rm -rf "$HERO_DIR"
    mkdir -p "$HERO_DIR/data" "$HERO_DIR/config"
    # Default settings: the cards are the size a new user gets.
    echo '{}' >"$HERO_DIR/config/settings.json"
    printf '{"x":120.0,"y":80.0,"w":%s.0,"h":%s.0,"mode":"windowed"}\n' "$WIN_W" "$WIN_H" >"$HERO_DIR/data/window.json"
    # A neutral zsh for every card: the app hands the shell the ZDOTDIR it
    # was launched with, so the user's own .zshrc (a fetch banner, their
    # name and host in the prompt) stays out of the shot.
    mkdir -p "$HERO_DIR/zdot"
    cat >"$HERO_DIR/zdot/.zshrc" <<'Z'
PROMPT='%F{8}%1~%f %F{5}❯%f '
Z
    # An existing (empty) canvas, so the launch seeds one terminal instead of
    # the first-run welcome card.
    echo '{}' >"$HERO_DIR/data/workspace.json"

    A="$HERO_DIR/api"; W="$HERO_DIR/web"; I="$HERO_DIR/infra"
    mkdir -p "$A/src" "$W/src" "$I" "$A/.demo" "$W/.demo"
    cat >"$A/src/auth.rs" <<'R'
use axum::http::{HeaderMap, StatusCode};

/// Checks the bearer token on every request to /api.
pub fn verify_token(headers: &HeaderMap) -> Result<Claims, StatusCode> {
    let header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or(StatusCode::UNAUTHORIZED)?;
    decode(token).map_err(|_| StatusCode::UNAUTHORIZED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_header_is_unauthorized() {
        assert_eq!(verify_token(&HeaderMap::new()), Err(StatusCode::UNAUTHORIZED));
    }
}
R
    cat >"$A/.demo/ask.txt" <<'T'
⏺ Update(src/auth.rs)
  ⎿  Added 4 lines

   12 +    if claims.expired() {
   13 +        return Err(StatusCode::UNAUTHORIZED);
   14 +    }

 Do you want to make this edit to auth.rs?
 ❯ 1. Yes
   2. Yes, allow all edits this session
   3. No, and tell Claude what to do differently
T
    cat >"$A/.demo/build.sh" <<'B'
#!/bin/sh
for c in serde_derive tokio hyper tower axum sqlx-core sqlx-postgres jsonwebtoken api; do
  printf '   \033[1;32mCompiling\033[0m %s v0.%s.%s\n' "$c" "$((RANDOM % 9))" "$((RANDOM % 20))"
  sleep 0.4
done
printf '    \033[1;36mBuilding\033[0m [=========>       ] 212/297\n'
sleep 3600
B
    cat >"$W/.demo/test.sh" <<'B'
#!/bin/sh
printf '\n \033[1;32m✓\033[0m src/price.test.ts (12)\n \033[1;32m✓\033[0m src/session.test.ts (9)\n \033[1;31m✗\033[0m src/cart.test.ts (14)\n'
printf '   \033[31m✗ applies the discount once\033[0m\n     expected 81 to be 90\n\n'
printf ' Tests  \033[1;31m1 failed\033[0m | \033[1;32m34 passed\033[0m (35)\n'
exit 1
B
    cat >"$W/.demo/done.txt" <<'T'
⏺ The cart now applies a discount once per order,
  and the two tests that covered the double
  discount pass.

  Changed src/cart.ts and src/cart.test.ts.

> █
T
    cat >"$I/.demo-log.txt" <<'T'
12:04:31 deploy api@3f2a91c to staging
12:04:33 pull ghcr.io/acme/api:3f2a91c
12:04:40 health /readyz 200 in 41ms
12:04:41 traffic 10% -> 3f2a91c
12:05:41 errors 0.02% p95 118ms
12:05:42 traffic 50% -> 3f2a91c
12:06:42 errors 0.01% p95 121ms
12:06:43 traffic 100% -> 3f2a91c
T
    chmod +x "$A/.demo/build.sh" "$W/.demo/test.sh"
    cat >"$W/src/cart.ts" <<'R'
export function total(items: Item[], discount?: Discount): number {
  const sum = items.reduce((s, i) => s + i.price * i.qty, 0);
  return discount ? applyOnce(sum, discount) : sum;
}
R
    for d in "$A" "$W"; do
        git -C "$d" init -q && git -C "$d" add -A && git -C "$d" -c user.name=demo -c user.email=demo@example.com commit -qm "first"
    done
    # A change for the diff card to show.
    cat >"$W/src/cart.ts" <<'R'
export function total(items: Item[], discount?: Discount): number {
  const sum = items.reduce((s, i) => s + i.price * i.qty, 0);
  if (!discount || discount.used) return sum;
  discount.used = true;
  return applyOnce(sum, discount);
}
R
}

# --- the app ----------------------------------------------------------------
launch() {
    before=$(pgrep -f "$APP/Contents/MacOS/infiniterm$" | sort)
    # -n: a second copy beside any instance already running.
    open -n --env ZDOTDIR="$HERO_DIR/zdot" --env INFINITERM_DATA_DIR="$INFINITERM_DATA_DIR" --env INFINITERM_CONFIG_DIR="$INFINITERM_CONFIG_DIR" "$APP"
    sleep 2
    pgrep -f "$APP/Contents/MacOS/infiniterm$" | sort | grep -vxF "$before" | head -1 >"$PIDFILE"
    i=0
    until "$IFT" ls >/dev/null 2>&1; do
        i=$((i + 1)); [ $i -gt 60 ] && { echo "the app did not answer" >&2; exit 1; }
        sleep 0.5
    done
    sleep 1.5
}

ids() { "$IFT" ls | cut -f1 | sort; }
# Opens a terminal card, moves its shell to $1, and prints its id. (`ift <dir>`
# opens an editor card with a tree, not a terminal.)
card() {
    before=$(ids)
    "$IFT" run card.new.terminal >/dev/null
    sleep 1
    id=$(ids | grep -vxF "$before" | head -1)
    sleep 1
    send "$id" "cd $1" --enter
    echo "$id"
}
label() { INFINITERM_CARD_ID=$1 "$IFT" name "$2" >/dev/null; [ -n "$3" ] && INFINITERM_CARD_ID=$1 "$IFT" group "$3" >/dev/null; true; }
# A new card's shell takes a moment to start; send retries until it is there.
send() {
    n=0
    until "$IFT" send "$@" >/dev/null 2>&1; do
        n=$((n + 1)); [ $n -gt 20 ] && { echo "card $1 never got a shell" >&2; exit 1; }
        sleep 0.3
    done
}
say() { send "$1" "$2" --enter; }
hook() { echo '{}' | INFINITERM_CARD_ID=$1 "$HOOK" "$2" ${3:-}; }

scene() {
    A="$HERO_DIR/api"; W="$HERO_DIR/web"
    # Six cards in creation order fill the block 3 wide and 2 high: the top
    # row api, the bottom row web. No groups: grouping a card moves it to a
    # free block, which scatters the layout. The seeded terminal is the first
    # card, so no hole is left where it was.
    c1=$(ids | head -1)
    send "$c1" "cd $A" --enter; label "$c1" "claude"
    say "$c1" "clear; cat .demo/ask.txt; sleep 3600"
    hook "$c1" UserPromptSubmit; sleep 0.3; hook "$c1" PermissionRequest
    c2=$(card "$A"); label "$c2" "build"
    say "$c2" "clear; .demo/build.sh"
    c3=$(card "$W"); label "$c3" "tests"
    say "$c3" "clear; .demo/test.sh"
    c4=$(card "$W"); label "$c4" "claude"
    say "$c4" "clear; cat .demo/done.txt; sleep 3600"
    hook "$c4" UserPromptSubmit; sleep 0.3; hook "$c4" Stop
    "$IFT" -n "$A/src/auth.rs" >/dev/null; sleep 1
    "$IFT" diff "$W" >/dev/null; sleep 1
    "$IFT" run canvas.zoom.fitAll >/dev/null
    # Long enough for the build to pass the 5 s working mark and the test
    # run to end red.
    sleep 6
}

# The close-up: the waiting card fitted (Cmd+1), reached with the arrows
# from the diff card in the corner, past the done card quickly enough that
# it stays unseen.
closeup() {
    "$IFT" run focus.move.left >/dev/null; sleep 0.2
    "$IFT" run focus.move.left >/dev/null; sleep 0.2
    "$IFT" run focus.move.up >/dev/null; sleep 0.3
    "$IFT" run canvas.zoom.fitCard >/dev/null
    sleep 2
}

shot() {
    osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $(pid)) to true" >/dev/null
    sleep 1
    wid=$("$ROOT/tools/winid" --pid "$(pid)" | head -1 | cut -f1)
    screencapture -x -o -l "$wid" "$1"
    echo "shot: $1"
}

demo
launch
scene
shot "$HERO_DIR/canvas.png"
closeup
shot "$HERO_DIR/closeup.png"
[ "${KEEP_OPEN:-}" = 1 ] || stop
