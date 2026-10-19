"""Read-only runtime checks share submission validation, not job admission."""

import asyncio
import copy
import json
import os
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

from jsonschema import Draft202012Validator
from mcp import Client
from mcp.client.stdio import StdioServerParameters


def test_execution_preflight_cli_and_all_real_mcp_profiles_do_not_submit(
    tmp_path, monkeypatch
):
    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    repo = Path(__file__).parents[2]
    root = tmp_path / "state"
    socket = root / "worker.sock"
    worker = subprocess.Popen(
        [
            binary,
            "worker",
            "--state",
            str(root),
            "--profile",
            str(repo / "profiles/ci.json"),
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        deadline = time.monotonic() + 10
        while not socket.exists():
            assert worker.poll() is None and time.monotonic() < deadline
            time.sleep(0.01)
        monkeypatch.setenv("HARBOR_CAD_SOCKET", str(socket))
        case = subprocess.check_output([binary, "case", "init"])
        plan = json.loads(
            subprocess.check_output([binary, "case", "plan", "/dev/stdin"], input=case)
        )["plan"]

        async def check():
            expected = None
            for profile in ("cad", "simulation", "results", "all"):
                parameters = StdioServerParameters(
                    command=sys.executable,
                    args=["-m", "harbor_cad_mcp.server", "--profile", profile],
                    env=dict(os.environ),
                )
                async with Client(parameters) as client:
                    tools = {tool.name for tool in (await client.list_tools()).tools}
                    assert "execution_preflight" in tools
                    result = await client.call_tool(
                        "execution_preflight", {"plan": plan}
                    )
                    assert not result.is_error, result
                    report = result.structured_content
                    assert report["execution_approved"] is False
                    assert report["resources_reserved"] is False
                    assert report["executed"] is False
                    assert report["qualification"] == "not_assessed"
                    if expected is not None:
                        assert report == expected
                    expected = report
                    invalid = copy.deepcopy(plan)
                    invalid["policy"] = "research"
                    rejected = await client.call_tool(
                        "execution_preflight", {"plan": invalid}
                    )
                    assert rejected.is_error and "invalid_input" in str(
                        rejected.content
                    )
                    invalid = copy.deepcopy(plan)
                    invalid["stages"][0]["ram_bytes"] = 1024**3
                    rejected = await client.call_tool(
                        "execution_preflight", {"plan": invalid}
                    )
                    assert rejected.is_error and "resource_unavailable" in str(
                        rejected.content
                    )
            return expected

        expected = asyncio.run(check())
        actual = json.loads(
            subprocess.check_output(
                [binary, "job", "preflight", "/dev/stdin"],
                input=json.dumps(plan).encode(),
            )
        )["data"]
        assert actual == expected
        schemas = json.loads(subprocess.check_output([binary, "schema"]))
        Draft202012Validator(schemas["ExecutionPreflight"]).validate(actual)
        with sqlite3.connect(
            f"file:{root / 'jobs.sqlite3'}?mode=ro", uri=True
        ) as connection:
            for table in (
                "jobs",
                "events",
                "artifacts",
                "job_profiles",
                "job_executions",
                "job_authorizations",
                "studies",
            ):
                assert (
                    connection.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
                    == 0
                )
        assert not (root / "artifacts").exists()
    finally:
        worker.terminate()
        worker.wait(timeout=10)
