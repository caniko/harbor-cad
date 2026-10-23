"""Real worker design loop through a real MCP client: approval, external proposal, bounded status."""

import asyncio
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


def test_design_mcp_approval_proposal_status_and_export(tmp_path):
    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    repo = Path(__file__).parents[2]
    state = tmp_path / "state"
    endpoint = state / "worker.sock"

    def command(*arguments):
        process = subprocess.run(
            [binary, *map(str, arguments)], capture_output=True, timeout=30, check=False
        )
        assert process.returncode == 0, process.stdout
        return json.loads(process.stdout)

    envelope = command("design", "init")
    assert envelope == json.loads((repo / "examples/design-envelope.json").read_text())
    envelope["budget"]["max_evaluations"] = 8
    envelope["budget"]["max_artifact_bytes"] = 8 * 1024 * 1024
    Draft202012Validator(command("schema")["DesignEnvelope"]).validate(envelope)

    def jobs():
        with sqlite3.connect(f"file:{state / 'jobs.sqlite3'}?mode=ro", uri=True) as db:
            return db.execute("select count(*) from jobs").fetchone()[0]

    worker = subprocess.Popen(
        [
            binary,
            "worker",
            "--state",
            str(state),
            "--profile",
            str(repo / "profiles/ci.json"),
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        deadline = time.monotonic() + 5
        while not endpoint.exists():
            assert worker.poll() is None and time.monotonic() < deadline
            time.sleep(0.01)

        async def exercise():
            parameters = StdioServerParameters(
                command=sys.executable,
                args=["-m", "harbor_cad_mcp.server", "--profile", "simulation"],
                env={
                    **os.environ,
                    "PYTHONPATH": str(repo / "python"),
                    "HARBOR_CAD_SOCKET": str(endpoint),
                },
            )
            async with Client(parameters) as client:
                tools = {t.name: t for t in (await client.list_tools()).tools}

                async def call(name, arguments, error=None):
                    result = await client.call_tool(name, arguments)
                    if error:
                        assert result.is_error and error in result.content[0].text
                        return None
                    assert not result.is_error, result.content
                    Draft202012Validator(tools[name].output_schema).validate(
                        result.structured_content
                    )
                    return result.structured_content

                planned = await call("design_plan", {"envelope": envelope})
                assert planned["prepared"]["executed"] is False
                digest = planned["approval_digest"]
                submit = {"envelope": envelope, "idempotency_key": "d"}
                await call(
                    "design_submit",
                    {**submit, "approved_digest": "0" * 64},
                    error="invalid_input",
                )
                assert jobs() == 0
                submitted = await call(
                    "design_submit", {**submit, "approved_digest": digest}
                )
                design_id = submitted["id"]

                def candidate(height):
                    return {
                        "design_id": design_id,
                        "candidate": {
                            "params": {"height": height, "acceleration": 0.34},
                            "provenance": "agent",
                        },
                    }

                await call("design_propose", candidate(6.5), error="invalid_input")
                proposed = await call("design_propose", candidate(6.0))
                assert proposed["candidate"]["origin"] == "external"
                deadline = time.monotonic() + 60
                while True:
                    status = await call("design_status", {"design_id": design_id})
                    if status["state"] != "running":
                        break
                    assert time.monotonic() < deadline, status
                    await asyncio.sleep(0.05)
                assert status["state"] == "budget_exhausted"
                assert status["budget"]["evaluations"] == 8
                assert status["physical_validation"] == "unqualified"
                assert len(json.dumps(status)) < 65536
                report = await call("design_export", {"design_id": design_id})
                assert report["winner"]["params"] == {
                    "acceleration": 0.34,
                    "height": 6.0,
                }
                assert {c["evidence"] for c in report["winner"]["constraints"]} == {
                    "simulated"
                }
                assert "not physical validation" in report["statement"]
                return status

        status = asyncio.run(exercise())
        assert jobs() == sum(len(i["jobs"]) for i in status["iterations"])
    finally:
        worker.terminate()
        worker.wait(timeout=5)
