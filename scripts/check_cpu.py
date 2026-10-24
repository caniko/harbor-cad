"""One bounded CPU gate shared by generated CI and the approved local shell."""

import os
import subprocess
import time
from pathlib import Path


def main():
    root = Path(__file__).resolve().parents[1]
    uv = os.environ.get("UV_BIN", "uv")
    env = {**os.environ, "CARGO_BUILD_JOBS": "2", "UV_PYTHON": "3.13.15"}
    deadline = time.monotonic() + 20 * 60

    def run(*command, **settings):
        print("+", " ".join(command), flush=True)
        subprocess.run(
            command,
            cwd=root,
            env={**env, **settings},
            check=True,
            timeout=max(0.01, deadline - time.monotonic()),
        )

    # CI downloads into its isolated job; local qualification uses the approved
    # interpreter already on PATH and never installs another host interpreter.
    if os.environ.get("GITHUB_ACTIONS") == "true":
        run(uv, "python", "install", "--no-config", "--no-bin", "3.13.15")
    run(uv, "sync", "--locked", "--project", "python", UV_PYTHON_DOWNLOADS="never")
    run("cargo", "fmt", "--all", "--", "--check")
    run(
        uv,
        "run",
        "--locked",
        "--project",
        "python",
        "ruff",
        "format",
        "--check",
        "python",
        "adapters",
        "scripts",
    )
    run(
        uv,
        "run",
        "--locked",
        "--project",
        "python",
        "ruff",
        "check",
        "python",
        "adapters",
        "scripts",
    )
    run("cargo", "clippy", "--locked", "--all-targets", "--", "-D", "warnings")
    run("cargo", "test", "--locked")
    run("cargo", "build", "--locked")
    run(
        uv,
        "run",
        "--locked",
        "--project",
        "python",
        "pytest",
        "-q",
        "python/tests",
        HARBOR_CAD_TEST_BINARY=str(root / "target/debug/harbor-cad"),
    )


if __name__ == "__main__":
    main()
