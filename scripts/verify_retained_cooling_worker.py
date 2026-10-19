"""Exact approved source-bound cooling originals, shared CLI/MCP and owned lifecycle."""

import argparse
import copy
import json
import math
import subprocess
import xml.etree.ElementTree as ET
from pathlib import Path

import jsonschema
from atmospheric_transport_prerequisites import immutable
from native_worker_campaign import WorkerCampaign
from verify_atmospheric_transport_worker import tree_identity
from verify_cad_spectral_worker import copy_closed_source_state
from verify_native_cpu import verify_manifest
from verify_openlb_hip import service_resources
from verify_retained_cooling_history import verifier
from verify_spectral_cpu import checksum
from verify_systemd import (
    admission_record,
    retention_snapshot,
    wait_admission_release,
    wait_retention_release,
)


def verify_vtk(module, spec, original, root):
    normalized = module.normalize(spec, original)
    nx, ny = spec["source_shape"]
    q = spec["spatial_refinement"]
    nx, height = nx * q, (ny - 2) * q
    reports = []
    for step in spec["observation_steps"]:
        csv = root / f"cooling-{step}.csv"
        rows = sorted(
            module.rows(csv.read_bytes(), module.COLUMNS),
            key=lambda r: (int(r["j"]), int(r["i"])),
        )
        vtk = root / f"cooling-{step}.vts"
        document = ET.fromstring(module.read_regular(vtk, 64 * 1024**2))
        assert document.attrib == {
            "type": "StructuredGrid",
            "version": "1.0",
            "byte_order": "LittleEndian",
        }
        grid = document.find("StructuredGrid")
        extent = f"0 {nx - 1} 0 {height - 1} 0 0"
        piece = grid.find("Piece")
        assert grid.attrib["WholeExtent"] == piece.attrib["Extent"] == extent
        assert not list(piece.find("CellData"))
        coordinates = piece.find("Points/DataArray")
        assert (
            coordinates.attrib["type"] == "Float64"
            and coordinates.attrib["unit"] == "m"
        )
        observed = [float(v) for v in coordinates.text.split()]
        expected = [
            v
            for row in rows
            for v in (
                float(row["x_m"]),
                float(row["y_m"]),
                spec["destination_origin_m"][2],
            )
        ]
        assert observed == expected
        arrays = {
            array.attrib["Name"]: array
            for array in piece.findall("PointData/DataArray")
        }
        names = [
            "i",
            "j",
            "parent_i",
            "parent_j",
            "water_fraction",
            "specific_enthalpy_j_kg",
            "temperature_k",
            "liquid_fraction",
        ]
        assert set(arrays) == set(names)
        for name, array in arrays.items():
            assert array.attrib["type"] == ("Int32" if name in names[:4] else "Float64")
            assert array.attrib["unit"] == {
                "temperature_k": "K",
                "specific_enthalpy_j_kg": "J/kg",
            }.get(name, "1")
            assert [float(v) for v in array.text.split()] == [
                float(row[name]) for row in rows
            ]
        fields = {a.attrib["Name"]: a for a in grid.findall("FieldData/DataArray")}
        assert set(fields) == {"physical_time_s", "extrusion_m", "subcontrol_volume_m3"}
        assert math.isclose(
            float(fields["physical_time_s"].text),
            step * normalized["physical_step_s"],
            rel_tol=5e-13,
            abs_tol=0,
        )
        assert float(fields["extrusion_m"].text) == spec["extrusion_m"]
        assert (
            float(fields["subcontrol_volume_m3"].text)
            == normalized["spacing_m"] ** 2 * spec["extrusion_m"]
        )
        reports.append(
            {
                "native_step": step,
                "original_csv_sha256": checksum(csv),
                "vtk_sha256": checksum(vtk),
                "all_original_float64_values_equal": True,
                "explicit_control_points_and_empty_cell_data": True,
            }
        )
    return reports


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in (
        "executable",
        "mcp",
        "runtime",
        "authority",
        "source",
        "native-reference",
        "output",
    ):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    binary, mcp, runtime = map(immutable, (args.executable, args.mcp, args.runtime))
    native = json.loads(runtime.read_text())
    reference = args.native_reference.resolve(strict=True)
    numerical = json.loads((reference / "verification.json").read_text())
    standalone = json.loads(immutable(numerical["runtime"]).read_text())
    if (
        checksum(Path(numerical["runtime"])) != numerical["runtime_sha256"]
        or native.get("retained_cooling") != standalone["retained_cooling"]
        or native.get("retained_cooling_closure")
        != standalone["retained_cooling_closure"]
        or numerical["package_qualification"] != "passed_scoped_cpu_native_cooling"
        or not numerical["spatial"]["passed"]
        or not numerical["temporal"]["passed"]
        or numerical["unqualified_coarse_spatial"]["passed"]
    ):
        raise ValueError(
            "exact separately conservation/analytic/history-qualified isolated cooling package required"
        )
    if any(
        native.get(k)
        for k in (
            "cad",
            "openlb",
            "render",
            "video",
            "filter",
            "thermal",
            "wetting",
            "freezing",
            "spectral",
            "fem",
        )
    ):
        raise ValueError("independent cooling-only native worker runtime required")
    for key in ("bwrap", "retained_cooling", "retained_cooling_closure"):
        immutable(native[key])
    source = args.source.resolve(strict=True)
    report = json.loads((source / "verification.json").read_text())
    sources = [
        r
        for r in report["results"]
        if r.get("interface") == "cli"
        and r["job"]["state"] == "succeeded"
        and r["job"]["exit_code"] == 0
        and r["receipt"]["request"]["resolution"] == 24
        and r["receipt"]["request"]["contact_angle_deg"] == 90.0
    ]
    if len(sources) != 1:
        raise ValueError(
            "one exact succeeded stationary original n24 native wetting source required"
        )
    source_job = sources[0]["job"]["id"]
    baseline = tree_identity(source)
    root = args.output.resolve()
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    (root / ".doty-protect").write_text(
        "Preserve exact native retained-cooling worker originals, source history, refusals and lifecycle failures.\n"
    )
    state = root / "state"
    state.mkdir(mode=0o700)
    copy_closed_source_state(source / "state", state)
    source_tree = state / "artifacts" / source_job
    original = source_tree / "stages/wetting/wetting-0.csv"
    source_identity = tree_identity(source_tree)
    original_bytes = original.read_bytes()
    candidates = [r for r in numerical["results"] if r["case"] == "retained-s2-t1"]
    if len(candidates) != 1 or checksum(original) != candidates[0]["source_sha256"]:
        raise ValueError(
            "exact conserved original worker phase must match separately qualified cooling source bytes"
        )
    module, _ = verifier()
    initialization = json.loads(
        (
            Path(__file__).resolve().parents[1] / "examples/retained-cooling.json"
        ).read_text()
    )
    initialization["retained"]["source_job"] = source_job
    initialization["retained"]["physical_time_s"] = 0.0
    request = {
        "schema_version": 1,
        "initialization": initialization,
        "spatial_refinement": 2,
        "integration_substeps": 1,
        "base_steps": 8192,
        "observation_base_steps": [0, 256, 1024, 8192],
    }
    results, rejections, lifecycle = [], [], []
    before = service_resources()
    with WorkerCampaign(
        binary, mcp, runtime, args.authority, root, timeout=900
    ) as campaign:
        schemas = campaign.command("schema")
        validator = jsonschema.Draft202012Validator(schemas["ExecutionPlan"])

        def plan(current, key):
            path = root / ("request-" + key + ".json")
            path.write_text(json.dumps(current, allow_nan=False))
            planned = campaign.command(
                "--socket", campaign.endpoint, "results", "plan-retained-cooling", path
            )["data"]
            assert planned == campaign.mcp_call(
                "retained_cooling_plan", {"request_spec": current}
            )
            validator.validate(planned["plan"])
            return planned

        for interface, sub in (("cli", 1), ("mcp", 2)):
            current = copy.deepcopy(request)
            current["integration_substeps"] = sub
            key = "retained-cooling-" + interface
            planned = plan(current, key)
            bound = planned["plan"]["retained_cooling"]
            assert (
                planned["plan"]["schema_version"] == 18
                and bound["prepared"]["executed"] is False
            )
            preparation = root / ("prepare-" + interface + ".json")
            preparation.write_text(json.dumps(initialization))
            prepared = campaign.command(
                "--socket",
                campaign.endpoint,
                "results",
                "prepare-retained-cooling",
                preparation,
            )["data"]
            assert (
                prepared
                == bound["prepared"]
                == campaign.mcp_call(
                    "results_prepare_retained_cooling",
                    {"request_spec": initialization},
                    profile="results",
                )
            )
            try:
                original.write_bytes(original_bytes + b"changed before acknowledgment")
                assert not campaign.submit(
                    planned,
                    "retained-cooling-reject-source-" + interface,
                    allow_error=True,
                )["ok"]
                campaign.mcp_call(
                    "retained_cooling_plan",
                    {"request_spec": current},
                    expect_error=True,
                )
                rejections.append(
                    {"source_mutation_before_ack": interface, "rejected": True}
                )
            finally:
                original.write_bytes(original_bytes)
            if interface == "mcp":
                job = campaign.mcp_call(
                    "job_submit",
                    {
                        "plan": planned["plan"],
                        "approved_digest": planned["approval_digest"],
                        "idempotency_key": key,
                    },
                )
                campaign.owned.append(job["unit"])
            else:
                job = campaign.submit(planned, key)
            owned = campaign.wait(job, {"running"})
            active = retention_snapshot(state, job, str(binary))
            reservation = admission_record(state, job)
            assert (
                reservation is not None
                and active["intent"]["binding"]["sandbox_policy"]
                == "harbor-cad-retained-cooling-cpu-v1"
            )
            retained_root = state / "artifacts" / job["id"]
            for record in bound["originals"]:
                old = source_tree / record["path"]
                retained = retained_root / "source-wetting" / record["path"]
                assert (
                    retained.read_bytes() == old.read_bytes()
                    and retained.stat().st_ino != old.stat().st_ino
                    and checksum(retained) == record["sha256"]
                )
            (root / ("request-" + key + ".json")).write_text("{}")
            campaign.restart()
            assert campaign.submit(planned, key)["id"] == job["id"]
            outcome = campaign.wait(job, {"succeeded"})
            assert outcome["invocation_id"] == owned["invocation_id"]
            bundle = root / ("bundle-" + interface)
            campaign.command("artifact", "export", "--state", state, job["id"], bundle)
            manifests = verify_manifest(bundle)
            work = bundle / "stages/retained-cooling"
            receipt = json.loads((work / "retained-cooling-receipt.json").read_text())
            envelope = json.loads(
                (bundle / "native-retained-cooling-request.json").read_text()
            )
            assert receipt["request_sha256"] == checksum(
                bundle / "native-retained-cooling-request.json"
            )
            assert (
                receipt["sandbox"]["policy"] == "harbor-cad-retained-cooling-cpu-v1"
                and len(receipt["sandbox"]["checks"]) == 9
                and all(receipt["sandbox"]["checks"].values())
            )
            reconstructed = module.verify(
                envelope["native_request"],
                original_bytes,
                receipt,
                work,
                envelope["maximum_relative_conservation_error"],
            )
            assert reconstructed == receipt["independent_verification"]
            portable = verify_vtk(
                module, envelope["native_request"], original_bytes, work
            )
            fields = [
                m
                for m in manifests
                if m["path"].startswith("stages/retained-cooling/cooling-")
                and m["format"] == "csv"
            ]
            assert len(fields) == 4 and all(
                m["association"] == "native_original_parent_congruent_control"
                and "temperature_k:K" in m["units"]
                for m in fields
            )
            qualification = campaign.command(
                # Qualification independently reconstructs published VTK views.
                "--socket",
                campaign.endpoint,
                "qualify",
                "--job",
                job["id"],
            )["data"]
            assert qualification == campaign.mcp_call(
                "qualification_report", {"job_id": job["id"]}, profile="results"
            )
            assert (
                qualification["capabilities"][0]["runtime_execution"] == "recorded"
                and qualification["capabilities"][0]["numerical_verification"]
                == "reported_pass"
                and qualification["physical_validation"] == "unqualified"
            )
            samples = []
            for step, time in zip(
                current["observation_base_steps"],
                planned["plan"]["observation"]["retained_times_s"],
            ):
                query = {
                    "job_id": job["id"],
                    "physical_time_s": time,
                    "region": initialization["retained"]["destination_region"],
                    "points": [[0, 1], [60, 36], [121, 70]],
                }
                path = root / (f"query-{interface}-{step}.json")
                path.write_text(json.dumps(query))
                observed = campaign.command(
                    "--socket",
                    campaign.endpoint,
                    "results",
                    "sample-retained-cooling",
                    path,
                )["data"]
                assert observed == campaign.mcp_call(
                    "results_sample_retained_cooling",
                    {"request_spec": query},
                    profile="results",
                )
                assert (
                    observed["source"] == bound
                    and observed["portable_field"] in manifests
                    and len(observed["complete_history"]) == 4
                    and observed["physical_validation"] == "unqualified"
                )
                native_step = step * 4 * sub
                rows = {
                    (int(r["i"]), int(r["j"])): r
                    for r in module.rows(
                        (work / f"cooling-{native_step}.csv").read_bytes(),
                        module.COLUMNS,
                    )
                }
                for point in observed["values"]:
                    native_row = rows[point["i"], point["j"]]
                    for field in (
                        "water_fraction",
                        "specific_enthalpy_j_kg",
                        "temperature_k",
                        "liquid_fraction",
                    ):
                        assert point[field] == float(native_row[field])
                samples.append(observed)
            for changed in (
                {**query, "region": "other"},
                {**query, "physical_time_s": query["physical_time_s"] + 1e-9},
                {**query, "points": [[999, 999]]},
            ):
                campaign.mcp_call(
                    "results_sample_retained_cooling",
                    {"request_spec": changed},
                    profile="results",
                    expect_error=True,
                )
                rejections.append({"query_mutation": changed, "rejected": True})
            for path in (
                retained_root
                / "source-wetting"
                / bound["prepared"]["retained"]["original_field"]["path"],
                retained_root / "stages/retained-cooling/cooling-0.csv",
                retained_root / "stages/retained-cooling/heat-exchange.csv",
                retained_root / "stages/retained-cooling/cooling-0.vts",
            ):
                saved = path.read_bytes()
                try:
                    path.write_bytes(saved + b"changed")
                    assert not campaign.command(
                        "--socket",
                        campaign.endpoint,
                        "qualify",
                        "--job",
                        job["id"],
                        allow_error=True,
                    )["ok"]
                    campaign.mcp_call(
                        "results_sample_retained_cooling",
                        {"request_spec": query},
                        profile="results",
                        expect_error=True,
                    )
                finally:
                    path.write_bytes(saved)
            resources = json.loads((bundle / "service-resources.json").read_text())
            controls = json.loads((bundle / "service-owner.json").read_text())[
                "kernel_resources"
            ]["controls"]
            assert (
                controls["cpu.max"] == "200000 100000"
                and controls["memory.swap.max"] == "0"
                and controls["pids.max"] == "128"
            )
            assert (
                int(controls["memory.max"])
                == math.ceil(planned["plan"]["stages"][0]["ram_bytes"] / 4096) * 4096
                and resources["kernel_resources"]["aggregate_memory_peak_bytes"] > 0
            )
            wait_admission_release(state, job)
            wait_retention_release(state, job)
            assert tree_identity(source_tree) == source_identity
            results.append(
                {
                    "interface": interface,
                    "job": outcome,
                    "source": bound,
                    "queries": samples,
                    "independent": reconstructed,
                    "lossless_vtk": portable,
                    "qualification": qualification,
                    "resources": resources,
                    "roots_and_reservation_released": True,
                    "source_unchanged": True,
                    "same_invocation_after_restart": True,
                    "manifest_sha256": checksum(bundle / "manifest.json"),
                }
            )
        for field, value in (
            ("freezing", None),
            ("wetting", None),
            ("schema_version", 12),
        ):
            altered = copy.deepcopy(planned)
            altered["plan"][field] = value
            assert not validator.is_valid(altered["plan"])
            assert not campaign.submit(
                altered, "retained-cooling-reject-" + field, allow_error=True
            )["ok"]
            rejections.append({"plan_mutation": field, "rejected": True})
        assert not campaign.submit(
            planned,
            "retained-cooling-reject-approval",
            approval="0" * 64,
            allow_error=True,
        )["ok"]
        rejections.append({"approval_drift": True, "rejected": True})
        for name, target in (
            ("cancel", "cancelled"),
            ("service-death", "failed"),
            ("source-loss", "failed"),
        ):
            slow = copy.deepcopy(request)
            slow["integration_substeps"] = 4
            planned = plan(slow, "lifecycle-" + name)
            key = "retained-cooling-" + name
            job = campaign.submit(planned, key)
            campaign.wait(job, {"running"})
            active = retention_snapshot(state, job, str(binary))
            if name == "cancel":
                campaign.command(
                    "--socket", campaign.endpoint, "job", "cancel", job["id"]
                )
            elif name == "service-death":
                campaign.worker.kill()
                campaign.worker.wait(timeout=5)
                subprocess.run(
                    [
                        "systemctl",
                        "--user",
                        "kill",
                        "--signal=KILL",
                        "--kill-whom=all",
                        job["unit"],
                    ],
                    env=campaign.environment,
                    check=True,
                    timeout=30,
                )
                campaign.start()
            else:
                saved = root / "temporarily-unavailable-source.csv"
                original.rename(saved)
            try:
                outcome = campaign.wait(job, {target})
            finally:
                if name == "source-loss":
                    saved.rename(original)
            assert campaign.submit(planned, key)["id"] == job["id"]
            bundle = root / ("bundle-" + name)
            campaign.command("artifact", "export", "--state", state, job["id"], bundle)
            verify_manifest(bundle)
            wait_admission_release(state, job)
            wait_retention_release(state, job)
            lifecycle.append(
                {
                    "kind": name,
                    "job": outcome,
                    "active_retention": active,
                    "roots_and_reservation_released": True,
                    "manifest_sha256": checksum(bundle / "manifest.json"),
                }
            )
    assert tree_identity(source) == baseline
    result = {
        "schema_version": 1,
        "binary": str(binary),
        "binary_sha256": checksum(binary),
        "mcp": str(mcp),
        "mcp_sha256": checksum(mcp),
        "runtime": str(runtime),
        "runtime_sha256": checksum(runtime),
        "source_reference": str(source),
        "source_report_sha256": checksum(source / "verification.json"),
        "native_reference_sha256": checksum(reference / "verification.json"),
        "results": results,
        "rejections": rejections,
        "lifecycle": lifecycle,
        "original_source_tree_unchanged": True,
        "service_resources_before": before,
        "service_resources_after": service_resources(),
        "scope": "synthetic stationary original-parent retained-water native CPU conduction; fixed conservation gates and separately qualified analytic/spatial/temporal references; no physical lifetime",
        "physical_validation": "unqualified",
    }
    (root / "verification.json").write_text(
        json.dumps(result, indent=2, allow_nan=False)
    )
    print(
        json.dumps(
            {
                "results": len(results),
                "rejections": len(rejections),
                "lifecycle": len(lifecycle),
                "verification_sha256": checksum(root / "verification.json"),
            }
        )
    )


if __name__ == "__main__":
    main()
