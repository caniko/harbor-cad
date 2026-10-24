"""Prescribed snow parity through the same real Rust worker and MCP client."""

import asyncio
import copy
import json
import os
import subprocess
import time
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator, ValidationError
from mcp import Client
from mcp.client.stdio import StdioServerParameters


def test_snow_prescription_schema_approval_and_typed_applicability_match_cli_and_real_mcp(
    tmp_path, monkeypatch
):
    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    repo = Path(__file__).parents[2]
    spec = json.loads((repo / "examples/snow-reference.json").read_text())
    schemas = json.loads(subprocess.check_output([binary, "schema"]))
    Draft202012Validator(schemas["SnowReferenceSpec"]).validate(spec)
    planned = json.loads(
        subprocess.check_output(
            [binary, "case", "plan-snow-reference", "/dev/stdin"],
            input=json.dumps(spec).encode(),
        )
    )
    validated = json.loads(
        subprocess.check_output(
            [binary, "case", "validate-snow-reference", "/dev/stdin"],
            input=json.dumps(spec).encode(),
        )
    )
    Draft202012Validator(schemas["ExecutionPlan"]).validate(planned["plan"])
    Draft202012Validator(schemas["PreparedSnowBoundary"]).validate(validated)
    assert planned["snow_boundary"] == validated and validated["executed"] is False
    assert planned["plan"]["thermal"] == validated["native"]
    openings = json.loads((repo / "examples/snow-openings.json").read_text())
    Draft202012Validator(schemas["SnowOpeningRequest"]).validate(openings)
    opening_report = json.loads(
        subprocess.check_output(
            [binary, "case", "validate-snow-openings", "/dev/stdin"],
            input=json.dumps(openings).encode(),
        )
    )
    Draft202012Validator(schemas["PreparedSnowOpenings"]).validate(opening_report)
    assert opening_report["input"] == openings and not opening_report["executed"]
    assert abs(opening_report["openings"][0]["covered_area_m2"] - 0.0011) < 1e-15
    assert abs(opening_report["openings"][0]["remaining_area_m2"] - 0.0005) < 1e-15
    spectral = json.loads((repo / "examples/spectral-reference.json").read_text())
    Draft202012Validator(schemas["SpectralReferenceSpec"]).validate(spectral)
    spectral_reference = json.loads(
        subprocess.check_output(
            [binary, "case", "validate-spectral-reference", "/dev/stdin"],
            input=json.dumps(spectral).encode(),
        )
    )
    Draft202012Validator(schemas["PreparedSpectralReference"]).validate(
        spectral_reference
    )
    assert not spectral_reference["executed"]
    assert abs(spectral_reference["absorbed_irradiance_w_m2"] - 132.0) < 1e-10
    spectral_plan = json.loads(
        subprocess.check_output(
            [binary, "case", "plan-spectral-reference", "/dev/stdin"],
            input=json.dumps(spectral).encode(),
        )
    )
    Draft202012Validator(schemas["ExecutionPlan"]).validate(spectral_plan["plan"])
    assert (
        spectral_plan["plan"]["schema_version"] == 13
        and spectral_plan["plan"]["observation"]["retained_times_s"] == []
    )
    for old_field in ("freezing", "wetting", "case", "thermal_contact", "atmosphere"):
        with pytest.raises(ValidationError):
            Draft202012Validator(schemas["ExecutionPlan"]).validate(
                {**spectral_plan["plan"], old_field: None}
            )
    reflection = json.loads(
        (repo / "examples/spectral-reflection-reference.json").read_text()
    )
    Draft202012Validator(schemas["SpectralReflectionSpec"]).validate(reflection)
    reflection_reference = json.loads(
        subprocess.check_output(
            [binary, "case", "validate-spectral-reflection-reference", "/dev/stdin"],
            input=json.dumps(reflection).encode(),
        )
    )
    Draft202012Validator(schemas["PreparedSpectralReflection"]).validate(
        reflection_reference
    )
    assert (
        not reflection_reference["executed"]
        and reflection_reference["model_relative_error_bound"] < 1e-5
    )
    atmosphere = json.loads((repo / "examples/atmosphere-reference.json").read_text())
    Draft202012Validator(schemas["AtmosphericReferenceSpec"]).validate(atmosphere)
    atmosphere_reference = json.loads(
        subprocess.check_output(
            [binary, "case", "validate-atmospheric-reference", "/dev/stdin"],
            input=json.dumps(atmosphere).encode(),
        )
    )
    Draft202012Validator(schemas["PreparedAtmosphericReference"]).validate(
        atmosphere_reference
    )
    assert (
        not atmosphere_reference["executed"] and len(atmosphere_reference["umu"]) == 64
    )
    atmosphere_plan = json.loads(
        subprocess.check_output(
            [binary, "case", "plan-atmospheric-reference", "/dev/stdin"],
            input=json.dumps(atmosphere).encode(),
        )
    )
    Draft202012Validator(schemas["ExecutionPlan"]).validate(atmosphere_plan["plan"])
    assert (
        atmosphere_plan["plan"]["schema_version"] == 14
        and atmosphere_plan["atmospheric_reference"] == atmosphere_reference
    )
    for field in ("case", "spectral", "freezing", "thermal_contact", "atmosphere"):
        with pytest.raises(ValidationError):
            Draft202012Validator(schemas["ExecutionPlan"]).validate(
                {**atmosphere_plan["plan"], field: None}
            )
    profile = json.loads((repo / "profiles/ci.json").read_text())
    profile.update(
        policy="research", service_mode="systemd", allowed_input_root=str(tmp_path)
    )
    profile_path = tmp_path / "profile.json"
    profile_path.write_text(json.dumps(profile))
    state = tmp_path / "state"
    socket = state / "worker.sock"
    worker = subprocess.Popen(
        [binary, "worker", "--state", str(state), "--profile", str(profile_path)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        deadline = time.monotonic() + 10
        while not socket.exists():
            assert worker.poll() is None and time.monotonic() < deadline
            time.sleep(0.01)
        monkeypatch.setenv("HARBOR_CAD_SOCKET", str(socket))

        async def exercise():
            parameters = StdioServerParameters(
                command=str(repo / "python/.venv/bin/python"),
                args=["-m", "harbor_cad_mcp.server", "--profile", "simulation"],
                env=dict(os.environ),
            )
            async with Client(parameters) as client:
                actual = await client.call_tool(
                    "case_plan_snow_reference", {"spec": spec}
                )
                assert not actual.is_error and actual.structured_content == planned
                actual = await client.call_tool(
                    "snow_reference_validate", {"spec": spec}
                )
                assert not actual.is_error and actual.structured_content == validated
                actual = await client.call_tool(
                    "snow_openings_validate", {"spec": openings}
                )
                assert (
                    not actual.is_error and actual.structured_content == opening_report
                )
                for mutated in (
                    {**openings, "convection_from_open_area": True},
                    {**openings, "synthetic": False},
                    {**openings, "geometry_tolerance": {"value": 1.0, "unit": "kg"}},
                ):
                    refusal = await client.call_tool(
                        "snow_openings_validate", {"spec": mutated}
                    )
                    assert refusal.is_error and "invalid_input" in str(refusal.content)
                actual = await client.call_tool(
                    "spectral_reference_validate", {"spec": spectral}
                )
                assert (
                    not actual.is_error
                    and actual.structured_content == spectral_reference
                )
                actual = await client.call_tool(
                    "spectral_reference_plan", {"spec": spectral}
                )
                assert (
                    not actual.is_error and actual.structured_content == spectral_plan
                )
                rejected = await client.call_tool(
                    "job_submit",
                    {
                        "plan": spectral_plan["plan"],
                        "approved_digest": spectral_plan["approval_digest"],
                        "idempotency_key": "no-authority-spectral",
                    },
                )
                assert rejected.is_error and "unqualified" in str(rejected.content)
                actual = await client.call_tool(
                    "spectral_reflection_reference_validate", {"spec": reflection}
                )
                assert (
                    not actual.is_error
                    and actual.structured_content == reflection_reference
                )
                actual = await client.call_tool(
                    "atmospheric_reference_validate", {"spec": atmosphere}
                )
                assert (
                    not actual.is_error
                    and actual.structured_content == atmosphere_reference
                )
                actual = await client.call_tool(
                    "atmospheric_reference_plan", {"spec": atmosphere}
                )
                assert (
                    not actual.is_error and actual.structured_content == atmosphere_plan
                )
                rejected = await client.call_tool(
                    "job_submit",
                    {
                        "plan": atmosphere_plan["plan"],
                        "approved_digest": atmosphere_plan["approval_digest"],
                        "idempotency_key": "no-authority-atmosphere",
                    },
                )
                assert rejected.is_error and "unqualified" in str(rejected.content)
                receiver = copy.deepcopy(spectral)
                receiver.update(
                    wavelengths=atmosphere["wavelengths"],
                    absorptivity=[0.5] * 3,
                    ageing_action=[0.25] * 3,
                )
                receiver["source"] = {
                    "kind": "directional",
                    "propagation_direction": atmosphere_reference[
                        "propagation_direction"
                    ],
                    "irradiance": atmosphere["toa_irradiance"],
                }
                transfer = {
                    "schema_version": 1,
                    "source_job": "00000000-0000-0000-0000-000000000001",
                    "receiver": receiver,
                    "angular_mapping": "native_midpoint_solid_angle_quadrature",
                    "maximum_relative_conservation_error": 1e-10,
                }
                Draft202012Validator(schemas["AtmosphericTransferRequest"]).validate(
                    transfer
                )
                invalid = {**transfer, "maximum_relative_conservation_error": 0.01}
                cli = await asyncio.to_thread(
                    subprocess.run,
                    [
                        binary,
                        "--socket",
                        str(socket),
                        "results",
                        "transfer-atmosphere",
                        "/dev/stdin",
                    ],
                    input=json.dumps(invalid).encode(),
                    capture_output=True,
                    check=False,
                    timeout=30,
                )
                assert (
                    cli.returncode != 0
                    and json.loads(cli.stdout)["error"]["code"] == "invalid_input"
                )
                results_parameters = StdioServerParameters(
                    command=parameters.command,
                    args=["-m", "harbor_cad_mcp.server", "--profile", "results"],
                    env=dict(os.environ),
                )
                async with Client(results_parameters) as results_client:
                    rejected = await results_client.call_tool(
                        "results_transfer_atmosphere", {"request_spec": invalid}
                    )
                    assert rejected.is_error and "invalid_input" in str(
                        rejected.content
                    )
                    rejected = await results_client.call_tool(
                        "atmospheric_transport_plan", {"request_spec": invalid}
                    )
                    assert rejected.is_error and "invalid_input" in str(
                        rejected.content
                    )
                    rejected = await results_client.call_tool(
                        "atmospheric_transport_plan", {"request_spec": transfer}
                    )
                    assert rejected.is_error and "unqualified" in str(rejected.content)
                for request_spec, code in [
                    (invalid, "invalid_input"),
                    (transfer, "unqualified"),
                ]:
                    result = await asyncio.to_thread(
                        subprocess.run,
                        [
                            binary,
                            "--socket",
                            str(socket),
                            "results",
                            "plan-atmospheric-transport",
                            "/dev/stdin",
                        ],
                        input=json.dumps(request_spec).encode(),
                        capture_output=True,
                        check=False,
                        timeout=30,
                    )
                    assert (
                        result.returncode != 0
                        and json.loads(result.stdout)["error"]["code"] == code
                    )
                transport_plan = {
                    key: value
                    for key, value in atmosphere_plan["plan"].items()
                    if key != "atmosphere"
                }
                transport_plan.update(
                    schema_version=15,
                    atmospheric_transport={
                        "schema_version": 1,
                        "request": transfer,
                        "atmosphere": atmosphere,
                        "source": {
                            "job_id": transfer["source_job"],
                            "science_id": "a" * 64,
                            "execution_id": "b" * 64,
                            "execution_binding_digest": "c" * 64,
                            "authorization_digest": "d" * 64,
                            "receipt": {
                                "path": "stages/atmosphere/atmosphere-receipt.json",
                                "sha256": "e" * 64,
                                "bytes": 1,
                            },
                            "original": {
                                "path": "stages/atmosphere/native/atmosphere-original.txt",
                                "sha256": "f" * 64,
                                "bytes": 1,
                            },
                        },
                    },
                )
                validator = Draft202012Validator(schemas["ExecutionPlan"])
                validator.validate(transport_plan)
                for field in ("atmosphere", "spectral", "thermal", "source", "case"):
                    with pytest.raises(ValidationError):
                        validator.validate({**transport_plan, field: None})
                for version in range(1, 15):
                    with pytest.raises(ValidationError):
                        validator.validate(
                            {**transport_plan, "schema_version": version}
                        )
                with pytest.raises(ValidationError):
                    validator.validate(
                        {**atmosphere_plan["plan"], "atmospheric_transport": None}
                    )
                for field in ("radiance", "physical_time_s", "temperature_k"):
                    with pytest.raises(ValidationError):
                        Draft202012Validator(
                            schemas["AtmosphericTransferRequest"]
                        ).validate({**transfer, field: None})
                for field, value in [
                    ("diffuse_isotropic", True),
                    ("backend", "hip"),
                    ("relative_tolerance", 0.1),
                    ("profile", "../../credentials"),
                ]:
                    rejected = await client.call_tool(
                        "atmospheric_reference_validate",
                        {"spec": {**atmosphere, field: value}},
                    )
                    assert rejected.is_error and "invalid_input" in str(
                        rejected.content
                    )
                for field, value in (
                    ("reflectance", 1.1),
                    ("maximum_model_error", 0.02),
                    ("reflectance_provenance", ""),
                    ("temperature_k", 273.15),
                ):
                    rejected = await client.call_tool(
                        "spectral_reflection_reference_validate",
                        {"spec": {**reflection, field: value}},
                    )
                    assert rejected.is_error and "invalid_input" in str(
                        rejected.content
                    )
                for field, value in (
                    ("precision", "Float64"),
                    ("variant", "cuda_ad_spectral"),
                    ("absorptivity", [0.2, 1.1]),
                    ("history_interpolation", "measured_solar_history"),
                ):
                    rejected = await client.call_tool(
                        "spectral_reference_validate",
                        {"spec": {**spectral, field: value}},
                    )
                    assert rejected.is_error and "invalid_input" in str(
                        rejected.content
                    )
                rejected = await client.call_tool(
                    "snow_reference_validate",
                    {"spec": {**spec, "snow": {**spec["snow"], "coverage": "partial"}}},
                )
                assert rejected.is_error and "invalid_input" in str(rejected.content)
                assert "dry snow" in str(rejected.content)
                rejected = await client.call_tool(
                    "job_submit",
                    {
                        "plan": planned["plan"],
                        "approved_digest": planned["approval_digest"],
                        "idempotency_key": "snow-without-authority",
                    },
                )
                assert rejected.is_error and "unqualified" in str(rejected.content)
                assert "authoritative same-user admission" in str(rejected.content)

        asyncio.run(exercise())
    finally:
        worker.terminate()
        worker.wait(timeout=5)
