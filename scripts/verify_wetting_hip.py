"""Qualify packaged Float64 HIP wetting against the packaged CPU driver.

Runs both native drivers directly inside an independently bounded user-service
fixture on the same synthetic descriptor. A passing report is numerical/device
evidence only: worker KFD containment, B1 and physical validation stay separate.
"""

import argparse
import csv
import hashlib
import importlib.util
import io
import json
import math
import subprocess
from pathlib import Path

from verify_openlb_hip import service_resources
from verify_wetting_cpu import request

SOURCE = "145cd54810b468f4b6fd3ed86b10644264841578"
# Same Float64 criterion as the airflow HIP qualification. Phase drives every
# scientific gate; parasitic lattice velocity is near zero, so its relative norm
# is ill-conditioned and retained as a diagnostic, like airflow pressure.
TOLERANCE = 1e-10


def bridge():
    path = Path(__file__).resolve().parents[1] / "adapters/wetting_reference.py"
    spec = importlib.util.spec_from_file_location("wetting", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run(executable, root, label, descriptor, env, timeout=1250):
    inputs, output = root / f"inputs-{label}", root / label
    inputs.mkdir()
    output.mkdir()
    path = inputs / "request.json"
    path.write_text(json.dumps(descriptor, allow_nan=False, indent=2))
    with (output / "process.log").open("xb") as log:
        process = subprocess.run(
            [executable, "reference", str(path)],
            cwd=output,
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            timeout=timeout,
            check=False,
        )
    return process.returncode, output


def rows(path):
    reader = csv.reader(io.StringIO(path.read_text()))
    if next(reader) != ["x_m", "y_m", "material", "phi", "u_lattice", "v_lattice"]:
        raise ValueError("exact wetting field columns required")
    return list(reader)


def l2(cpu, hip):
    difference = math.sqrt(sum((a - b) ** 2 for a, b in zip(cpu, hip, strict=True)))
    norm = math.sqrt(sum(a * a for a in cpu))
    return difference, difference / norm if norm else difference


def compare(cpu_root, hip_root, steps):
    agreement = []
    for step in steps:
        cpu, hip = (rows(r / f"wetting-{step}.csv") for r in (cpu_root, hip_root))
        if not cpu or len(cpu) != len(hip):
            raise ValueError(f"CPU/HIP node count differs at step {step}")
        # Coordinates and material IDs are geometry, not arithmetic: exact text.
        if any(c[:3] != h[:3] for c, h in zip(cpu, hip, strict=True)):
            raise ValueError(f"CPU/HIP coordinates/material differ at step {step}")
        for column, name in ((3, "phi"), (4, "u_lattice"), (5, "v_lattice")):
            a = [float(r[column]) for r in cpu]
            b = [float(r[column]) for r in hip]
            absolute, relative = l2(a, b)
            if not math.isfinite(relative):
                raise ValueError(f"nonfinite CPU/HIP {name} difference")
            passed = relative <= TOLERANCE if name == "phi" else None
            if passed is False:
                raise ValueError(f"CPU/HIP {name} relative L2 {relative} at {step}")
            agreement.append(
                {
                    "step": step,
                    "field": name,
                    "absolute_l2_difference": absolute,
                    "relative_l2_difference": relative,
                    "agreement_passed": passed,
                    "criterion": f"relative L2 <= {TOLERANCE}"
                    if name == "phi"
                    else "diagnostic only; parasitic lattice velocity",
                }
            )
    return agreement


def hip_identity(receipt, selection):
    compiled = receipt.get("compiled_hip_version")
    if (
        receipt.get("backend") != "hip"
        or receipt.get("source_revision") != SOURCE
        or receipt.get("pci") != selection["pci"]
        or receipt.get("backend_uuid") != selection["backend_uuid"]
        or receipt.get("compiled_architecture") != receipt.get("architecture", "")
        or not isinstance(compiled, int)
        or compiled <= 0
        or receipt.get("hip_runtime_version") != compiled
        or receipt.get("hip_driver_version") != compiled
        or receipt.get("gpu_blocks") != 1
        or receipt.get("gpu_kernel_completion_verified") is not True
        or receipt.get("software_fallback") is not False
        or receipt.get("precision") != "float64"
    ):
        raise ValueError("HIP wetting receipt lacks exact device/runtime/completion")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("hip-native", "cpu-native", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--pci", required=True)
    parser.add_argument("--uuid", required=True)
    parser.add_argument("--resolution", type=int, default=24)
    args = parser.parse_args()
    before = service_resources()
    wetting = bridge()
    hip_exe = str(args.hip_native.resolve(strict=True))
    cpu_exe = str(args.cpu_native.resolve(strict=True))
    root = args.output.resolve()
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    home = root / "home"
    home.mkdir()
    env = {"HOME": str(home), "LC_ALL": "C"}
    selection = {
        "role": "compute",
        "backend": "hip",
        "pci": args.pci,
        "backend_uuid": args.uuid,
    }
    cpu_spec = request(90, args.resolution)
    hip_spec = {**cpu_spec, "backend": "hip", "selection": selection}
    wetting.validate(cpu_spec)
    wetting.validate(hip_spec)
    results = {}
    for label, executable, spec in (
        ("cpu", cpu_exe, cpu_spec),
        ("hip", hip_exe, hip_spec),
    ):
        code, output = run(executable, root, label, spec, env)
        if code:
            raise RuntimeError(f"{label} wetting failed; inspect {output}/process.log")
        receipt = json.loads((output / "wetting-receipt.json").read_text())
        if receipt["request"] != spec or receipt["backend"] != spec["backend"]:
            raise ValueError(f"{label} receipt science/backend identity changed")
        checks = [
            {
                "step": item["step"],
                "sha256": hashlib.sha256(
                    (output / item["path"]).read_bytes()
                ).hexdigest(),
                "check": wetting.assess_field(
                    spec, (output / item["path"]).read_bytes()
                ),
            }
            for item in receipt["snapshots"]
        ]
        verified = wetting.verify(spec, checks)
        if not verified["mass_passed"] or not verified["angle_passed"]:
            raise ValueError(f"{label} unchanged wetting mass/angle gate failed")
        results[label] = {"receipt": receipt, "fields": checks, "gates": verified}
    hip_identity(results["hip"]["receipt"], selection)
    agreement = compare(root / "cpu", root / "hip", cpu_spec["observation_steps"])
    rejections = []
    for label, executable, spec, extra, expected in (
        (
            "stale_uuid",
            hip_exe,
            {**hip_spec, "selection": {**selection, "backend_uuid": "GPU-" + "0" * 32}},
            {},
            "HIP UUID/PCI mismatch",
        ),
        (
            "wrong_backend",
            hip_exe,
            {**hip_spec, "selection": {**selection, "backend": "vulkan"}},
            {},
            "explicit HIP compute",
        ),
        (
            "missing_card",
            hip_exe,
            {**hip_spec, "selection": {**selection, "pci": "ffff:ff:1f.7"}},
            {},
            "HIP PCI card missing",
        ),
        (
            "architecture_spoof",
            hip_exe,
            hip_spec,
            {"HSA_OVERRIDE_GFX_VERSION": "11.0.0"},
            "architecture spoofing",
        ),
        ("cpu_descriptor_on_hip", hip_exe, cpu_spec, {}, "compiled backend required"),
        ("cpu_fallback", cpu_exe, hip_spec, {}, "compiled backend required"),
    ):
        code, output = run(executable, root, f"reject-{label}", spec, {**env, **extra})
        log = (output / "process.log").read_text()
        if (
            code == 0
            or expected not in log
            or {p.name for p in output.iterdir()} != {"process.log"}
        ):
            raise ValueError(f"{label} did not reject before scientific output")
        rejections.append({"case": label, "exit_code": code, "reason": log.strip()})
    report = {
        "schema_version": 1,
        "fixture": "synthetic equal-property planar wetting; native drivers only",
        "results": results,
        "retained_field_agreement": agreement,
        "rejections": rejections,
        "kfd_isolation": "unqualified",
        "physical_validation": "unqualified",
        "performance": "not compared",
        "service_resources_before": before,
        "service_resources_after": service_resources(),
    }
    (root / "verification.json").write_text(
        json.dumps(report, indent=2, allow_nan=False)
    )
    print(json.dumps({"agreement": agreement, "rejections": len(rejections)}, indent=2))


if __name__ == "__main__":
    main()
