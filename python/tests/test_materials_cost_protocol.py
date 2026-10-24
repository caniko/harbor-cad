"""Sourced material listing and estimate-only cost through the shared Rust worker."""

import asyncio
import json
import os
import subprocess
import sys
import time
from pathlib import Path

from mcp import Client
from mcp.client.stdio import StdioServerParameters


def test_materials_and_cost_match_cli_through_real_mcp(tmp_path, monkeypatch):
    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    repo = Path(__file__).parents[2]
    root = tmp_path / "state"
    socket = root / "worker.sock"
    request = {
        "geometry": {
            "volume_m3": 1.2e-4,
            "bbox_m": [0.2, 0.15, 0.08],
            "min_wall_m": 0.0025,
            "surface_area_m2": None,
        },
        "material_id": "pa12-mjf",
        "process_id": "mjf-pa12",
        "quantity": 50,
    }
    request_path = tmp_path / "cost.json"
    request_path.write_text(json.dumps(request))
    listed = json.loads(subprocess.check_output([binary, "materials", "list"]))
    expected = json.loads(
        subprocess.check_output([binary, "cost", "estimate", str(request_path)])
    )
    assert expected["estimate"]["currency"] == "USD"
    assert (expected["estimate"]["low"] is None) == bool(
        expected["estimate"]["missing"]
    )
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

        async def check():
            parameters = StdioServerParameters(
                command=sys.executable,
                args=["-m", "harbor_cad_mcp.server", "--profile", "simulation"],
                env=dict(os.environ),
            )
            async with Client(parameters) as client:
                result = await client.call_tool("materials_list", {})
                assert not result.is_error, result
                assert result.structured_content == listed
                assert len(listed["materials"]) == 13
                result = await client.call_tool(
                    "cost_estimate", {"request_spec": request}
                )
                assert not result.is_error, result
                assert result.structured_content == expected
                bad = dict(request, material_id="unobtainium")
                refused = await client.call_tool("cost_estimate", {"request_spec": bad})
                assert refused.is_error and "invalid_input:" in str(refused.content)

        asyncio.run(check())
    finally:
        worker.terminate()
        worker.communicate(timeout=10)
