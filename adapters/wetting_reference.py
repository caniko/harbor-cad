"""Independent SI, phase-mass and contour-fit checks for a synthetic 2D wetting reference."""

import csv
import hashlib
import importlib.util
import io
import json
import math
import os
import stat
import subprocess
import sys
from pathlib import Path

POLICY = "harbor-cad-wetting-cpu-v1"
HIP_POLICY = "harbor-cad-wetting-hip-single-kfd-v1"
SELECTION = {"role", "backend", "pci", "backend_uuid"}
FIELDS = {
    "schema_version",
    "synthetic",
    "backend",
    "formulation",
    "diameter_m",
    "initial_center_above_wall_m",
    "resolution",
    "interface_width_m",
    "density_liquid_kg_m3",
    "density_vapor_kg_m3",
    "viscosity_liquid_m2_s",
    "viscosity_vapor_m2_s",
    "surface_tension_n_m",
    "contact_angle_deg",
    "phase_relaxation_time",
    "steps",
    "observation_steps",
    "mass_tolerance",
    "angle_tolerance_deg",
    "material_provenance",
    "boundary_provenance",
}


def device_selection(spec):
    """CPU carries no device; HIP carries exactly one explicit PCI/UUID compute card."""
    if spec.get("backend") == "cpu" and "selection" not in spec:
        return None
    selection = spec.get("selection")
    if (
        spec.get("backend") != "hip"
        or not isinstance(selection, dict)
        or set(selection) != SELECTION
        or selection["role"] != "compute"
        or selection["backend"] != "hip"
        or not all(
            isinstance(selection[key], str) and selection[key]
            for key in ("pci", "backend_uuid")
        )
    ):
        raise ValueError(
            "explicit CPU or exact HIP PCI/UUID compute wetting selection required"
        )
    return selection


def validate(spec):
    if (
        set(spec) - {"selection"} != FIELDS
        or type(spec["schema_version"]) is not int
        or spec["schema_version"] != 1
        or spec["synthetic"] is not True
        or spec["formulation"] != "well_balanced_contact_angle_2d"
    ):
        raise ValueError(
            "exact synthetic planar diffuse-interface wetting contract required"
        )
    device_selection(spec)
    for key in FIELDS - {
        "synthetic",
        "backend",
        "formulation",
        "observation_steps",
        "material_provenance",
        "boundary_provenance",
    }:
        if type(spec[key]) not in (int, float) or not math.isfinite(spec[key]):
            raise ValueError("explicit finite numeric wetting inputs required")
    for key in ("resolution", "steps"):
        if type(spec[key]) is not int:
            raise ValueError(
                "integer wetting refinement and integration budget required"
            )
    n, diameter = spec["resolution"], spec["diameter_m"]
    rho, nu = spec["density_liquid_kg_m3"], spec["viscosity_liquid_m2_s"]
    if (
        not 24 <= n <= 96
        or not 100 <= spec["steps"] <= 800000
        or spec["initial_center_above_wall_m"] != 0
        or not 0 < diameter <= 0.001
        or not 0 < rho <= 1e5
        or not 0 < nu <= 1
        or rho != spec["density_vapor_kg_m3"]
        or nu != spec["viscosity_vapor_m2_s"]
    ):
        raise ValueError(
            "bounded equal-property synthetic reference only; water/air ratios unqualified"
        )
    dx = diameter / n
    dt = (1 - 0.5) / 3 * dx * dx / nu
    sigma = spec["surface_tension_n_m"] * dt * dt / (rho * dx**3)
    if (
        not math.isfinite(dt)
        or dt <= 0
        or not math.isfinite(sigma)
        or not 0 < sigma <= 0.02
        or not 0 < spec["interface_width_m"] <= diameter / 6
        or not 3 <= spec["interface_width_m"] / dx <= 16
        or not 60 <= spec["contact_angle_deg"] <= 120
        or not 0.6 <= spec["phase_relaxation_time"] <= 1.5
        or not 0 < spec["mass_tolerance"] <= 1e-3
        or not 0 < spec["angle_tolerance_deg"] <= 5
    ):
        raise ValueError(
            "resolved interface, bounded lattice parameters and unchanged mass/angle gates required"
        )
    retained = spec["observation_steps"]
    if (
        not isinstance(retained, list)
        or not 2 <= len(retained) <= 32
        or any(type(step) is not int for step in retained)
        or retained != sorted(set(retained))
        or retained[0] != 0
        or retained[-1] != spec["steps"]
    ):
        raise ValueError("ordered initial/final integration observations required")
    for key in ("material_provenance", "boundary_provenance"):
        if (
            not isinstance(spec[key], str)
            or not spec[key].strip()
            or len(spec[key]) > 4096
        ):
            raise ValueError("explicit material and wall provenance required")
    return {
        "spacing_m": dx,
        "physical_step_s": dt,
        "surface_tension_lattice": sigma,
        "interface_width_lattice": spec["interface_width_m"] / dx,
    }


def read_field(data, dx):
    if not 0 < len(data) <= 16 * 1024**2 or not 0 < dx < 1:
        raise ValueError("bounded SI field required")
    reader = csv.DictReader(io.StringIO(data.decode("ascii")))
    if reader.fieldnames != ["x_m", "y_m", "material", "phi", "u_lattice", "v_lattice"]:
        raise ValueError("exact scientific wetting field columns required")
    grid = {}
    for row in reader:
        if None in row or None in row.values():
            raise ValueError("incomplete wetting field row")
        x, y, phi, u, v = (
            float(row[key]) for key in ("x_m", "y_m", "phi", "u_lattice", "v_lattice")
        )
        material = int(row["material"])
        if (
            not all(map(math.isfinite, (x, y, phi, u, v)))
            or material not in (1, 2)
            or not -0.05 <= phi <= 1.05
        ):
            raise ValueError("finite bounded phase/velocity/material field required")
        index = (round(x / dx), round(y / dx))
        if (
            any(
                abs(value / dx - i) > 1e-8
                for value, i in zip((x, y), index, strict=True)
            )
            or min(index) < 0
            or index in grid
        ):
            raise ValueError("distinct complete native Cartesian field required")
        grid[index] = (material, phi, u, v)
        if len(grid) > 40000:
            raise ValueError("wetting cell budget exceeded")
    if not grid:
        raise ValueError("empty wetting field")
    nx, ny = max(x for x, _ in grid) + 1, max(y for _, y in grid) + 1
    if len(grid) != nx * ny:
        raise ValueError("incomplete native wetting Cartesian grid")
    if any(
        material != (2 if y in (0, ny - 1) else 1)
        for (_, y), (material, *_) in grid.items()
    ):
        raise ValueError("exact periodic-x and two planar wall materials required")
    return grid, (nx, ny)


def circle_fit(points):
    # Coordinates are lattice-scaled and centered, avoiding SI ill-conditioning.
    if len(points) < 12:
        raise ValueError("resolved contour has too few independent crossings")
    center = [sum(p[i] for p in points) / len(points) for i in (0, 1)]
    local = [[p[i] - center[i] for i in (0, 1)] for p in points]
    rows = [[2 * x, 2 * y, 1.0] for x, y in local]
    rhs = [x * x + y * y for x, y in local]
    normal = [
        [sum(row[i] * row[j] for row in rows) for j in range(3)]
        + [sum(row[i] * b for row, b in zip(rows, rhs, strict=True))]
        for i in range(3)
    ]
    for i in range(3):
        pivot = max(range(i, 3), key=lambda k: abs(normal[k][i]))
        normal[i], normal[pivot] = normal[pivot], normal[i]
        value = normal[i][i]
        if abs(value) < 1e-12:
            raise ValueError("degenerate wetting contour fit")
        normal[i] = [v / value for v in normal[i]]
        for k in range(3):
            if k != i:
                value = normal[k][i]
                normal[k] = [
                    a - value * b for a, b in zip(normal[k], normal[i], strict=True)
                ]
    cx, cy, c = [row[3] for row in normal]
    if c + cx * cx + cy * cy <= 0:
        raise ValueError("invalid wetting radius")
    radius = math.sqrt(c + cx * cx + cy * cy)
    cx, cy = cx + center[0], cy + center[1]
    residual = max(abs(math.hypot(x - cx, y - cy) / radius - 1) for x, y in points)
    return cx, cy, radius, residual


def assess_field(spec, data):
    units = validate(spec)
    dx = units["spacing_m"]
    grid, shape = read_field(data, dx)
    if shape != (int(2.5 * spec["resolution"]) + 1, int(1.5 * spec["resolution"]) + 1):
        raise ValueError("native domain differs from approved wetting geometry")
    fluid = [v for v in grid.values() if v[0] == 1]
    amount = math.fsum(1 - v[1] for v in fluid) * dx**2
    points = []
    for y in range(1, shape[1] - 1):
        crossings = []
        for x in range(shape[0] - 1):
            a, b = grid[x, y][1] - 0.5, grid[x + 1, y][1] - 0.5
            if a * b < 0 or a == 0:
                crossings.append(x - a / (b - a) if b != a else float(x))
        if not crossings:
            continue
        # A circle can touch an integer-grid row at precisely phi=0.5.
        # This zero-length tangency has one point and no liquid interval.
        # Retain it as contour evidence without inventing a second crossing.
        tangent = len(crossings) == 1 and all(
            grid[x, y][1] >= 0.5 for x in range(shape[0])
        )
        if tangent and 1 < crossings[0] < shape[0] - 2:
            if y - 0.5 >= units["interface_width_lattice"] / 2:
                points.append((crossings[0], y))
            continue
        if len(crossings) != 2 or crossings[0] <= 1 or crossings[1] >= shape[0] - 2:
            raise ValueError("one isolated droplet contour required")
        if y - 0.5 >= units["interface_width_lattice"] / 2:
            points.extend((x, y) for x in crossings)
    cx, cy, radius, residual = circle_fit(points)
    cosine = (0.5 - cy) / radius
    if (
        not -1 < cosine < 1
        or residual > 0.05
        or abs(cx - (shape[0] - 1) / 2) > 0.01 * radius
    ):
        raise ValueError("wall-intersecting symmetric near-circular droplet required")
    return {
        "shape": list(shape),
        "fluid_nodes": len(fluid),
        "droplet_area_m2": amount,
        "contact_angle_deg": math.degrees(math.acos(cosine)),
        "circle_radius_m": radius * dx,
        "circle_center_m": [cx * dx, cy * dx],
        "relative_radial_residual": residual,
        "contour_points": len(points),
        "max_speed_lattice": max(math.hypot(v[2], v[3]) for v in fluid),
    }


def verify(spec, snapshots):
    validate(spec)
    if [v["step"] for v in snapshots] != spec["observation_steps"]:
        raise ValueError("exact approved wetting observations required")
    initial = snapshots[0]["check"]["droplet_area_m2"]
    if initial <= 0:
        raise ValueError("positive phase area required")
    mass = max(abs(v["check"]["droplet_area_m2"] / initial - 1) for v in snapshots)
    error = abs(snapshots[-1]["check"]["contact_angle_deg"] - spec["contact_angle_deg"])
    return {
        "mass_relative_error": mass,
        "mass_tolerance": spec["mass_tolerance"],
        "mass_passed": mass <= spec["mass_tolerance"],
        "angle_abs_error_deg": error,
        "angle_tolerance_deg": spec["angle_tolerance_deg"],
        "angle_passed": error <= spec["angle_tolerance_deg"],
        "physical_validation": "unqualified",
    }


def check_output_directory(root):
    # The worker opens its stage log before native launch. No scientific output
    # or aliased/nonregular log may predate this immutable run.
    for entry in root.iterdir():
        metadata = entry.lstat()
        if (
            entry.name != "wetting.log"
            or not stat.S_ISREG(metadata.st_mode)
            or metadata.st_nlink != 1
            or metadata.st_size != 0
        ):
            raise ValueError(
                "fresh wetting stage with only its empty worker log required"
            )


def hip_sandbox(fem, selection, request_path, dev=Path("/dev"), sys_root=Path("/sys")):
    """Worker HIP sandbox: only the selected render node, KFD and selected sysfs metadata."""
    if not os.environ.get("HARBOR_CAD_WETTING_POLICY"):
        return None
    if os.environ["HARBOR_CAD_WETTING_POLICY"] != HIP_POLICY:
        raise ValueError("exact HIP wetting sandbox policy required")
    mounts = set(
        fem.read_regular("/wetting-runtime-closure.txt", 2 * 1024**2)
        .decode()
        .splitlines()
    )
    if set(map(str, Path("/nix/store").iterdir())) != mounts:
        raise ValueError("HIP wetting sandbox must expose only its operation closure")
    alias = dev / "dri/by-path" / f"pci-{selection['pci']}-render"
    node = alias.resolve() if alias.is_symlink() else None
    checks = {
        "operation_closure_only": True,
        "selected_gpu_nodes_only": node is not None
        and (dev / "kfd").is_char_device()
        and node.is_char_device()
        and {p.name for p in (dev / "dri").iterdir()} == {"by-path", node.name}
        and [p.name for p in (dev / "dri/by-path").iterdir()] == [alias.name],
        "selected_sysfs_only": {p.name for p in sys_root.iterdir()}
        <= {"bus", "dev", "devices"},
        "no_host_home": set(Path("/home").iterdir()) == {Path("/home/worker")},
        "no_session_bus": not Path("/run/user").exists()
        and "DBUS_SESSION_BUS_ADDRESS" not in os.environ,
        "no_worker_socket": not list(Path("/work").glob("*.sock")),
        "network_namespace_isolated": os.readlink("/proc/self/ns/net")
        != os.environ["HARBOR_CAD_HOST_NETNS"],
        "descriptor_readonly": os.statvfs(request_path).f_flag & os.ST_RDONLY != 0,
    }
    if not all(checks.values()):
        raise ValueError("HIP wetting sandbox boundary failed")
    return {"policy": HIP_POLICY, "checks": checks}


def main():
    if len(sys.argv) != 3 or sys.argv[1] != "reference":
        raise ValueError("usage: harbor-cad-wetting reference request.json")
    module = importlib.util.spec_from_file_location("fem_reference", "@fem_bridge@")
    fem = importlib.util.module_from_spec(module)
    module.loader.exec_module(fem)
    raw = fem.read_regular(sys.argv[2], 65536)
    spec = fem.strict_json(raw)
    validate(spec)
    selection = device_selection(spec)
    if selection is None:
        sandbox = fem.cpu_sandbox(
            "HARBOR_CAD_WETTING_POLICY",
            POLICY,
            "/wetting-runtime-closure.txt",
            sys.argv[2],
        )
    else:
        sandbox = hip_sandbox(fem, selection, sys.argv[2])
    if sandbox is None:
        raise ValueError("operation-specific wetting sandbox required")
    check_output_directory(Path.cwd())
    with Path("process.log").open("xb") as log:
        process = subprocess.run(
            ["@native@", "reference", sys.argv[2]],
            stdout=log,
            stderr=subprocess.STDOUT,
            timeout=1200,
            env={"HOME": "/home/worker", "LC_ALL": "C"},
            check=False,
        )
    if process.returncode:
        raise ValueError(
            "native wetting solve failed; original process.log and any fields retained"
        )
    receipt = fem.strict_json(fem.read_regular("wetting-receipt.json", 65536))
    if (
        receipt["request"] != spec
        or receipt["executed"] is not True
        or receipt["source_revision"] != "145cd54810b468f4b6fd3ed86b10644264841578"
        or receipt["backend"] != spec["backend"]
        or receipt["precision"] != "float64"
        or receipt["software_fallback"] is not False
        or (
            selection is not None
            and (
                receipt.get("pci") != selection["pci"]
                or receipt.get("backend_uuid") != selection["backend_uuid"]
                or receipt.get("gpu_kernel_completion_verified") is not True
            )
        )
    ):
        raise ValueError(
            "native wetting execution or exact science/source identity failed"
        )
    snapshots = []
    for item in receipt["snapshots"]:
        if item["path"] != f"wetting-{item['step']}.csv":
            raise ValueError("unexpected native wetting path")
        data = fem.read_regular(item["path"], 16 * 1024**2)
        check = assess_field(spec, data)
        if (
            not math.isclose(
                item["droplet_area_m2"], check["droplet_area_m2"], rel_tol=1e-12
            )
            or item["fluid_nodes"] != check["fluid_nodes"]
            or item["shape"] != check["shape"]
        ):
            raise ValueError("independent field/native phase mass or coverage differs")
        snapshots.append(
            {
                "step": item["step"],
                "time_s": item["time_s"],
                "path": item["path"],
                "sha256": hashlib.sha256(data).hexdigest(),
                "check": check,
            }
        )
    checks = verify(spec, snapshots)
    receipt.update(
        request_sha256=hashlib.sha256(raw).hexdigest(),
        sandbox=sandbox,
        independent_fields=snapshots,
        numerical_verification=checks,
    )
    path = Path("verified-wetting-receipt.json")
    path.write_text(json.dumps(receipt, indent=2, allow_nan=False))
    if not checks["mass_passed"] or not checks["angle_passed"]:
        raise ValueError(
            "unchanged wetting mass/angle gate failed; native fields retained"
        )


if __name__ == "__main__":
    main()
