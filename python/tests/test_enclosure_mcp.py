"""Real MCP client: general-solid mesh and enclosure thermal planning are discoverable and refuse typed."""

import asyncio
import os
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

from jsonschema import Draft202012Validator
from mcp import Client
from mcp.client.stdio import StdioServerParameters


def known(value, unit):
    return {
        "availability": "known",
        "value": {"value": value, "unit": unit},
        "provenance": "synthetic MCP refusal probe",
        "synthetic": True,
    }


MESH = {
    "source_job": "2fe6dac3-6bcd-439a-bdbd-80d1029ada8c",
    "regions": ["housing", "lid"],
    "mesh": {
        "characteristic_length_max_m": 0.006,
        "element_order": 1,
        "max_elements": 200000,
        "volume_relative_tolerance": 0.01,
        "min_quality": 0.05,
        "geometry_tolerance_m": 1e-05,
    },
}
THERMAL = {
    "mesh_job": "2fe6dac3-6bcd-439a-bdbd-80d1029ada8c",
    "spec": {
        "schema_version": 1,
        "materials": {
            region: {"kind": "declared", "conductivity": known(0.2, "W/(m*K)")}
            for region in ("housing", "lid")
        },
        "heat_source_region": "lid",
        "heat_source_power": known(2.0, "W"),
        "ambient_temperature": known(300.0, "K"),
        "film_coefficient": known(10.0, "W/(m^2*K)"),
        "exposed_regions": ["housing"],
        "internal_film_coefficient": known(3.0, "W/(m^2*K)"),
        "radiation": False,
        "energy_balance_tolerance": 1e-3,
    },
}


def test_solid_mesh_and_enclosure_thermal_tools_discover_and_refuse_typed(tmp_path):
    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    repo = Path(__file__).parents[2]
    state = tmp_path / "state"
    endpoint = state / "worker.sock"
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

        async def exercise(profile, present, absent, arguments):
            parameters = StdioServerParameters(
                command=sys.executable,
                args=["-m", "harbor_cad_mcp.server", "--profile", profile],
                env={
                    **os.environ,
                    "PYTHONPATH": str(repo / "python"),
                    "HARBOR_CAD_SOCKET": str(endpoint),
                },
            )
            async with Client(parameters) as client:
                tools = {t.name: t for t in (await client.list_tools()).tools}
                assert absent not in tools
                tool = tools[present]
                assert "not" in tool.description.lower()
                Draft202012Validator.check_schema(tool.output_schema)
                Draft202012Validator(tool.input_schema).validate(arguments)
                # CI admission has no host authority: a well-formed request is
                # refused as unqualified before any source is resolved.
                refused = await client.call_tool(present, arguments)
                assert refused.is_error
                text = refused.content[0].text
                assert (
                    "unqualified:" in text
                    and "requires authoritative admission" in text
                )
                malformed = await client.call_tool(present, {"request_spec": {}})
                assert malformed.is_error
                assert "invalid_input:" in malformed.content[0].text

        asyncio.run(
            exercise(
                "cad",
                "cad_plan_solid_mesh",
                "enclosure_thermal_plan",
                {"request_spec": MESH},
            )
        )
        asyncio.run(
            exercise(
                "simulation",
                "enclosure_thermal_plan",
                "cad_plan_solid_mesh",
                {"request_spec": THERMAL},
            )
        )
        with sqlite3.connect(f"file:{state / 'jobs.sqlite3'}?mode=ro", uri=True) as db:
            assert db.execute("select count(*) from jobs").fetchone()[0] == 0
    finally:
        worker.terminate()
        worker.wait(timeout=5)
