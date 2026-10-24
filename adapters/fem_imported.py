"""Independent imported-box static FEM reference with explicit CAD/world coordinates."""

import hashlib
import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path


def load_bridge(name, path):
    source = importlib.util.spec_from_file_location(name, path)
    bridge = importlib.util.module_from_spec(source)
    source.loader.exec_module(bridge)
    return bridge


def validate(spec, fem, cad):
    if (
        set(spec)
        != {
            "schema_version",
            "geometry",
            "reference",
            "material_provenance",
            "boundary_provenance",
        }
        or type(spec["schema_version"]) is not int
        or spec["schema_version"] != 1
    ):
        raise ValueError("exact independent imported FEM descriptor required")
    cad.validate(spec["geometry"])
    fem.validate(spec["reference"])
    geometry, reference = spec["geometry"], spec["reference"]
    if (
        geometry["synthetic"] is not True
        or geometry["resolution"] != reference["resolution"]
        or geometry["geometry_tolerance_m"] != reference["geometry_tolerance_m"]
    ):
        raise ValueError(
            "controlled synthetic imported reference with unchanged mesh policy required"
        )
    lengths = cad.expected_lengths(geometry)
    if any(
        abs(a - b) > geometry["geometry_tolerance_m"]
        or abs(a - b) > geometry["volume_relative_tolerance"] * b
        for a, b in zip(lengths, reference["size_m"], strict=True)
    ):
        raise ValueError(
            "material/reference geometry differs from approved CAD world bounds"
        )
    for key in ("material_provenance", "boundary_provenance"):
        if (
            not isinstance(spec[key], str)
            or not spec[key].strip()
            or len(spec[key].encode()) > 4096
        ):
            raise ValueError(
                "explicit imported reference material and boundary provenance required"
            )


def main():
    sys.dont_write_bytecode = True
    if len(sys.argv) != 3 or sys.argv[1] != "reference":
        raise ValueError("usage: harbor-cad-fem-imported reference request.json")
    fem = load_bridge("harbor_cad_fem", "@fem_bridge@")
    cad = load_bridge("harbor_cad_cad_mesh", "@cad_mesh_bridge@")
    raw = fem.read_regular(sys.argv[2], 1024**2)
    spec = fem.strict_json(raw)
    if spec.get("schema_version") == 2:
        enclosure = load_bridge("harbor_cad_enclosure_thermal", "@enclosure_bridge@")
        enclosure.main(raw, spec, fem, sys.argv[2])
        return
    validate(spec, fem, cad)
    sandbox = fem.cpu_sandbox(
        "HARBOR_CAD_FEM_IMPORTED_POLICY",
        "harbor-cad-fem-imported-cpu-v1",
        "/fem-imported-runtime-closure.txt",
        sys.argv[2],
    )
    if sandbox is not None:
        sandbox["checks"]["source_brep_readonly"] = (
            os.statvfs("/inputs/solid.brep").f_flag & os.ST_RDONLY != 0
        )
        sandbox["checks"]["named_source_only"] = set(Path("/inputs").iterdir()) == {
            Path("/inputs/solid.brep")
        }
        if not all(sandbox["checks"].values()):
            raise ValueError("imported FEM source sandbox boundary failed")
    geometry, reference = spec["geometry"], spec["reference"]
    brep = fem.read_regular("/inputs/solid.brep", 64 * 1024**2)
    if (
        len(brep) != geometry["brep_bytes"]
        or hashlib.sha256(brep).hexdigest() != geometry["brep_sha256"]
    ):
        raise ValueError("approved imported BREP bytes changed")
    if any(p.name != "fem-imported.log" for p in Path.cwd().iterdir()):
        raise ValueError("new empty stage-local imported FEM output required")
    nodes, cells, sets = fem.mesh(reference, geometry=geometry)
    Path("reference.inp").write_text(fem.deck(reference, nodes, cells, sets))
    environment = {
        "OMP_NUM_THREADS": "1",
        "CCX_NPROC_RESULTS": "1",
        "CCX_NPROC_EQUATION_SOLVER": "1",
        "OPENBLAS_NUM_THREADS": "1",
        "MKL_NUM_THREADS": "1",
        "HOME": "/nonexistent",
        "LC_ALL": "C",
    }
    with Path("calculix.log").open("xb") as log:
        process = subprocess.run(
            ["@calculix@", "-i", "reference"],
            env=environment,
            stdout=log,
            stderr=subprocess.STDOUT,
            timeout=120,
            check=False,
        )
    text = fem.read_regular("calculix.log", 16 * 1024**2).decode()
    if (
        process.returncode
        or "*ERROR" in text
        or "Version @ccx_version@" not in text
        or "Job finished" not in text
    ):
        raise ValueError("exact imported native CalculiX reference did not succeed")
    data = fem.read_regular("reference.dat", 32 * 1024**2)
    fields = fem.read_dat(data.decode())
    origin = [geometry["bounds_m"][i] for i in (0, 2, 4)]
    checks = fem.verify(reference, nodes, cells, fields, origin=origin)
    if fem.read_regular("/inputs/solid.brep", 64 * 1024**2) != brep:
        raise ValueError("original imported BREP changed during solving")
    fem.atomic_json(
        "imported-fields.json",
        {
            "schema_version": 1,
            "static": True,
            "coordinate_unit": "m",
            "world_origin_m": origin,
            "fields": {
                name: [
                    {
                        "solver_step_parameter": record["time"],
                        "physical_time_s": None,
                        "values": [
                            {"id": list(identity), "value": value}
                            for identity, value in sorted(record["values"].items())
                        ],
                    }
                    for record in snapshots
                ]
                for name, snapshots in fields.items()
            },
        },
    )
    fem.atomic_json(
        "fem-imported-receipt.json",
        {
            "schema_version": 1,
            "adapter": "CalculiX",
            "backend": "cpu",
            "factorization": "SPOOLES",
            "executed": True,
            "software_fallback": False,
            "synthetic": True,
            "precision": "float64",
            "formulation": reference["mode"],
            "calculix_version": "@ccx_version@",
            "gmsh_version": "@gmsh_version@",
            "calculix_source_sha256": "9c88385c10fb04f5dc6c4e98027a51bebdd8aee3920e05190d6c1dd08357d6e7",
            "gmsh_source_sha256": "be3f66f225d27ba9fa014f07e83169285da8a051b0e8ab7103d88066b39bdd3e",
            "request_sha256": hashlib.sha256(raw).hexdigest(),
            "brep_sha256": geometry["brep_sha256"],
            "mesh_sha256": hashlib.sha256(
                fem.read_regular("mesh.json", 32 * 1024**2)
            ).hexdigest(),
            "native_field_sha256": hashlib.sha256(data).hexdigest(),
            "world_origin_m": origin,
            "world_bounds_m": geometry["bounds_m"],
            "source_transform": geometry["source_transform"],
            "placement_translation_unit": "mm",
            "source_unit": "mm",
            "coordinate_unit": "m",
            "scale_to_m": 0.001,
            "region_name": geometry["region_name"],
            "geometry_provenance": geometry["geometry_provenance"],
            "material_provenance": spec["material_provenance"],
            "boundary_provenance": spec["boundary_provenance"],
            "nodes": len(nodes),
            "elements": len(cells),
            "numerical_verification": checks,
            "gap_healing": False,
            "physical_validation": "unqualified",
            **({"sandbox": sandbox} if sandbox is not None else {}),
            "scope": "synthetic imported axis-aligned box static conduction/free expansion; no contact, physical-time or material-validation claim",
        },
    )
    print(json.dumps(checks, allow_nan=False, indent=2))


if __name__ == "__main__":
    main()
