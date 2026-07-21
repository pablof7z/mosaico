"""Hermes lifecycle bridge for the Mosaico fabric."""

from __future__ import annotations

import json
import os
import subprocess
from typing import Any, Callable


def _payload(kwargs: dict[str, Any]) -> dict[str, Any]:
    return {
        "session_id": kwargs.get("session_id") or "",
        "cwd": os.getcwd(),
        "pid": os.getpid(),
    }


def _invoke(hook_type: str, kwargs: dict[str, Any]) -> dict[str, Any] | None:
    try:
        completed = subprocess.run(
            ["mosaico", "harness", "hook", "hermes", "--type", hook_type],
            input=json.dumps(_payload(kwargs)),
            text=True,
            capture_output=True,
            timeout=5,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if completed.returncode != 0 or not completed.stdout.strip():
        return None
    try:
        parsed = json.loads(completed.stdout)
    except json.JSONDecodeError:
        return None
    return parsed if isinstance(parsed, dict) else None


def _context(result: dict[str, Any] | None) -> str | None:
    if not result:
        return None
    value = result.get("context")
    return value if isinstance(value, str) and value.strip() else None


def on_session_start(**kwargs: Any) -> None:
    _invoke("session-start", kwargs)


def on_pre_llm_call(**kwargs: Any) -> dict[str, str] | None:
    content = _context(_invoke("user-prompt-submit", kwargs))
    return {"context": content} if content else None


def on_session_end(**kwargs: Any) -> None:
    _invoke("stop", kwargs)


def on_session_finalize(**kwargs: Any) -> None:
    _invoke("session-end", kwargs)


def register(ctx: Any) -> None:
    def on_post_tool_call(**kwargs: Any) -> None:
        content = _context(_invoke("post-tool-use", kwargs))
        if content:
            inject: Callable[[str], bool] = ctx.inject_message
            inject(content)

    ctx.register_hook("on_session_start", on_session_start)
    ctx.register_hook("pre_llm_call", on_pre_llm_call)
    ctx.register_hook("post_tool_call", on_post_tool_call)
    ctx.register_hook("on_session_end", on_session_end)
    ctx.register_hook("on_session_finalize", on_session_finalize)
