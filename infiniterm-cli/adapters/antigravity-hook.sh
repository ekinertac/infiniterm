#!/bin/sh
# infiniterm's Antigravity CLI adapter (`agy`).
# Installed by `ift install-antigravity-hooks` as
# ~/.gemini/hooks/infiniterm-antigravity-hook.sh with the infiniterm-hook path
# filled in below, and wired in ~/.gemini/config/hooks.json as
# `infiniterm-antigravity-hook.sh <Event>`. Do not edit in place; re-run install.
#
# Antigravity hook research (antigravity.google/docs/hooks, and agy 1.3.2 run
# with a logging hook in a scratch workspace, 2026-10-11):
#
# Events: PreInvocation, PostInvocation, PreToolUse, PostToolUse, Stop. The
# event NAME is not in the stdin JSON, so the command carries it as $1. Every
# event gets `conversationId`, `workspacePaths`, `transcriptPath`, `modelName`.
#
# Turn started — `PreInvocation` fires before each model call (`invocationNum`
# counts them), so a turn sends it several times; each one maps to
# UserPromptSubmit, which only keeps the card working.
#
# Tool running — `PreToolUse` and `PostToolUse` carry `toolCall.name`.
#
# Turn finished — `Stop` carries `fullyIdle` (true when nothing is left to do),
# `terminationReason` (`NO_TOOL_CALL` after a plain answer) and `error`. Stop with
# fullyIdle false is the loop between steps and is ignored. A non-empty `error`,
# or a reason that names an error, is StopFailure; otherwise Stop.
#
# Permission prompt — there is no event for one. `PreToolUse` runs BEFORE the
# approval UI and its hook has exited when the user decides, so infiniterm
# cannot show yellow "waiting" for Antigravity (the Cursor and Codex gap).
#
# Output — this adapter prints NOTHING. PreToolUse documents a required
# `decision`, but an empty answer was checked to leave the permission flow as it
# was (the tool still reached the approval step); printing "allow" could
# bypass the user's prompts, which is not this adapter's business.
#
# Resume — `agy --conversation <id>`; `conversationId` is the session id.

HOOK="__INFINITERM_HOOK__"

command -v python3 >/dev/null 2>&1 || exit 0

# The heredoc below is python's stdin (the script itself), so the JSON the
# agent pipes in is read here and handed over in the environment. Reading it
# from inside python got nothing, and no event ever reached the app.
IFT_HOOK_INPUT=$(cat)
export IFT_HOOK_INPUT

exec python3 - "$HOOK" "$1" <<'PY'
import json
import os
import subprocess
import sys

hook_bin, name = sys.argv[1], sys.argv[2]


def main() -> None:
    try:
        raw = os.environ.get("IFT_HOOK_INPUT", "")
        inp = json.loads(raw) if raw.strip() else {}
    except Exception:
        return
    if not os.environ.get("INFINITERM_CARD_ID"):
        return
    event = map_event(name, inp)
    if not event:
        return
    paths = inp.get("workspacePaths") or []
    payload = {
        "hook_event_name": event,
        "session_id": inp.get("conversationId"),
        "cwd": paths[0] if paths else None,
    }
    tool = (inp.get("toolCall") or {}).get("name")
    if tool:
        payload["tool_name"] = tool
    try:
        subprocess.run(
            [hook_bin, event, "antigravity"],
            input=json.dumps(payload),
            text=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=0.5,
            check=False,
        )
    except Exception:
        pass


def map_event(name: str, inp: dict):
    if name == "PreInvocation":
        return "UserPromptSubmit"
    if name == "PreToolUse":
        return "PreToolUse"
    if name == "PostToolUse":
        return "PostToolUse"
    if name == "Stop":
        if not inp.get("fullyIdle"):
            return None
        reason = str(inp.get("terminationReason") or "").lower()
        if inp.get("error") or "error" in reason:
            return "StopFailure"
        return "Stop"
    return None


main()
PY
