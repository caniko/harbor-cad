"""Version-5 FEM identity and absence of invented fluid/time fields across schemas."""

import asyncio
import json
import os
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator, ValidationError
from mcp import Client
from mcp.client.stdio import StdioServerParameters


def test_fem_cli_plan_and_schema_preserve_the_independent_si_recipe(tmp_path):
    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    spec = {
        "schema_version": 1,
        "synthetic": True,
        "backend": "cpu",
        "mode": "thermal_boundary",
        "size_m": [0.02, 0.01, 0.01],
        "resolution": 4,
        "geometry_tolerance_m": 1e-6,
        "temperatures_k": [293.15, 303.15],
        "numerical_tolerance": 1e-6,
        "conductivity_w_m_k": 20.0,
    }
    source = tmp_path / "fem.json"
    source.write_text(json.dumps(spec))
    reply = json.loads(
        subprocess.check_output([binary, "case", "plan-fem-reference", str(source)])
    )
    plan = reply["plan"]
    assert plan["schema_version"] == 5 and plan["fem"] == spec and "case" not in plan
    assert plan["observation"]["retained_times_s"] == []
    schema = json.loads(subprocess.check_output([binary, "schema"]))
    validator = Draft202012Validator(schema["ExecutionPlan"])
    validator.validate(plan)
    for changed in (
        {**plan, "case": None},
        {**plan, "fem": None},
        {**plan, "schema_version": 4},
    ):
        with pytest.raises(ValidationError):
            validator.validate(changed)
    spec["mode"] = "free_expansion"
    del spec["conductivity_w_m_k"]
    spec.update(young_modulus_pa=200e9, poisson_ratio=0.3, expansion_per_k=12e-6)
    source.write_text(json.dumps(spec))
    changed = json.loads(
        subprocess.check_output([binary, "case", "plan-fem-reference", str(source)])
    )
    assert changed["approval_digest"] != reply["approval_digest"]
    validator.validate(changed["plan"])


def test_factorization_inspection_is_not_execution_or_a_cpu_fallback(tmp_path):
    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    repo = Path(__file__).parents[2]
    backends = json.loads(subprocess.check_output([binary, "backend", "list"]))
    expected_revisions = {
        "pastix4calculix": "d6b493bc431ecf860b3578a2ee6a25f392eb44be",
        "parsec": "b580d208094e6cb4106e41d5eeec84f304a090b8",
    }
    for adapter, revision in expected_revisions.items():
        [entry] = [item for item in backends if item["adapter"] == adapter]
        assert entry["candidate_revision"] == revision
        assert entry["availability"] == "inspection_only"
        assert entry["build"] == "not_packaged" and entry["execution"] == "unsupported"
        assert (
            entry["runtime"]
            == entry["numerical_verification"]
            == entry["physical_validation"]
            == "unqualified"
        )
        assert entry["executed"] is False and entry["resource_estimate"] is None
        assert entry["integer_abi"] == "ilp64_required_not_integrated"
        assert entry["backend"] == "cuda" and entry["priority"] == "best_effort"
    spec = {
        "schema_version": 1,
        "synthetic": True,
        "backend": "cpu",
        "mode": "thermal_boundary",
        "size_m": [0.02, 0.01, 0.01],
        "resolution": 4,
        "geometry_tolerance_m": 1e-6,
        "temperatures_k": [293.15, 303.15],
        "numerical_tolerance": 1e-6,
        "conductivity_w_m_k": 20.0,
    }
    cpu_plan = json.loads(
        subprocess.check_output(
            [binary, "case", "plan-fem-reference", "/dev/stdin"],
            input=json.dumps(spec).encode(),
            timeout=10,
        )
    )
    profile = json.loads((repo / "profiles/ci.json").read_text())
    profile.update(policy="research", service_mode="systemd")
    profile_path = tmp_path / "planning-profile.json"
    profile_path.write_text(json.dumps(profile))
    state = tmp_path / "state"
    environment = {
        **os.environ,
        "PYTHONPATH": str(repo / "python"),
        "HARBOR_CAD_SOCKET": str(state / "worker.sock"),
    }
    mcp_binary = os.environ.get("HARBOR_CAD_MCP_TEST_BINARY")
    if mcp_binary:
        environment["PYTHONPATH"] = ""
    worker = subprocess.Popen(
        [
            binary,
            "worker",
            "--state",
            str(state),
            "--profile",
            str(profile_path),
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
            for profile in ("cad", "simulation", "results", "all"):
                parameters = StdioServerParameters(
                    command=mcp_binary or sys.executable,
                    args=([] if mcp_binary else ["-m", "harbor_cad_mcp.server"])
                    + ["--profile", profile],
                    env=environment,
                )
                async with Client(parameters) as client:
                    inspected = await client.call_tool("backend_list", {})
                    assert (
                        not inspected.is_error
                        and inspected.structured_content["result"] == backends
                    )
                    assert [
                        json.loads(item.text) for item in inspected.content
                    ] == backends
                    doctor = await client.call_tool("doctor", {})
                    assert (
                        not doctor.is_error
                        and doctor.structured_content["backends"] == backends
                    )
                    if profile not in {"simulation", "all"}:
                        continue
                    cpu = await client.call_tool(
                        "case_plan_fem_reference", {"spec": spec}
                    )
                    assert not cpu.is_error and cpu.structured_content == cpu_plan
                    for changed in (
                        {**spec, "backend": "cuda"},
                        {**spec, "backend": "hip"},
                        {**spec, "backend": "vulkan"},
                        {**spec, "factorization": "pastix"},
                        {**spec, "factorization": "parsec"},
                    ):
                        cli = await asyncio.to_thread(
                            subprocess.run,
                            [binary, "case", "plan-fem-reference", "/dev/stdin"],
                            input=json.dumps(changed).encode(),
                            capture_output=True,
                            timeout=10,
                            check=False,
                        )
                        assert (
                            cli.returncode != 0
                            and json.loads(cli.stdout)["error"]["code"]
                            == "invalid_input"
                        )
                        refused = await client.call_tool(
                            "case_plan_fem_reference", {"spec": changed}
                        )
                        assert refused.is_error and "invalid_input:" in str(
                            refused.content
                        )

        asyncio.run(check())
        with sqlite3.connect(
            f"file:{state / 'jobs.sqlite3'}?mode=ro", uri=True
        ) as connection:
            assert connection.execute("SELECT count(*) FROM jobs").fetchone()[0] == 0
        assert not (state / "artifacts").exists()
    finally:
        worker.terminate()
        worker.communicate(timeout=10)
