"""Strict explicit thermal input and original-source refusal via the same worker."""

import ast
import asyncio
import copy
import inspect
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator, ValidationError
from mcp import Client
from mcp.client.stdio import StdioServerParameters


def test_cooling_campaign_tools_exist_in_their_selected_mcp_profiles(monkeypatch):
    repo = Path(__file__).parents[2]
    monkeypatch.syspath_prepend(str(repo / "scripts"))
    from native_worker_campaign import WorkerCampaign

    default = inspect.signature(WorkerCampaign.mcp_call).parameters["profile"].default
    calls = {}
    tree = ast.parse((repo / "scripts/verify_retained_cooling_worker.py").read_text())
    for node in ast.walk(tree):
        if not (
            isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and isinstance(node.func.value, ast.Name)
            and node.func.value.id == "campaign"
            and node.func.attr == "mcp_call"
        ):
            continue
        profile = next((k.value for k in node.keywords if k.arg == "profile"), None)
        profile = default if profile is None else ast.literal_eval(profile)
        calls.setdefault(profile, []).append(
            (node.lineno, ast.literal_eval(node.args[0]))
        )
    assert calls

    async def exercise():
        for profile, tools in calls.items():
            parameters = StdioServerParameters(
                command=sys.executable,
                args=["-m", "harbor_cad_mcp.server", "--profile", profile],
                env={**os.environ, "PYTHONPATH": str(repo / "python")},
            )
            async with Client(parameters) as client:
                names = {tool.name for tool in (await client.list_tools()).tools}
            for line, tool in tools:
                assert tool in names, (
                    f"cooling campaign:{line}: {tool} absent in {profile}"
                )

    asyncio.run(exercise())


def test_cooling_schema_never_accepts_injected_source_fields_or_missing_thermal_state():
    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    repo = Path(__file__).parents[2]
    request = json.loads((repo / "examples/retained-cooling.json").read_text())
    schema = json.loads(subprocess.check_output([binary, "schema"]))[
        "RetainedCoolingRequest"
    ]
    validator = Draft202012Validator(schema)
    validator.validate(request)
    changes = []
    for key in ("phase", "velocity", "source_temperature", "execute"):
        changes.append({**request, key: None})
    for key in ("initial_temperature", "material_temperature_domain", "moisture_risk"):
        changed = copy.deepcopy(request)
        changed["thermal"].pop(key)
        changes.append(changed)
    changed = copy.deepcopy(request)
    changed["thermal"]["initial_temperature"] = None
    changes.append(changed)
    for changed in changes:
        with pytest.raises(ValidationError):
            validator.validate(changed)


def test_cli_real_results_mcp_preserve_explicit_inputs_and_refuse_unregistered_source(
    tmp_path,
):
    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    repo = Path(__file__).parents[2]
    state = tmp_path / "state"
    settings = json.loads((repo / "profiles/ci.json").read_text())
    settings["allowed_input_root"] = str(tmp_path)
    profile = tmp_path / "profile.json"
    profile.write_text(json.dumps(settings))
    environment = {
        **os.environ,
        "HARBOR_CAD_SOCKET": str(state / "worker.sock"),
        "PYTHONPATH": str(repo / "python"),
        "PYTHONDONTWRITEBYTECODE": "1",
    }
    worker = subprocess.Popen(
        [binary, "worker", "--state", str(state), "--profile", str(profile)],
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    original = json.loads((repo / "examples/retained-cooling.json").read_text())
    changes = [original]
    for key, value in (
        ("initial_temperature", {"value": 274.15, "unit": "K"}),
        ("conductivity", {"value": 0.6, "unit": "kg"}),
        ("material_provenance", ""),
        ("maximum_relative_conservation_error", 0.02),
    ):
        changed = copy.deepcopy(original)
        changed["thermal"][key] = value
        changes.append(changed)
    changes.append({**original, "phase": None})
    try:
        deadline = time.monotonic() + 10
        while not (state / "worker.sock").exists():
            assert worker.poll() is None and time.monotonic() < deadline, (
                worker.communicate(timeout=5)
            )
            time.sleep(0.01)

        async def exercise():
            parameters = StdioServerParameters(
                command=sys.executable,
                args=["-m", "harbor_cad_mcp.server", "--profile", "results"],
                env=environment,
            )
            async with Client(parameters) as client:
                names = {tool.name for tool in (await client.list_tools()).tools}
                assert "results_prepare_retained_cooling" in names
                assert {
                    "retained_cooling_plan",
                    "results_sample_retained_cooling",
                } <= names
                for changed in changes:
                    cli = await asyncio.to_thread(
                        subprocess.run,
                        [binary, "results", "prepare-retained-cooling", "/dev/stdin"],
                        input=json.dumps(changed).encode(),
                        env=environment,
                        capture_output=True,
                        timeout=30,
                        check=False,
                    )
                    response = json.loads(cli.stdout)
                    mcp = await client.call_tool(
                        "results_prepare_retained_cooling", {"request_spec": changed}
                    )
                    assert cli.returncode != 0 and response["ok"] is False
                    assert mcp.is_error and response["error"]["code"] in str(
                        mcp.content
                    )

            execution = {
                "schema_version": 1,
                "initialization": original,
                "spatial_refinement": 2,
                "integration_substeps": 1,
                "base_steps": 16,
                "observation_base_steps": [0, 4, 16],
            }
            parameters.args[-1] = "simulation"
            async with Client(parameters) as client:
                names = {tool.name for tool in (await client.list_tools()).tools}
                assert "retained_cooling_plan" in names
                assert "results_sample_retained_cooling" not in names
                cli = await asyncio.to_thread(
                    subprocess.run,
                    [binary, "results", "plan-retained-cooling", "/dev/stdin"],
                    input=json.dumps(execution).encode(),
                    env=environment,
                    capture_output=True,
                    timeout=30,
                    check=False,
                )
                response = json.loads(cli.stdout)
                mcp = await client.call_tool(
                    "retained_cooling_plan", {"request_spec": execution}
                )
                assert cli.returncode != 0 and response["ok"] is False
                assert response["error"]["code"] == "unqualified" and mcp.is_error
                assert response["error"]["code"] in str(mcp.content)

        asyncio.run(exercise())
        assert not list(state.glob("artifacts/**/*"))
    finally:
        worker.terminate()
        worker.communicate(timeout=5)
