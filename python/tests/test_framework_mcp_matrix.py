"""Executable interface coverage is not native execution or scientific qualification."""

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


def test_framework_matrix_matches_real_profiles_and_shared_worker(tmp_path):
    matrix = json.loads(
        Path(__file__).with_name("framework_mcp_matrix.json").read_text()
    )
    assert matrix["schema_version"] == 1
    frameworks = {row["framework"]: row for row in matrix["frameworks"]}
    assert len(frameworks) == len(matrix["frameworks"])
    assert set(frameworks) == {
        "Fleetix",
        "SQLite",
        "systemd",
        "Bubblewrap",
        "Official MCP SDK",
        "Nix",
        "Harbor-Rs",
        "Harbor-Py",
        "FreeCAD",
        "Gmsh",
        "OpenLB",
        "HIP/ROCm",
        "CalculiX",
        "PaStiX4CalculiX",
        "PaRSEC",
        "libRadtran",
        "Mitsuba",
        "Dr.Jit",
        "VTK",
        "Viskores",
        "Kokkos",
        "ParaView",
        "EGL",
        "FFmpeg",
        "VAAPI",
        "Catalyst",
        "Conduit",
        "VTKHDF",
    }
    profiles = {"cad", "simulation", "results", "all"}
    registered = {}
    for group in matrix["tool_groups"]:
        assert group["tools"] and "all" in group["profiles"]
        assert set(group["profiles"]) <= profiles
        for name in group["tools"]:
            assert name not in registered
            registered[name] = set(group["profiles"])
    assert set().union(*(set(row["tools"]) for row in frameworks.values())) == set(
        registered
    )
    for row in frameworks.values():
        assert row["scope"]
        if row["coverage"] in {"missing", "optional_deferred"}:
            assert not row["tools"] and not row.get("inspection")
        else:
            assert row["coverage"] in {
                "inspection",
                "bounded_workflow",
                "embedded_workflow",
            }
            assert row["tools"]
            if row["coverage"] == "inspection":
                assert row["inspection"] and "doctor" in row["tools"]
            else:
                assert row["refusal_probe"] in row["tools"]
                assert row["refusal_probe"] in matrix["refusal_probes"]
    assert frameworks["PaStiX4CalculiX"]["coverage"] == "missing"
    assert frameworks["PaRSEC"]["coverage"] == "missing"
    for name in ("Catalyst", "Conduit", "VTKHDF"):
        assert frameworks[name]["coverage"] == "optional_deferred"

    repo = Path(__file__).parents[2]
    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    state = tmp_path / "state"
    empty_path = tmp_path / "no-native-programs"
    empty_path.mkdir()
    environment = {
        **os.environ,
        "PATH": str(empty_path),
        "PYTHONPATH": str(repo / "python"),
        "HARBOR_CAD_SOCKET": str(state / "worker.sock"),
    }
    worker = subprocess.Popen(
        [
            binary,
            "worker",
            "--state",
            str(state),
            "--profile",
            str(repo / "profiles/ci.json"),
        ],
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        deadline = time.monotonic() + 10
        while not (state / "worker.sock").exists():
            assert worker.poll() is None and time.monotonic() < deadline
            time.sleep(0.01)

        async def check():
            for profile in sorted(profiles):
                parameters = StdioServerParameters(
                    command=sys.executable,
                    args=["-m", "harbor_cad_mcp.server", "--profile", profile],
                    env=environment,
                )
                async with Client(parameters) as client:
                    tools = {
                        tool.name: tool for tool in (await client.list_tools()).tools
                    }
                    assert set(tools) == {
                        name
                        for name, allowed in registered.items()
                        if profile in allowed
                    }
                    for tool in tools.values():
                        Draft202012Validator.check_schema(tool.input_schema)
                    for name in set(registered) - set(tools) | set(
                        matrix["forbidden_tools"]
                    ):
                        assert name not in tools
                        denied = await client.call_tool(name, {})
                        assert denied.is_error and "Unknown tool" in str(denied.content)
                    report = await client.call_tool("doctor", {})
                    assert not report.is_error
                    doctor = report.structured_content
                    assert doctor["hardware_qualification"].startswith("unqualified")
                    assert doctor["build_provenance"]["executed"] is False
                    assert doctor["build_provenance"]["qualification"] == "not_assessed"
                    for row in frameworks.values():
                        value = doctor
                        for key in row.get("inspection", []):
                            value = value[key]
                        if row.get("inspection"):
                            assert value
                    for name, arguments in matrix["refusal_probes"].items():
                        if name in tools:
                            Draft202012Validator(tools[name].input_schema).validate(
                                arguments
                            )
                            refused = await client.call_tool(name, arguments)
                            assert refused.is_error and "invalid_input:" in str(
                                refused.content
                            )

        asyncio.run(check())
        with sqlite3.connect(
            f"file:{state / 'jobs.sqlite3'}?mode=ro", uri=True
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
        assert not (state / "artifacts").exists()
    finally:
        worker.terminate()
        worker.communicate(timeout=10)
