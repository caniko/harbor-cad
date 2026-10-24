"""Imported BREP/OCC mesh correspondence; source placement is retained in SI.

Scope: one axis-aligned box, six semantic planar faces and structured C3D8.
Source API evidence: docs/cad-mesh.md. This operation is meshing, not a solve.
"""

import hashlib
import importlib.util
import math
import os
import re
import sys
from pathlib import Path


def number(value):
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
    ):
        raise ValueError("finite explicit SI numeric input required")
    return float(value)


def expected_lengths(spec):
    bounds = spec["bounds_m"]
    return [bounds[i + 1] - bounds[i] for i in (0, 2, 4)]


def expected_volume(spec):
    return math.prod(expected_lengths(spec))


def validate(spec):
    keys = {
        "schema_version",
        "synthetic",
        "backend",
        "formulation",
        "geometry_provenance",
        "brep_file",
        "brep_sha256",
        "brep_bytes",
        "region_name",
        "bounds_m",
        "volume_m3",
        "source_unit",
        "scale_to_m",
        "placement_translation_unit",
        "source_transform",
        "resolution",
        "geometry_tolerance_m",
        "volume_relative_tolerance",
    }
    if (
        set(spec) != keys
        or type(spec["schema_version"]) is not int
        or spec["schema_version"] != 1
        or type(spec["synthetic"]) is not bool
        or spec["backend"] != "cpu"
        or spec["formulation"] != "imported_axis_aligned_box"
    ):
        raise ValueError(
            "exact independent imported CPU box-correspondence descriptor required"
        )
    if (
        spec["brep_file"] != "solid.brep"
        or not isinstance(spec["brep_sha256"], str)
        or not re.fullmatch("[0-9a-f]{64}", spec["brep_sha256"])
        or type(spec["brep_bytes"]) is not int
        or not 1 <= spec["brep_bytes"] <= 64 * 1024**2
    ):
        raise ValueError("fixed bounded BREP source with exact bytes/digest required")
    if (
        not isinstance(spec["geometry_provenance"], str)
        or not spec["geometry_provenance"].strip()
        or len(spec["geometry_provenance"].encode()) > 4096
        or not isinstance(spec["region_name"], str)
        or not re.fullmatch("[A-Za-z0-9_-]{1,128}", spec["region_name"])
    ):
        raise ValueError("explicit named region and source provenance required")
    if (
        spec["source_unit"] != "mm"
        or number(spec["scale_to_m"]) != 0.001
        or spec["placement_translation_unit"] != "mm"
    ):
        raise ValueError(
            "explicit CAD millimetre geometry/placement and SI scale required"
        )
    bounds = spec["bounds_m"]
    transform = spec["source_transform"]
    if (
        not isinstance(bounds, list)
        or len(bounds) != 6
        or not isinstance(transform, list)
        or len(transform) != 16
    ):
        raise ValueError("complete CAD world bounds and original placement required")
    list(map(number, bounds))
    list(map(number, transform))
    if transform[12:] != [0.0, 0.0, 0.0, 1.0]:
        raise ValueError("explicit affine source placement required")
    rotation = [transform[i : i + 3] for i in (0, 4, 8)]
    for i in range(3):
        for j in range(3):
            if not math.isclose(
                sum(rotation[i][k] * rotation[j][k] for k in range(3)),
                float(i == j),
                abs_tol=1e-12,
            ):
                raise ValueError("rigid original FreeCAD placement required")
    a, b, c = rotation
    determinant = (
        a[0] * (b[1] * c[2] - b[2] * c[1])
        - a[1] * (b[0] * c[2] - b[2] * c[0])
        + a[2] * (b[0] * c[1] - b[1] * c[0])
    )
    if not math.isclose(determinant, 1.0, abs_tol=1e-12):
        raise ValueError("proper original FreeCAD placement required")
    lengths = expected_lengths(spec)
    volume = number(spec["volume_m3"])
    if (
        min(lengths) <= 0.0
        or max(lengths) / min(lengths) > 1000.0
        or volume <= 0.0
        or not math.isfinite(expected_volume(spec))
    ):
        raise ValueError("positive resolved bounded-aspect-ratio box required")
    if not 0 < number(spec["volume_relative_tolerance"]) <= 1e-10 or not math.isclose(
        volume, expected_volume(spec), rel_tol=spec["volume_relative_tolerance"]
    ):
        raise ValueError(
            "unchanged CAD-to-mesh volume gate and axis-aligned box volume required"
        )
    if (
        type(spec["resolution"]) is not int
        or not 2 <= spec["resolution"] <= 32
        or not 1e-10 <= number(spec["geometry_tolerance_m"]) < 0.001 * min(lengths)
        or any(10 * math.ulp(value) > spec["geometry_tolerance_m"] for value in bounds)
    ):
        raise ValueError(
            "bounded spatial refinement and resolved correspondence tolerance required"
        )


def main():
    sys.dont_write_bytecode = True
    if len(sys.argv) != 3 or sys.argv[1] != "mesh":
        raise ValueError("usage: harbor-cad-cad-mesh mesh request.json")
    source = importlib.util.spec_from_file_location("harbor_cad_fem", "@fem_bridge@")
    fem = importlib.util.module_from_spec(source)
    source.loader.exec_module(fem)
    raw = fem.read_regular(sys.argv[2], 1024**2)
    spec = fem.strict_json(raw)
    if spec.get("formulation") == "imported_general_solids":
        source = importlib.util.spec_from_file_location(
            "harbor_cad_solid_mesh", "@solid_mesh_bridge@"
        )
        solid = importlib.util.module_from_spec(source)
        source.loader.exec_module(solid)
        solid.main(raw, spec, fem, sys.argv[2])
        return
    validate(spec)
    sandbox = fem.cpu_sandbox(
        "HARBOR_CAD_CAD_MESH_POLICY",
        "harbor-cad-cad-mesh-cpu-v1",
        "/cad-mesh-runtime-closure.txt",
        sys.argv[2],
    )
    if sandbox is not None:
        checks = sandbox["checks"]
        checks["source_brep_readonly"] = (
            os.statvfs("/inputs/solid.brep").f_flag & os.ST_RDONLY != 0
        )
        checks["named_source_only"] = set(Path("/inputs").iterdir()) == {
            Path("/inputs/solid.brep")
        }
        if not all(checks.values()):
            raise ValueError("imported CAD mesh source sandbox boundary failed")
    brep = fem.read_regular("/inputs/solid.brep", 64 * 1024**2)
    if (
        len(brep) != spec["brep_bytes"]
        or hashlib.sha256(brep).hexdigest() != spec["brep_sha256"]
    ):
        raise ValueError("imported BREP source bytes changed")
    if any(p.name != "mesh.log" for p in Path.cwd().iterdir()):
        raise ValueError("new empty stage-local CAD mesh directory required")
    mesh_spec = {
        "size_m": expected_lengths(spec),
        "resolution": spec["resolution"],
        "geometry_tolerance_m": spec["geometry_tolerance_m"],
    }
    nodes, cells, sets = fem.mesh(mesh_spec, geometry=spec)
    tolerance = spec["geometry_tolerance_m"]
    bounds = spec["bounds_m"]
    if any(
        not bounds[2 * axis] - tolerance
        <= xyz[axis]
        <= bounds[2 * axis + 1] + tolerance
        for xyz in nodes.values()
        for axis in range(3)
    ):
        raise ValueError("native mesh left the approved CAD world bounds")
    for name, ids in sets.items():
        axis = "xyz".index(name[0])
        side = bounds[2 * axis + int(name.endswith("max"))]
        if any(abs(nodes[node][axis] - side) > tolerance for node in ids):
            raise ValueError("semantic mesh boundary left the approved named CAD face")
    if fem.read_regular("/inputs/solid.brep", 64 * 1024**2) != brep:
        raise ValueError("imported BREP source changed during meshing")
    fem.atomic_json(
        "cad-mesh-receipt.json",
        {
            "schema_version": 1,
            "adapter": "Gmsh",
            "backend": "cpu",
            "executed": True,
            "software_fallback": False,
            "precision": "float64",
            "formulation": spec["formulation"],
            "gmsh_version": "@gmsh_version@",
            "gmsh_source_sha256": "be3f66f225d27ba9fa014f07e83169285da8a051b0e8ab7103d88066b39bdd3e",
            "request_sha256": hashlib.sha256(raw).hexdigest(),
            "brep_sha256": spec["brep_sha256"],
            "mesh_sha256": hashlib.sha256(
                fem.read_regular("mesh.json", 32 * 1024**2)
            ).hexdigest(),
            "synthetic": spec["synthetic"],
            "region_name": spec["region_name"],
            "geometry_provenance": spec["geometry_provenance"],
            "source_unit": "mm",
            "coordinate_unit": "m",
            "scale_to_m": 0.001,
            "source_transform": spec["source_transform"],
            "placement_translation_unit": "mm",
            "world_bounds_m": spec["bounds_m"],
            "cad_volume_m3": spec["volume_m3"],
            "nodes": len(nodes),
            "elements": len(cells),
            "boundary_node_counts": {name: len(ids) for name, ids in sets.items()},
            "gap_healing": False,
            "physical_validation": "unqualified",
            **({"sandbox": sandbox} if sandbox is not None else {}),
        },
    )


if __name__ == "__main__":
    main()
