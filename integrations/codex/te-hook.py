#!/usr/bin/env python3
"""Codex hook dispatcher for tenex-edge.

The Codex config trusts this source-tree script. Keep it thin: all session,
turn, inbox, and peer-context logic belongs in the Rust CLI behind
`tenex-edge hook --host codex`.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


def log(message: str) -> None:
    path = Path(
        os.environ.get(
            "TENEX_EDGE_HOOK_LOG",
            str(Path.home() / ".tenex" / "edge" / "codex-hook.log"),
        )
    )
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        stamp = datetime.now(timezone.utc).isoformat()
        with path.open("a", encoding="utf-8") as f:
            f.write(f"{stamp} {message}\n")
    except Exception:
        # Hooks must fail open; logging must never break Codex.
        pass


def tenex_edge_bin() -> str | None:
    override = os.environ.get("TENEX_EDGE_BIN")
    if override:
        return os.path.expanduser(override)
    found = shutil.which("tenex-edge")
    if found:
        return found
    fallback = Path.home() / ".local" / "bin" / "tenex-edge"
    if fallback.exists():
        return str(fallback)
    return None


def main() -> int:
    if len(sys.argv) < 2:
        log("missing hook type")
        return 0

    hook_type = sys.argv[1]
    raw_stdin = sys.stdin.read()
    bin_path = tenex_edge_bin()
    if not bin_path:
        log(f"tenex-edge binary not found for hook={hook_type}")
        return 0

    cmd = [bin_path, "hook", "--host", "codex", "--type", hook_type]
    try:
        result = subprocess.run(
            cmd,
            input=raw_stdin,
            text=True,
            capture_output=True,
            check=False,
        )
    except Exception as exc:
        log(f"exec failed hook={hook_type}: {exc}")
        return 0

    if result.stdout:
        print(result.stdout, end="")
    if result.stderr:
        log(f"stderr hook={hook_type}: {result.stderr.strip()}")
    if result.returncode != 0:
        log(f"nonzero hook={hook_type}: code={result.returncode}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
