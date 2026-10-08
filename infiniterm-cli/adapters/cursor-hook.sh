#!/bin/sh
# infiniterm's Cursor adapter (Cursor Agent CLI and IDE agent hooks).
# Installed by `ift install-cursor-hooks` as ~/.cursor/hooks/infiniterm-cursor-hook.sh
# with the infiniterm-hook path filled in below. Do not edit in place; re-run install.
#
# Cursor hook research (cursor.com/docs/hooks, checked 2026-10-09):
#
# Prompt submitted — `beforeSubmitPrompt` fires after Enter, before the model
# request. stdin JSON includes `prompt`, `attachments`, and the common fields
# (`conversation_id`, `session_id`, `hook_event_name`, `transcript_path`, …).
# Maps to infiniterm `UserPromptSubmit`.
#
# Tool running — `preToolUse` before any tool (Shell, Read, Write, MCP, Task, …)
# and `postToolUse` after success. stdin carries `tool_name`, `tool_input`,
# `tool_use_id`, `cwd`. Maps to `PreToolUse` / `PostToolUse`.
#
# Permission prompt — there is no hook that fires while the approval UI is open.
# `beforeShellExecution` / `beforeMCPExecution` run once before execution and may
# return `"permission": "ask"`, but the hook process has already exited when the
# user decides, so infiniterm cannot show yellow "waiting" for Cursor approvals
# (same class of gap as Codex questions). `postToolUseFailure` with
# `failure_type: "permission_denied"` is after a denial, not while waiting.
#
# Failed turn — `postToolUseFailure` when a tool errors or times out (skip when
# `is_interrupt` is true, user cancelled the tool). `stop` with `status: "error"`
# when the agent loop ends in error. Both map to `StopFailure`. `stop` with
# `status: "completed"` maps to `Stop`; `"aborted"` maps to `Stop` (user stopped).
#
# Also wired: `sessionStart` / `sessionEnd` → SessionStart / SessionEnd.
# Session id for resume: `conversation_id` or `session_id` in the payload
# (`cursor agent --resume <id>`).

HOOK="__INFINITERM_HOOK__"

command -v python3 >/dev/null 2>&1 || exit 0

exec python3 - "$HOOK" <<'PY'
import json
import os
import subprocess
import sys
from typing import Optional

hook_bin = sys.argv[1]

def main() -> None:
    try:
        raw = sys.stdin.read()
        inp = json.loads(raw) if raw.strip() else {}
    except Exception:
        inp = {}

    card = os.environ.get("INFINITERM_CARD_ID")
    if not card:
        emit_cursor_output(inp)
        return

    event = map_event(inp)
    if not event:
        emit_cursor_output(inp)
        return

    session = (
        inp.get("conversation_id")
        or inp.get("session_id")
        or inp.get("sessionId")
        or inp.get("conversationId")
    )
    payload = {
        "hook_event_name": event,
        "session_id": session,
        "cwd": inp.get("cwd"),
        "transcript_path": inp.get("transcript_path"),
    }
    tool = inp.get("tool_name")
    if tool:
        payload["tool_name"] = tool

    try:
        subprocess.run(
            [hook_bin, event, "cursor"],
            input=json.dumps(payload),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=0.5,
            check=False,
        )
    except Exception:
        pass

    emit_cursor_output(inp)


def map_event(inp: dict) -> Optional[str]:
    name = inp.get("hook_event_name") or inp.get("hookEventName")
    if not name:
        return None

    if name == "beforeSubmitPrompt":
        return "UserPromptSubmit"
    if name == "preToolUse":
        return "PreToolUse"
    if name == "postToolUse":
        return "PostToolUse"
    if name == "postToolUseFailure":
        if inp.get("is_interrupt"):
            return None
        return "StopFailure"
    if name == "stop":
        status = inp.get("status")
        if status == "error":
            return "StopFailure"
        if status in ("completed", "aborted"):
            return "Stop"
        return None
    if name == "sessionStart":
        return "SessionStart"
    if name == "sessionEnd":
        return "SessionEnd"
    return None


def emit_cursor_output(inp: dict) -> None:
    name = inp.get("hook_event_name") or inp.get("hookEventName") or ""
    out: dict = {}
    if name == "beforeSubmitPrompt":
        out = {"continue": True}
    elif name == "preToolUse":
        out = {"permission": "allow"}
    elif name in ("beforeShellExecution", "beforeMCPExecution"):
        out = {"permission": "allow"}
    elif name == "beforeReadFile":
        out = {"permission": "allow"}
    elif name == "subagentStart":
        out = {"permission": "allow"}
    try:
        sys.stdout.write(json.dumps(out) + "\n")
    except Exception:
        pass


if __name__ == "__main__":
    main()
PY
