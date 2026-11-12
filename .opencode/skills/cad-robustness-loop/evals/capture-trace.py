"""Capture a completed session's visible history without CLI pipe truncation.

Usage: python3 -B capture-trace.py SESSION_ID NEW_TRACE_PATH
The raw export stays in an anonymous temporary file, never in the saved trace.
"""

import os
from pathlib import Path
import subprocess
import sys
import tempfile

session, output = sys.argv[1:]
with tempfile.TemporaryFile() as raw:
    # The installed CLI truncates large exports to pipes; a regular fd drains fully.
    subprocess.run(
        ["opencode", "api", "GET", f"/api/experimental/session/{session}/export"],
        stdout=raw,
        check=True,
        timeout=30,
    )
    raw.seek(0)
    with Path(output).open("x") as trace:
        subprocess.run(
            ["jq", "-e", "-f", str(Path(__file__).with_name("export-trace.jq"))],
            stdin=raw,
            stdout=trace,
            env={**os.environ, "SESSION_ID": session},
            check=True,
            timeout=10,
        )
