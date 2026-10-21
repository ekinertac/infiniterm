#!/usr/bin/env python3
"""Proves the CLAUDE.md split lost no text.

What it does: reads CLAUDE.md as it was on origin/master (before the split) and checks that
every non-empty line of it appears in the new CLAUDE.md or in a docs/agent/*.md file. A line
may have been split at sentence boundaries (one long paragraph of the join section became
several paragraphs in two files): those lines are cut at the listed anchors and each piece
is checked, with whitespace folded, so a newline in place of a space does not count as a
change. Lines that are not found are printed as the exceptions.

Where it fits: run once for the PR that split CLAUDE.md (2026-10-19); kept so a later
reader can see how "lossless" was shown. Run from the repo root:
    python3 docs/agent/check_lossless.py [base-ref]
Exit status 0 when nothing but the listed headings is missing.
"""
import glob
import subprocess
import sys

BASE = sys.argv[1] if len(sys.argv) > 1 else "origin/master"

# The join viewer paragraph is one line in the old file; it was split before each of these.
SPLIT_LINE_START = "- Join, the viewer side (#423, stage 2)"
ANCHORS = [
    " The viewer backend is the `viewer` flag",
    " A host with no window (#438",
    " `ift serve` (`infiniterm-cli/src/serve.rs`) is the process",
    " The app's takeover: before anything",
    " A host with NO canvas (no file",
    " `ift join` starts it (`remote::start_serve`",
    " The login step (#450",
    " Lost connections (#442)",
    " Settings and keybindings are the HOST's (#433)",
]

# Headings the split replaced on purpose (the new files have their own headings).
EXPECTED_MISSING = {
    "## Decisions taken in this codebase",
    "## Traps already hit",
}


def fold(text: str) -> str:
    return " ".join(text.split())


def main() -> int:
    old = subprocess.run(
        ["git", "show", f"{BASE}:CLAUDE.md"], capture_output=True, text=True, check=True
    ).stdout
    files = ["CLAUDE.md"] + sorted(glob.glob("docs/agent/*.md"))
    corpus_lines = set()
    corpus_all = []
    for path in files:
        with open(path, encoding="utf-8") as handle:
            body = handle.read()
        corpus_all.append(body)
        for line in body.split("\n"):
            if line.strip():
                corpus_lines.add(line.strip())
    folded_corpus = fold("\n".join(corpus_all))

    checked = exact = split = 0
    missing = []
    for line in old.split("\n"):
        if not line.strip():
            continue
        checked += 1
        if line.strip() in corpus_lines:
            exact += 1
            continue
        pieces = [line]
        if line.startswith(SPLIT_LINE_START):
            pieces = []
            rest = line
            for anchor in ANCHORS:
                head, sep, tail = rest.partition(anchor)
                if not sep:
                    pieces = [line]
                    break
                pieces.append(head)
                rest = anchor + tail
            else:
                pieces.append(rest)
        if all(fold(piece) in folded_corpus for piece in pieces):
            split += 1
            continue
        missing.append(line)

    print(f"base {BASE}: {checked} non-empty lines checked in {len(files)} files")
    print(f"  found as whole lines: {exact}")
    print(f"  found as pieces of a split line: {split}")
    unexpected = [m for m in missing if m.strip() not in EXPECTED_MISSING]
    for m in missing:
        tag = "expected" if m.strip() in EXPECTED_MISSING else "MISSING"
        print(f"  {tag}: {m[:110]}")
    old_bytes = len(old.encode("utf-8"))
    new_bytes = len(open("CLAUDE.md", encoding="utf-8").read().encode("utf-8"))
    print(f"CLAUDE.md: {old_bytes} bytes before, {new_bytes} bytes after")
    for path in files[1:]:
        size = len(open(path, encoding="utf-8").read().encode("utf-8"))
        print(f"  {path}: {size} bytes")
    return 1 if unexpected else 0


if __name__ == "__main__":
    sys.exit(main())
