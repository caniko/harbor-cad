import asyncio
import json
import os
import subprocess
import time
from pathlib import Path

import pytest
from harbor_cad_mcp.server import build_server
from jsonschema import Draft202012Validator, ValidationError


def binary() -> str:
    return os.environ["HARBOR_CAD_TEST_BINARY"]


def cold_inputs():
    missing = {
        "availability": "missing",
        "reason": "physical input unavailable in this synthetic contract fixture",
    }
    return {
        "schema_version": 1,
        "synthetic": True,
        "geometry_sha256": "a" * 64,
        "thermal_region": "enclosure",
        "material": {
            key: dict(missing) for key in ("density", "conductivity", "specific_heat")
        },
        "minimum_valid_temperature": {"value": 200, "unit": "K"},
        "maximum_valid_temperature": {"value": 400, "unit": "K"},
        "initial_temperature": {"value": -40, "unit": "degC"},
        "duration": {"value": 1, "unit": "h"},
        "history_interpolation": "piecewise_linear",
        "ambient_history": dict(missing),
        "heater_history": dict(missing),
        "convection_coefficient": dict(missing),
        "observation_times": [{"value": 0, "unit": "s"}, {"value": 1, "unit": "h"}],
        "numerical_tolerance": 0.01,
        "moisture": {
            "assessment": "missing",
            "reason": "humidity and surface history missing",
        },
    }


def test_schema_parity_and_unknown_input_rejection():
    schemas = json.loads(subprocess.check_output([binary(), "schema"]))
    case = json.loads(subprocess.check_output([binary(), "case", "init"]))
    Draft202012Validator(schemas["CaseSpec"]).validate(case)
    cold = cold_inputs()
    Draft202012Validator(schemas["ColdRestartSpec"]).validate(cold)
    cold["heater_history"]["value"] = []
    with pytest.raises(ValidationError):
        Draft202012Validator(schemas["ColdRestartSpec"]).validate(cold)
    planned = json.loads(
        subprocess.check_output(
            [binary(), "case", "plan", "/dev/stdin"], input=json.dumps(case).encode()
        )
    )["plan"]
    Draft202012Validator(schemas["ExecutionPlan"]).validate(planned)
    injected = dict(planned, source=None)
    with pytest.raises(ValidationError):
        Draft202012Validator(schemas["ExecutionPlan"]).validate(injected)
    with pytest.raises(ValidationError):
        Draft202012Validator(schemas["ExecutionPlan"]).validate(
            dict(planned, cad_source=None)
        )
    injected_filter = dict(planned, filter={"time_s": 0, "field": "velocity"})
    with pytest.raises(ValidationError):
        Draft202012Validator(schemas["ExecutionPlan"]).validate(injected_filter)
    missing_filter = dict(planned, schema_version=4)
    with pytest.raises(ValidationError):
        Draft202012Validator(schemas["ExecutionPlan"]).validate(missing_filter)
    incomplete_v2 = dict(planned, schema_version=2)
    with pytest.raises(ValidationError):
        Draft202012Validator(schemas["ExecutionPlan"]).validate(incomplete_v2)
    case["execute_python"] = "print('no')"
    with pytest.raises(ValidationError):
        Draft202012Validator(schemas["CaseSpec"]).validate(case)


def test_static_result_schema_preserves_native_associations_and_rejects_interpolation():
    schemas = json.loads(subprocess.check_output([binary(), "schema"]))
    sample = {
        "schema_version": 1,
        "job_id": "a05f78ac-a7ce-4aed-a458-e4a4cbf0b9fc",
        "field": "heat_flux",
        "locations": [
            {
                "association": "integration_point",
                "element_id": 7,
                "integration_point": 2,
            }
        ],
    }
    Draft202012Validator(schemas["SampleRequest"]).validate(sample)
    Draft202012Validator(schemas["CompareRequest"]).validate(
        {"schema_version": 1, "left": sample, "right": sample}
    )
    for changed in (
        {**sample, "time_s": 1},
        {**sample, "artifact": "../../private.json"},
        {**sample, "field": "velocity"},
        {**sample, "locations": [{"association": "point", "coordinate_m": [0, 0, 0]}]},
        {
            **sample,
            "locations": [{"association": "node", "node_id": 1, "element_id": 7}],
        },
    ):
        with pytest.raises(ValidationError):
            Draft202012Validator(schemas["SampleRequest"]).validate(changed)


def test_thermal_result_schema_requires_explicit_time_and_native_node_association():
    schemas = json.loads(subprocess.check_output([binary(), "schema"]))
    sample = {
        "schema_version": 1,
        "job_id": "a05f78ac-a7ce-4aed-a458-e4a4cbf0b9fc",
        "field": "temperature",
        "physical_time_s": 60.0,
        "locations": [{"association": "node", "node_id": 1}],
    }
    Draft202012Validator(schemas["ThermalSampleRequest"]).validate(sample)
    Draft202012Validator(schemas["ThermalCompareRequest"]).validate(
        {
            "schema_version": 1,
            "left": sample,
            "right": {**sample, "physical_time_s": 120.0},
        }
    )
    for changed in (
        {k: v for k, v in sample.items() if k != "physical_time_s"},
        {**sample, "physical_time_s": None},
        {**sample, "field": "heat_flux"},
        {**sample, "interpolation": "linear"},
        {**sample, "locations": [{"association": "point", "coordinate_m": [0, 0, 0]}]},
    ):
        with pytest.raises(ValidationError):
            Draft202012Validator(schemas["ThermalSampleRequest"]).validate(changed)


def test_native_moisture_schema_rejects_caller_surface_temperatures_and_unknown_air():
    schemas = json.loads(subprocess.check_output([binary(), "schema"]))
    request = {
        "schema_version": 1,
        "job_id": "a05f78ac-a7ce-4aed-a458-e4a4cbf0b9fc",
        "physical_time_s": 60.0,
        "surface_region": "xmin",
        "moisture_risk": {
            "assessment": "dew_point_screening",
            "air_temperature": {"value": 20.0, "unit": "degC"},
            "relative_humidity": 0.5,
            "provenance": "synthetic air fixture",
        },
    }
    validator = Draft202012Validator(schemas["NativeMoistureRequest"])
    validator.validate(request)
    for assessment in (
        {"assessment": "missing", "reason": "humidity unavailable"},
        {"assessment": "inapplicable", "justification": "explicit dry reference"},
    ):
        validator.validate({**request, "moisture_risk": assessment})
    for changed in (
        {**request, "minimum_surface_temperature": {"value": 20.0, "unit": "degC"}},
        {**request, "surface_region": "face1"},
        {
            **request,
            "moisture_risk": {**request["moisture_risk"], "relative_humidity": None},
        },
    ):
        with pytest.raises(ValidationError):
            validator.validate(changed)


def test_wetting_si_descriptor_uses_generated_rust_schema_with_explicit_provenance():
    from test_wetting_reference import request

    schemas = json.loads(subprocess.check_output([binary(), "schema"]))
    spec = request()
    Draft202012Validator(schemas["WettingReferenceSpec"]).validate(spec)
    plan = json.loads(
        subprocess.check_output(
            [binary(), "case", "plan-wetting-reference", "/dev/stdin"],
            input=json.dumps(spec).encode(),
        )
    )["plan"]
    Draft202012Validator(schemas["ExecutionPlan"]).validate(plan)
    assert plan["schema_version"] == 9
    for injected in (spec, None):
        old = json.loads(
            subprocess.check_output(
                [binary(), "case", "plan", "/dev/stdin"],
                input=subprocess.check_output([binary(), "case", "init"]),
            )
        )
        old["wetting"] = injected
        with pytest.raises(ValidationError):
            Draft202012Validator(schemas["ExecutionPlan"]).validate(old)
    for changed in (
        {**spec, "evaporation": True},
        {**spec, "material_provenance": None},
        {**spec, "observation_steps": [0, True, spec["steps"]]},
    ):
        with pytest.raises(ValidationError):
            Draft202012Validator(schemas["WettingReferenceSpec"]).validate(changed)


def test_real_mcp_client_and_rust_worker(tmp_path, monkeypatch):
    from mcp import Client
    from mcp.client.stdio import StdioServerParameters

    root = tmp_path / "state"
    profile = Path(__file__).parents[2] / "profiles" / "ci.json"
    worker = subprocess.Popen(
        [binary(), "worker", "--state", str(root), "--profile", str(profile.resolve())]
    )
    socket = root / "worker.sock"
    monkeypatch.setenv("HARBOR_CAD_SOCKET", str(socket))
    try:
        deadline = time.monotonic() + 5
        while not socket.exists():
            assert time.monotonic() < deadline
            time.sleep(0.01)
        case = json.loads(subprocess.check_output([binary(), "case", "init"]))
        schemas = json.loads(subprocess.check_output([binary(), "schema"]))
        result = json.loads(
            subprocess.check_output(
                [binary(), "case", "plan", "/dev/stdin"],
                input=json.dumps(case).encode(),
            )
        )

        expected_cold = json.loads(
            subprocess.check_output(
                [binary(), "case", "validate-cold-restart", "/dev/stdin"],
                input=json.dumps(cold_inputs()).encode(),
            )
        )
        freezing = json.loads(
            (Path(__file__).parents[2] / "examples/freezing-reference.json").read_text()
        )
        Draft202012Validator(schemas["FreezingReferenceSpec"]).validate(freezing)
        expected_freezing = json.loads(
            subprocess.check_output(
                [binary(), "case", "validate-freezing-reference", "/dev/stdin"],
                input=json.dumps(freezing).encode(),
            )
        )
        Draft202012Validator(schemas["FreezingScale"]).validate(
            expected_freezing["scale"]
        )
        assert (
            not expected_freezing["executed"]
            and expected_freezing["moisture_risk"]["status"] == "missing_inputs"
        )

        async def check():
            params = StdioServerParameters(
                command=str(Path(os.sys.executable)),
                args=["-m", "harbor_cad_mcp.server", "--profile", "all"],
                env=dict(os.environ),
            )
            async with Client(params) as client:
                names = {t.name for t in (await client.list_tools()).tools}
                assert "job_submit" in names and "results_describe" in names
                assert {"results_sample", "results_compare"}.issubset(names)
                assert {"results_sample_thermal", "results_compare_thermal"}.issubset(
                    names
                )
                assert {"results_sample_freezing", "results_compare_freezing"}.issubset(
                    names
                )
                freezing_query = {
                    "schema_version": 1,
                    "job_id": "a05f78ac-a7ce-4aed-a458-e4a4cbf0b9fc",
                    "field": "temperature",
                    "physical_time_s": 0.0,
                    "points": [[0, 0], [3, 1]],
                }
                Draft202012Validator(schemas["FreezingSampleRequest"]).validate(
                    freezing_query
                )
                Draft202012Validator(schemas["FreezingCompareRequest"]).validate(
                    {
                        "schema_version": 1,
                        "left": freezing_query,
                        "right": freezing_query,
                    }
                )
                for tool, query in (
                    (
                        "results_sample_freezing",
                        {**freezing_query, "points": [[0, 0], [0, 0]]},
                    ),
                    (
                        "results_compare_freezing",
                        {
                            "schema_version": 1,
                            "left": freezing_query,
                            "right": {**freezing_query, "physical_time_s": 1.0},
                        },
                    ),
                ):
                    rejected = await client.call_tool(tool, {"request_spec": query})
                    assert rejected.is_error and "invalid_input" in str(
                        rejected.content
                    )
                assert "results_moisture" in names
                assert "results_retain_wetting" in names
                retention = {
                    "schema_version": 1,
                    "source_job": "a05f78ac-a7ce-4aed-a458-e4a4cbf0b9fc",
                    "physical_time_s": 0.0,
                    "extrusion": {"value": 1.0, "unit": "mm"},
                    "extrusion_provenance": "explicit synthetic reference depth",
                    "destination_region": "retained_phase",
                    "destination_origin_m": [0.0, 0.0, 0.0],
                    "maximum_relative_conservation_error": 1e-10,
                }
                Draft202012Validator(schemas["WettingRetentionRequest"]).validate(
                    retention
                )
                with pytest.raises(ValidationError):
                    Draft202012Validator(schemas["WettingRetentionRequest"]).validate(
                        {**retention, "temperature_k": 273.15}
                    )
                rejected = await client.call_tool(
                    "results_retain_wetting",
                    {
                        "request_spec": {
                            **retention,
                            "extrusion": {"value": 0.0, "unit": "m"},
                        }
                    },
                )
                assert rejected.is_error and "invalid_input" in str(rejected.content)
                rejected_sample = await client.call_tool(
                    "results_sample_thermal",
                    {
                        "request_spec": {
                            "schema_version": 1,
                            "job_id": "a05f78ac-a7ce-4aed-a458-e4a4cbf0b9fc",
                            "field": "temperature",
                            "physical_time_s": 60.0,
                            "locations": [{"association": "node", "node_id": 1}],
                        }
                    },
                )
                assert rejected_sample.is_error
                assert "case_plan_openlb_reference" in names
                assert "cad_plan_inspection" in names
                assert "cad_regions" in names and "cad_submit" in names
                assert "cad_plan_variant" in names
                assert "cad_plan_mesh" in names and "cad_mesh_submit" in names
                rejected_mesh = await client.call_tool(
                    "cad_plan_mesh",
                    {
                        "request_spec": {
                            "source_job": "a05f78ac-a7ce-4aed-a458-e4a4cbf0b9fc",
                            "region_name": "solid",
                            "resolution": 4,
                            "geometry_tolerance_m": 1e-6,
                        }
                    },
                )
                assert rejected_mesh.is_error
                assert "cold_restart_validate" in names
                assert "filter_plan" in names
                assert "case_plan_fem_reference" in names
                assert "case_plan_thermal_reference" in names
                assert "case_plan_contact_reference" in names
                assert "case_plan_thermal_contact" in names
                assert "freezing_reference_validate" in names
                assert "case_plan_freezing_reference" in names
                frozen = await client.call_tool(
                    "freezing_reference_validate", {"spec": freezing}
                )
                assert (
                    not frozen.is_error
                    and frozen.structured_content == expected_freezing
                )
                rejected_freezing = await client.call_tool(
                    "freezing_reference_validate",
                    {"spec": {**freezing, "energy_tolerance": 0.02}},
                )
                assert rejected_freezing.is_error
                assert "invalid_input" in str(rejected_freezing.content)
                rejected_native_plan = await client.call_tool(
                    "case_plan_freezing_reference", {"spec": freezing}
                )
                assert rejected_native_plan.is_error
                assert "invalid_input" in str(rejected_native_plan.content)
                from test_thermal_contact import fixture as coupling_fixture

                coupling = await client.call_tool(
                    "case_plan_thermal_contact", {"spec": coupling_fixture()}
                )
                assert coupling.is_error
                from test_contact_reference import request as contact_fixture
                from test_thermal_history import fixture

                rejected_thermal = await client.call_tool(
                    "case_plan_thermal_reference", {"spec": fixture()}
                )
                assert rejected_thermal.is_error
                rejected_contact = await client.call_tool(
                    "case_plan_contact_reference", {"spec": contact_fixture()}
                )
                assert rejected_contact.is_error
                cold = cold_inputs()
                checked = await client.call_tool(
                    "cold_restart_validate", {"case": cold}
                )
                assert not checked.is_error, checked
                report = checked.structured_content
                assert report["inputs_complete"] is False
                assert report["prescribed_heater_energy_j"] is None
                assert report["execution"] == "not_requested"
                assert report["physical_validation"] == "unqualified"
                assert report == expected_cold
                assert not any(x in names for x in ("shell", "python", "install"))
                # CI cannot implicitly turn a reference into an unsandboxed
                # native job. Planning rejection is returned by the same worker.
                native_case = dict(case)
                native_case["applicability"] = dict(
                    case["applicability"], formulation="periodic_forced_channel"
                )
                rejected = await client.call_tool(
                    "case_plan_openlb_reference", {"case": native_case}
                )
                assert rejected.is_error
                inspected = dict(
                    case,
                    geometry={
                        "source": "approved.FCStd",
                        "sha256": "a" * 64,
                        "synthetic": True,
                    },
                )
                rejected = await client.call_tool(
                    "cad_plan_inspection", {"case": inspected}
                )
                assert rejected.is_error
                response = await client.call_tool(
                    "job_submit",
                    {
                        "plan": result["plan"],
                        "approved_digest": result["approval_digest"],
                        "idempotency_key": "real-mcp-client",
                    },
                )
                job = response.structured_content
                assert job and job["id"], response
                deadline = time.monotonic() + 10
                while True:
                    status = (
                        await client.call_tool("job_status", {"job_id": job["id"]})
                    ).structured_content
                    if status["state"] == "succeeded":
                        break
                    assert status["state"] != "failed", status
                    assert time.monotonic() < deadline
                    await asyncio.sleep(0.02)
                cli_logs = json.loads(
                    await asyncio.to_thread(
                        subprocess.check_output,
                        [binary(), "job", "logs", job["id"], "--limit", "100"],
                        timeout=10,
                    )
                )["data"]
                assert cli_logs
                for profile in ("cad", "simulation", "results", "all"):
                    profile_params = StdioServerParameters(
                        command=os.sys.executable,
                        args=["-m", "harbor_cad_mcp.server", "--profile", profile],
                        env=dict(os.environ),
                    )
                    async with Client(profile_params) as reader:
                        tools = {t.name: t for t in (await reader.list_tools()).tools}
                        schema = tools["job_logs"].output_schema
                        assert schema
                        for after, limit, expected in (
                            (0, 100, cli_logs),
                            (0, 1, cli_logs[:1]),
                            (cli_logs[0]["seq"], 1, cli_logs[1:2]),
                            (cli_logs[-1]["seq"], 100, []),
                        ):
                            logs = await reader.call_tool(
                                "job_logs",
                                {"job_id": job["id"], "after": after, "limit": limit},
                            )
                            assert not logs.is_error
                            Draft202012Validator(schema).validate(
                                logs.structured_content
                            )
                            assert logs.structured_content == {"result": expected}
                            assert [
                                json.loads(item.text) for item in logs.content
                            ] == expected
                        for invalid in (
                            {"limit": 0},
                            {"limit": 101},
                            {"after": -1},
                            {"after": 1 << 63},
                        ):
                            refused = await reader.call_tool(
                                "job_logs", {"job_id": job["id"], **invalid}
                            )
                            assert refused.is_error and "invalid_input:" in str(
                                refused.content
                            )
                duplicate = (
                    await client.call_tool(
                        "job_submit",
                        {
                            "plan": result["plan"],
                            "approved_digest": result["approval_digest"],
                            "idempotency_key": "real-mcp-client",
                        },
                    )
                ).structured_content
                assert duplicate["id"] == job["id"]
                description = (
                    await client.call_tool("results_describe", {"job_id": job["id"]})
                ).structured_content
                assert "velocity_m_s" not in json.dumps(description)
                assert len(description["artifacts"]["items"]) == 5
                rejected = await client.call_tool(
                    "results_retain_wetting",
                    {"request_spec": {**retention, "source_job": job["id"]}},
                )
                assert rejected.is_error and "unqualified" in str(rejected.content)
                assert "native wetting source" in str(rejected.content)
                sampling = {
                    "schema_version": 1,
                    "job_id": job["id"],
                    "field": "temperature",
                    "locations": [{"association": "node", "node_id": 1}],
                }
                rejected = await client.call_tool(
                    "results_sample", {"request_spec": sampling}
                )
                assert rejected.is_error
                rejected = await client.call_tool(
                    "results_compare",
                    {
                        "request_spec": {
                            "schema_version": 1,
                            "left": sampling,
                            "right": sampling,
                        }
                    },
                )
                assert rejected.is_error
                Draft202012Validator(schemas["ExecutionBinding"]).validate(
                    description["execution_binding"]
                )
                assert description["artifacts"]["next_after"] is None
                evidence = (
                    await client.call_tool(
                        "qualification_report", {"job_id": job["id"]}
                    )
                ).structured_content
                Draft202012Validator(schemas["JobEvidenceReport"]).validate(evidence)
                assert evidence["execution_id"] == result["approval_digest"]
                assert evidence["scope"] == "historical_job_evidence"
                assert evidence["current_runtime_qualification"] == "not_assessed"
                assert evidence["physical_validation"] == "unqualified"
                assert evidence["capabilities"][0]["runtime_execution"] == "recorded"
                assert (
                    evidence["capabilities"][0]["numerical_verification"]
                    == "reported_pass"
                )
                artifacts = await client.call_tool(
                    "artifact_list", {"job_id": job["id"], "limit": 1}
                )
                page = artifacts.structured_content
                assert len(page["items"]) == 1 and page["total"] == 5
                remaining = await client.call_tool(
                    "artifact_list", {"job_id": job["id"], "after": page["next_after"]}
                )
                assert len(remaining.structured_content["items"]) == 4
                assert remaining.structured_content["next_after"] is None
            async with Client(build_server("results")) as client:
                names = {t.name for t in (await client.list_tools()).tools}
                assert "job_submit" not in names and "results_describe" in names
                assert "qualification_report" in names
                assert {"results_sample", "results_compare"}.issubset(names)
                assert "cad_plan_inspection" not in names
                assert "cad_regions" in names and "cad_submit" not in names
                assert "cold_restart_validate" not in names
                assert "filter_plan" not in names
                assert "case_plan_fem_reference" not in names
                assert "case_plan_thermal_reference" not in names
                assert "case_plan_contact_reference" not in names
                assert "case_plan_thermal_contact" not in names
                assert "results_transfer_temperature" in names
                assert "results_retain_wetting" in names
                assert {"render_plan", "video_plan", "presentation_submit"}.issubset(
                    names
                )
                rejected = await client.call_tool(
                    "presentation_submit",
                    {
                        "plan": result["plan"],
                        "approved_digest": result["approval_digest"],
                        "idempotency_key": "no-simulation-in-results",
                    },
                )
                assert rejected.is_error
            async with Client(build_server("cad")) as client:
                names = {t.name for t in (await client.list_tools()).tools}
                assert "cad_plan_inspection" in names and "job_submit" not in names
                assert "cad_regions" in names and "cad_submit" in names
                assert "cad_plan_variant" in names
                assert "job_status" in names and "job_logs" in names
                assert "case_plan_fem_reference" not in names
                assert "case_plan_thermal_reference" not in names
                assert "case_plan_contact_reference" not in names
                assert "case_plan_thermal_contact" not in names
                rejected = await client.call_tool("cad_regions", {"job_id": job["id"]})
                assert rejected.is_error
                variant = json.loads(
                    (
                        Path(__file__).parents[2] / "examples/cad-variant.json"
                    ).read_text()
                )
                rejected = await client.call_tool(
                    "cad_plan_variant", {"variant": variant}
                )
                assert rejected.is_error and "unqualified:" in str(rejected.content)
                rejected = await client.call_tool(
                    "cad_submit",
                    {
                        "plan": result["plan"],
                        "approved_digest": result["approval_digest"],
                        "idempotency_key": "wrong-cad-operation",
                    },
                )
                assert rejected.is_error

        asyncio.run(check())
    finally:
        worker.terminate()
        worker.wait(timeout=5)
