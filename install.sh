#!/bin/sh
# infiniterm installer: the one-line install the docs site serves at
# /install.sh (site/public/, copied verbatim into the build).
#
#   curl -fsSL https://infiniterm.app/install.sh | sh
#
# Downloads the latest release zip (the same file the in-app updater takes),
# checks it the way the updater does (codesign against the team requirement
# in infiniterm-core/src/update.rs, then spctl for notarization), copies it
# into /Applications, runs `ift install`, and wires agent hooks.
#
# Runs to the end without a person: agents install things now. Hooks edit
# another tool's config, so they are asked about only when a terminal is
# there to answer (/dev/tty; stdin is the pipe); with nobody there they are
# skipped and the commands printed, unless --hooks says which.
#
#   sh -s -- --hooks claude,pi     wire these, no questions (claude, pi,
#                                  codex, opencode)
#   sh -s -- --no-hooks            wire none, no questions
#
# An app already in /Applications is left alone: its own updater replaces
# it, and quitting it here would pull the canvas out from under whoever ran
# this from inside a card. INFINITERM_APPS_DIR and INFINITERM_ZIP exist for
# tools/test-install.sh. Exit codes: 0 ok, 1 install failed, 2 bad usage.
#
# Linux (#464): the release tarball for this architecture, named with its
# sha256 in latest-linux.json, unpacked into
# ~/.local/share/infiniterm/<version>/ with `current` pointing at it,
# `infiniterm` and `ift` linked in ~/.local/bin and a desktop entry written.
# Running it again IS the update: the new version goes in beside the old,
# `current` moves, and only the two newest versions are kept (a running
# iftd keeps working after its file is removed; the next app adopts it).
# INFINITERM_TARBALL and INFINITERM_LINUX_ROOT exist for
# tools/drive/linux/install-test.sh.
set -eu

REPO="ekinertac/infiniterm"
LATEST="https://github.com/$REPO/releases/latest/download"
TEAM='anchor apple generic and certificate leaf[subject.OU] = "QKN7RYV5PD"'
APPS="${INFINITERM_APPS_DIR:-/Applications}"
APP="$APPS/infiniterm.app"

say() { printf '%s\n' "$*"; }
die() { printf 'infiniterm install: %s\n' "$*" >&2; exit 1; }

hooks="ask"
while [ $# -gt 0 ]; do
    case "$1" in
        --hooks) [ $# -ge 2 ] || { say "--hooks takes a list: claude,pi,codex,opencode,cursor,antigravity" >&2; exit 2; }; hooks="$2"; shift ;;
        --hooks=*) hooks="${1#--hooks=}" ;;
        --no-hooks) hooks="" ;;
        -h|--help) say "usage: install.sh [--hooks claude,pi,codex,opencode,cursor,antigravity | --no-hooks]"; exit 0 ;;
        *) say "unknown option: $1 (try --hooks claude,pi or --no-hooks)" >&2; exit 2 ;;
    esac
    shift
done
case ",$hooks," in
    ,ask,|,,) ;;
    *) for h in $(printf '%s' "$hooks" | tr ',' ' '); do
           case "$h" in claude|pi|codex|opencode|cursor|antigravity) ;; *) say "unknown hook: $h (claude, pi, codex, opencode, cursor or antigravity)" >&2; exit 2 ;; esac
       done ;;
esac

# The Linux install: the tarball into a versioned folder, `current` moved to
# it. Sets IFT. See the header.
install_linux() {
    case "$(uname -m)" in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) die "there is no Linux build for $(uname -m)" ;;
    esac
    root="${INFINITERM_LINUX_ROOT:-$HOME/.local/share/infiniterm}"
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    if [ -n "${INFINITERM_TARBALL:-}" ]; then
        cp "$INFINITERM_TARBALL" "$tmp/infiniterm.tar.gz"
    else
        # Flat keys, one pair per architecture: "x86_64_url", "x86_64_sha256".
        manifest=$(curl -fsSL "$LATEST/latest-linux.json") || die "could not fetch latest-linux.json from $REPO"
        field() { printf '%s\n' "$manifest" | sed -n "s/^ *\"$1\": *\"\([^\"]*\)\".*/\1/p" | head -n 1; }
        url=$(field "${arch}_url")
        sha=$(field "${arch}_sha256")
        [ -n "$url" ] && [ -n "$sha" ] || die "latest-linux.json has no build for $arch"
        say "Downloading ${url##*/}"
        curl -fSL --progress-bar -o "$tmp/infiniterm.tar.gz" "$url" || die "download failed"
        [ "$(sha256sum "$tmp/infiniterm.tar.gz" | cut -d' ' -f1)" = "$sha" ] || die "checksum mismatch; not installing"
    fi
    mkdir -p "$tmp/x"
    tar -xzf "$tmp/infiniterm.tar.gz" -C "$tmp/x" || die "could not unpack the tarball"
    [ -f "$tmp/x/infiniterm/VERSION" ] && [ -x "$tmp/x/infiniterm/infiniterm" ] \
        || die "the tarball holds no infiniterm build"
    version=$(cat "$tmp/x/infiniterm/VERSION")
    case "$version" in '' | *[!0-9A-Za-z.+-]*) die "the tarball's VERSION is not a version: $version" ;; esac
    mkdir -p "$root"
    # The same version again (a repair) replaces the folder; the files a
    # running copy has open stay readable until it exits.
    rm -rf "$root/$version.new"
    mv "$tmp/x/infiniterm" "$root/$version.new"
    rm -rf "$root/$version"
    mv "$root/$version.new" "$root/$version"
    ln -sfn "$version" "$root/current"
    mkdir -p "$HOME/.local/bin"
    ln -sfn "$root/current/infiniterm" "$HOME/.local/bin/infiniterm"
    apps="$HOME/.local/share/applications"
    mkdir -p "$apps"
    cat > "$apps/infiniterm.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=infiniterm
Comment=Terminals, editors and agents on one canvas
Exec=$root/current/infiniterm
Icon=$root/current/icon.png
Terminal=false
Categories=System;TerminalEmulator;Development;
StartupWMClass=infiniterm
EOF
    # The two newest versions stay, the one `current` names among them.
    old=$(ls -1 "$root" | grep -v -e '^current$' -e '\.new$' | sort -V | head -n -2)
    for v in $old; do
        [ "$v" = "$version" ] || rm -rf "${root:?}/$v"
    done
    say "Installed infiniterm $version in $root/$version"
    IFT="$root/current/ift"
}

install_mac() {
[ "$(uname -m)" = arm64 ] || die "builds are Apple Silicon only for now"

if [ -d "$APP" ]; then
    say "infiniterm is already in $APPS; it updates itself, so it is left as it is."
else
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    if [ -n "${INFINITERM_ZIP:-}" ]; then
        cp "$INFINITERM_ZIP" "$tmp/app.zip"
    else
        # latest.json names the zip (its name carries the build number) and
        # its sha256, the manifest the updater reads.
        manifest=$(curl -fsSL "$LATEST/latest.json") || die "could not fetch latest.json from $REPO"
        field() { printf '%s\n' "$manifest" | sed -n "s/^ *\"$1\": *\"\([^\"]*\)\".*/\1/p" | head -n 1; }
        url=$(field url)
        sha=$(field sha256)
        [ -n "$url" ] && [ -n "$sha" ] || die "latest.json has no url or sha256"
        say "Downloading ${url##*/}"
        curl -fSL --progress-bar -o "$tmp/app.zip" "$url" || die "download failed"
        [ "$(shasum -a 256 "$tmp/app.zip" | cut -d' ' -f1)" = "$sha" ] || die "checksum mismatch; not installing"
    fi
    ditto -x -k "$tmp/app.zip" "$tmp/x" || die "could not unpack the zip"
    [ -d "$tmp/x/infiniterm.app" ] || die "the zip holds no infiniterm.app"
    codesign --verify --deep --strict -R="$TEAM" "$tmp/x/infiniterm.app" 2>/dev/null \
        || die "signature check failed; not installing"
    spctl -a -t exec "$tmp/x/infiniterm.app" 2>/dev/null \
        || die "Gatekeeper rejected the app (not notarized?); not installing"
    [ -w "$APPS" ] || die "$APPS is not writable by $(id -un)"
    ditto "$tmp/x/infiniterm.app" "$APP" || die "could not copy into $APPS"
    say "Installed $APP"
fi

IFT="$APP/Contents/MacOS/ift"
}

case "$(uname -s)" in
    Darwin) install_mac ;;
    Linux) install_linux ;;
    *) die "infiniterm runs on macOS and Linux" ;;
esac
"$IFT" install || die "ift install failed"

# An agent is offered only when the installed ift knows its installer: an
# app already in /Applications is left as it is, and an older build has
# fewer adapters (Codex and OpenCode came after 0.2.0).
supported() { "$IFT" --help 2>/dev/null | grep -q "install-$1-hooks"; }

# Which hooks, when nobody said.
if [ "$hooks" = ask ]; then
    hooks=""
    if (: </dev/tty) 2>/dev/null; then
        for h in claude pi codex opencode cursor antigravity; do
            case "$h" in
                claude) found=$(command -v claude || { [ -d "$HOME/.claude" ] && echo yes; } || true); what="Claude Code (edits ~/.claude/settings.json)" ;;
                pi) found=$(command -v pi || { [ -d "$HOME/.pi" ] && echo yes; } || true); what="Pi (adds an extension to ~/.pi/agent)" ;;
                codex) found=$(command -v codex || { [ -d "${CODEX_HOME:-$HOME/.codex}" ] && echo yes; } || true); what="Codex (edits ~/.codex/hooks.json; approve them once with /hooks in Codex)" ;;
                opencode) found=$(command -v opencode || { [ -d "${XDG_CONFIG_HOME:-$HOME/.config}/opencode" ] && echo yes; } || true); what="OpenCode (adds a plugin to ~/.config/opencode/plugins)" ;;
                cursor) found=$(command -v cursor || { [ -d "$HOME/.cursor" ] && echo yes; } || true); what="Cursor (edits ~/.cursor/hooks.json for the agent CLI)" ;;
                antigravity) found=$(command -v agy || { [ -d "$HOME/.gemini/antigravity-cli" ] && echo yes; } || true); what="Antigravity CLI (edits ~/.gemini/config/hooks.json; start a new agy session after)" ;;
            esac
            [ -n "$found" ] || continue
            supported "$h" || continue
            printf 'Show agent state from %s? [y/N] ' "$what" >/dev/tty
            read -r answer </dev/tty || answer=""
            case "$answer" in y|Y|yes) hooks="$hooks,$h" ;; esac
        done
    else
        say "No terminal to ask, so no agent hooks were installed. To add them:"
        for h in claude pi codex opencode cursor antigravity; do supported "$h" && say "  ift install-$h-hooks"; done
    fi
fi
for h in $(printf '%s' "$hooks" | tr ',' ' '); do
    supported "$h" || { say "This build has no ift install-$h-hooks yet; skipped. It comes with an update."; continue; }
    "$IFT" "install-$h-hooks" || die "ift install-$h-hooks failed"
done

say ""
if [ "$(uname -s)" = Linux ]; then
    say "Done. Start it from your applications menu, or with: infiniterm   (once ~/.local/bin is on your PATH)"
    say "To update, run this installer again."
else
    say "Done. Start it with: open -a infiniterm   (or \`ift\` once ~/.local/bin is on your PATH)"
fi
say "One more line for per-card shell history, in ~/.zshrc:"
say '  HISTFILE="${INFINITERM_HISTFILE:-$HOME/.zsh_history}"'
