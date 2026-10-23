"""Real MCP client + Rust worker for results_correlate; no native job is executed."""

import asyncio
import json
import os
import subprocess
import sys
import time
from pathlib import Path

from jsonschema import Draft202012Validator
from mcp import Client
from mcp.client.stdio import StdioServerParameters


def test_results_correlate_reads_bounded_csv_under_input_root(tmp_path):
    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    inputs = tmp_path / "inputs"
    inputs.mkdir()
    (inputs / "ok.csv").write_text(
        "time,probe,value,unit\n0,tc1,20.0,degC\n10,tc1,20.5,degC\n"
    )
    (inputs / "unsorted.csv").write_text(
        "time,probe,value,unit\n10,tc1,20.0,degC\n0,tc1,20.5,degC\n"
    )
    profile = json.loads((Path(__file__).parents[2] / "profiles/ci.json").read_text())
    profile["allowed_input_root"] = str(inputs)
    (tmp_path / "profile.json").write_text(json.dumps(profile))
    state = tmp_path / "state"
    socket = state / "worker.sock"
    environment = {
        **os.environ,
        "PYTHONPATH": str(Path(__file__).parents[1]),
        "HARBOR_CAD_SOCKET": str(socket),
    }
    mcp_binary = os.environ.get("HARBOR_CAD_MCP_TEST_BINARY")
    if mcp_binary:
        environment["PYTHONPATH"] = ""
    request = {
        "schema_version": 1,
        "job_id": "a05f78ac-a7ce-4aed-a458-e4a4cbf0b9fc",
        "measurement_csv": "ok.csv",
        "dimension": "temperature",
        "probe_map": {"tc1": {"association": "node", "node_id": 1}},
        "time_offset_s": 0.0,
        "time_window": [0.0, 10.0],
        "tolerance": {"abs": {"value": 1.0, "unit": "K"}, "rel": 0.0},
        "provenance": {
            "test_id": "CC-007",
            "rig": "climate chamber A",
            "sensor_type": "type T thermocouple",
            "sensor_accuracy": {"value": 0.5, "unit": "K"},
            "date": "2026-10-01",
        },
    }
    schemas = json.loads(subprocess.check_output([binary, "schema"]))
    Draft202012Validator(schemas["CorrelationRequest"]).validate(request)
    worker = subprocess.Popen(
        [
            binary,
            "worker",
            "--state",
            str(state),
            "--profile",
            str(tmp_path / "profile.json"),
        ],
        env=environment,
    )
    try:
        deadline = time.monotonic() + 10
        while not socket.exists():
            assert worker.poll() is None and time.monotonic() < deadline
            time.sleep(0.01)

        async def check():
            parameters = StdioServerParameters(
                command=mcp_binary or sys.executable,
                args=([] if mcp_binary else ["-m", "harbor_cad_mcp.server"])
                + ["--profile", "results"],
                env=environment,
            )
            async with Client(parameters) as client:
                tools = {t.name: t for t in (await client.list_tools()).tools}
                Draft202012Validator(tools["results_correlate"].input_schema).validate(
                    {"request_spec": request}
                )
                # A valid CSV passes and only then reaches the (absent) job.
                absent = await client.call_tool(
                    "results_correlate", {"request_spec": request}
                )
                assert absent.is_error and "invalid_input" not in str(absent.content)
                for csv, reason in (
                    ("unsorted.csv", "strictly increasing"),
                    ("../ok.csv", "non-traversing"),
                ):
                    refused = await client.call_tool(
                        "results_correlate",
                        {"request_spec": {**request, "measurement_csv": csv}},
                    )
                    assert refused.is_error and reason in str(refused.content)

        asyncio.run(check())
        (tmp_path / "request.json").write_text(
            json.dumps({**request, "measurement_csv": "unsorted.csv"})
        )
        cli = subprocess.run(
            [binary, "results", "correlate", str(tmp_path / "request.json")],
            env=environment,
            capture_output=True,
            check=False,
        )
        assert cli.returncode == 1 and b"strictly increasing" in cli.stdout
    finally:
        worker.terminate()
        worker.communicate(timeout=10)
