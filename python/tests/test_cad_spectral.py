"""Rust spectral-scene schema parity and shared-worker typed input boundaries."""

import json
import os
import subprocess
import time
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator, ValidationError


def test_cad_spectral_schema_retains_missing_response_and_refuses_foreign_or_null_properties():
    repo = Path(__file__).parents[2]
    request = json.loads((repo / "examples/cad-spectral-scene.json").read_text())
    schemas = json.loads(
        subprocess.check_output([os.environ["HARBOR_CAD_TEST_BINARY"], "schema"])
    )
    validator = Draft202012Validator(schemas["CadSpectralSceneRequest"])
    transport = json.loads((repo / "examples/cad-spectral-transport.json").read_text())
    transport_validator = Draft202012Validator(schemas["CadSpectralTransportRequest"])
    transport_validator.validate(transport)
    query = {
        "schema_version": 1,
        "job_id": "00000000-0000-0000-0000-000000000001",
        "seed": 17,
        "region_name": "solid",
    }
    query_validator = Draft202012Validator(schemas["CadOpticalResultsRequest"])
    query_validator.validate(query)
    for field in ("execute", "physical_time_s", "average_seeds"):
        with pytest.raises(ValidationError):
            query_validator.validate({**query, field: True})
    for changed in (
        {**transport, "execute": True},
        {**transport, "scene": None},
        {**transport, "samples_per_triangle": None},
    ):
        with pytest.raises(ValidationError):
            transport_validator.validate(changed)
    validator.validate(request)
    for response in (
        {"availability": "missing", "reason": "unknown optics"},
        request["materials"][0]["response"],
    ):
        validator.validate(
            {
                **request,
                "materials": [{**request["materials"][0], "response": response}],
            }
        )
    for changed in (
        {**request, "execute": True},
        {**request, "geometry_tolerance": None},
        {**request, "materials": [{**request["materials"][0], "response": None}]},
        {
            **request,
            "materials": [
                {
                    **request["materials"][0],
                    "ageing_action": {
                        "availability": "missing",
                        "reason": "no calibration",
                        "value": [0.0, 0.0],
                    },
                }
            ],
        },
    ):
        with pytest.raises(ValidationError):
            validator.validate(changed)


def test_real_cad_spectral_cli_and_stdio_profiles_use_the_same_bounded_worker(
    tmp_path, monkeypatch
):
    repo = Path(__file__).parents[2]
    monkeypatch.syspath_prepend(str(repo / "scripts"))
    from native_worker_campaign import WorkerCampaign

    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    campaign = object.__new__(WorkerCampaign)
    campaign.binary = binary
    campaign.mcp = repo / "python/.venv/bin/harbor-cad-mcp"
    campaign.endpoint = tmp_path / "state/worker.sock"
    campaign.environment = dict(os.environ)
    worker = subprocess.Popen(
        [
            binary,
            "worker",
            "--state",
            str(tmp_path / "state"),
            "--profile",
            str(repo / "profiles/ci.json"),
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        deadline = time.monotonic() + 5
        while not campaign.endpoint.exists():
            assert worker.poll() is None and time.monotonic() < deadline
            time.sleep(0.01)
        request = json.loads((repo / "examples/cad-spectral-scene.json").read_text())
        for key in ("execute", None):
            current = request if key is None else {**request, key: True}
            path = tmp_path / (str(key) + ".json")
            path.write_text(json.dumps(current))
            reply = campaign.command(
                "--socket",
                campaign.endpoint,
                "cad",
                "prepare-spectral-scene",
                path,
                allow_error=True,
            )
            assert reply["ok"] is False and reply["error"]["code"] == (
                "state_error" if key is None else "invalid_input"
            )
            for profile in ("cad", "results"):
                refusal = campaign.mcp_call(
                    "cad_prepare_spectral_scene",
                    {"request_spec": current},
                    profile=profile,
                    expect_error=True,
                )
                assert reply["error"]["code"] + ":" in str(refusal)
        optical = json.loads(
            (repo / "examples/cad-spectral-transport.json").read_text()
        )
        for extra in ({}, {"execute": True}):
            current = {**optical, **extra}
            path = tmp_path / ("optical-" + str(bool(extra)) + ".json")
            path.write_text(json.dumps(current))
            reply = campaign.command(
                "--socket",
                campaign.endpoint,
                "cad",
                "plan-spectral-transport",
                path,
                allow_error=True,
            )
            assert reply["ok"] is False and reply["error"]["code"] == (
                "invalid_input" if extra else "unqualified"
            )
            for profile in ("simulation", "all"):
                refusal = campaign.mcp_call(
                    "cad_plan_spectral_transport",
                    {"request_spec": current},
                    profile=profile,
                    expect_error=True,
                )
                assert reply["error"]["code"] + ":" in str(refusal)
        query = {
            "schema_version": 1,
            "job_id": "00000000-0000-0000-0000-000000000001",
            "seed": 17,
            "region_name": "solid",
        }
        for extra in ({}, {"execute": True}):
            current = {**query, **extra}
            path = tmp_path / ("query-" + str(bool(extra)) + ".json")
            path.write_text(json.dumps(current))
            reply = campaign.command(
                "--socket",
                campaign.endpoint,
                "results",
                "cad-optical",
                path,
                allow_error=True,
            )
            assert reply["ok"] is False and reply["error"]["code"] == (
                "invalid_input" if extra else "state_error"
            )
            refusal = campaign.mcp_call(
                "results_cad_optical",
                {"request_spec": current},
                profile="results",
                expect_error=True,
            )
            assert reply["error"]["code"] + ":" in str(refusal)
    finally:
        worker.terminate()
        worker.communicate(timeout=5)
