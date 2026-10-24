"""Fixed Gmsh/CalculiX CPU references; no native imports in MCP or discovery.

Exact manual/API sources and scientific scope: docs/fem-references.md.
"""

import hashlib
import importlib
import json
import math
import os
import re
import resource
import stat
import subprocess
import sys
import time
from pathlib import Path


def read_regular(path, maximum):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size > maximum:
            raise ValueError("bounded nonsymlink regular file required")
        data = stream.read(maximum + 1)
    if len(data) != info.st_size or len(data) > maximum:
        raise ValueError("file changed while reading")
    return data


def atomic_json(name, value):
    target = Path(name)
    partial = Path(name + ".partial")
    if target.exists() or target.is_symlink():
        raise ValueError("new stage-local output required")
    data = json.dumps(value, allow_nan=False, indent=2).encode()
    if len(data) > 32 * 1024**2:
        raise ValueError("FEM output descriptor limit")
    with partial.open("xb") as stream:
        stream.write(data)
    partial.replace(target)


def number(value):
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
    ):
        raise ValueError("finite numeric input required")
    return float(value)


def strict_json(raw):
    def pairs(entries):
        result = {}
        for name, value in entries:
            if name in result:
                raise ValueError("duplicate FEM request field")
            result[name] = value
        return result

    def constant(_):
        raise ValueError("standard finite JSON numbers required")

    result = json.loads(raw, object_pairs_hook=pairs, parse_constant=constant)
    if not isinstance(result, dict):
        raise TypeError("exact FEM request object required")
    return result


def validate(spec):
    common = {
        "schema_version",
        "synthetic",
        "backend",
        "mode",
        "size_m",
        "resolution",
        "geometry_tolerance_m",
        "temperatures_k",
        "numerical_tolerance",
    }
    mode = spec.get("mode")
    extra = (
        {"conductivity_w_m_k"}
        if mode == "thermal_boundary"
        else {"young_modulus_pa", "poisson_ratio", "expansion_per_k"}
    )
    if set(spec) != common | extra or mode not in {
        "thermal_boundary",
        "free_expansion",
    }:
        raise ValueError("exact allowlisted versioned reference request required")
    if (
        type(spec["schema_version"]) is not int
        or spec["schema_version"] != 1
        or spec["synthetic"] is not True
        or spec["backend"] != "cpu"
    ):
        raise ValueError(
            "explicit synthetic CPU reference required; GPU cannot fall back"
        )
    sizes = [number(v) for v in spec["size_m"]]
    temperatures = [number(v) for v in spec["temperatures_k"]]
    resolution = spec["resolution"]
    tolerance = number(spec["numerical_tolerance"])
    geometric = number(spec["geometry_tolerance_m"])
    if len(sizes) != 3 or any(v <= 0 for v in sizes) or max(sizes) / min(sizes) > 1000:
        raise ValueError("positive bounded-aspect-ratio three-dimensional box required")
    if (
        len(temperatures) != 2
        or any(v <= 0 for v in temperatures)
        or temperatures[0] == temperatures[1]
    ):
        raise ValueError("two distinct positive absolute temperatures in K required")
    if (
        type(resolution) is not int
        or not 2 <= resolution <= 32
        or not 0 < tolerance <= 1e-6
    ):
        raise ValueError(
            "bounded explicit refinement and unchanged reference gate required"
        )
    if not 1e-10 <= geometric < 1e-3 * min(sizes):
        raise ValueError("explicit resolved geometry tolerance required")
    if mode == "thermal_boundary":
        if number(spec["conductivity_w_m_k"]) <= 0:
            raise ValueError("positive constant conductivity required")
    elif (
        number(spec["young_modulus_pa"]) <= 0
        or not -1 < number(spec["poisson_ratio"]) < 0.5
        or number(spec["expansion_per_k"]) == 0
    ):
        raise ValueError(
            "stable isotropic elastic material and nonzero expansion required"
        )
    if (
        mode == "free_expansion"
        and abs(spec["expansion_per_k"] * (temperatures[1] - temperatures[0])) > 0.01
    ):
        raise ValueError("small-strain free expansion reference required")


def classify_box_faces(surfaces, bounds, tolerance):
    if len(surfaces) != 6 or len(bounds) != 6 or not 0 < tolerance:
        raise ValueError("one unambiguous six-face box required")
    result = {}
    for tag, box in surfaces.items():
        matches = []
        for axis in range(3):
            for side in range(2):
                target = bounds[2 * axis + side]
                expected = list(bounds)
                expected[2 * axis : 2 * axis + 2] = [target, target]
                if len(box) == 6 and all(
                    math.isfinite(a) and abs(a - b) <= tolerance
                    for a, b in zip(box, expected)
                ):
                    matches.append("xyz"[axis] + ("min" if side == 0 else "max"))
        if len(matches) != 1 or matches[0] in result:
            raise ValueError("missing, ambiguous or non-planar semantic face")
        result[matches[0]] = tag
    return result


def import_brep(gmsh, geometry):
    # OCC import's uniform gp_Trsf scaling preserves analytic lines/planes.
    # Generic occ.dilate uses gp_GTrsf and converts them to NURBS instead.
    # Gmsh 4.15.2 GModelIO_OCC.cpp: importShapes -> _healShape scaling, then
    # early return when all repair options are disabled. No healing/rebuild.
    for name in (
        "FixDegenerated",
        "FixSmallEdges",
        "FixSmallFaces",
        "SewFaces",
        "MakeSolids",
    ):
        gmsh.option.setNumber("Geometry.OCC" + name, 0)
    gmsh.option.setNumber("Geometry.OCCScaling", geometry["scale_to_m"])
    bodies = gmsh.model.occ.importShapes(
        "/inputs/solid.brep", highestDimOnly=True, format="brep"
    )
    if len(bodies) != 1 or bodies[0][0] != 3:
        raise ValueError("imported BREP must contain exactly one closed solid")
    return bodies[0][1]


def planar_vertex_bounds(gmsh, dim, tag):
    # getBoundingBox includes OCC tolerance padding. Exact four-vertex planar
    # boxes can retain a tighter requested geometric gate using the native CAD
    # coordinates, without changing coordinates or healing any gap.
    vertices = gmsh.model.getBoundary([(dim, tag)], oriented=False, recursive=True)
    if (
        dim != 2
        or gmsh.model.getType(dim, tag) != "Plane"
        or len(vertices) != 4
        or any(d != 0 for d, _ in vertices)
    ):
        raise ValueError("exact four-vertex planar face required")
    points = [gmsh.model.getValue(0, t, []) for _, t in vertices]
    if any(len(p) != 3 or not all(math.isfinite(v) for v in p) for p in points):
        raise ValueError("finite exact native vertex coordinates required")
    return tuple(min(p[a] for p in points) for a in range(3)) + tuple(
        max(p[a] for p in points) for a in range(3)
    )


def gmsh_api():
    # This exact module path is substituted by Nix. It owns its compatible native
    # library; no ambient Python/Qt/loader search-path injection is needed.
    sys.path.insert(0, "@gmsh_module@")
    gmsh = importlib.import_module("gmsh")
    if gmsh.__version__ != "@gmsh_version@":
        raise ValueError("exact Gmsh ABI/version required")
    return gmsh


def mesh(spec, geometry=None, *, exact_planar_vertices=False):
    gmsh = gmsh_api()
    gmsh.initialize(["harbor-cad-fem", "-nopopup"], readConfigFiles=False)
    try:
        gmsh.option.setNumber("General.NumThreads", 1)
        gmsh.option.setNumber("Mesh.MaxNumThreads1D", 1)
        gmsh.option.setNumber("Mesh.MaxNumThreads2D", 1)
        gmsh.option.setNumber("Mesh.MaxNumThreads3D", 1)
        gmsh.option.setNumber("Mesh.ElementOrder", 1)
        gmsh.model.add(
            "synthetic-reference" if geometry is None else "imported-reference"
        )
        lengths = spec["size_m"]
        if geometry is None:
            body = gmsh.model.occ.addBox(0, 0, 0, *lengths)
            bounds = [value for size in lengths for value in (0.0, size)]
        else:
            # Gmsh 4.15.2 documented OCC API; no healing or reconstruction.
            # https://gmsh.info/doc/texinfo/gmsh.html#gmsh_002fmodel_002focc_002fimportShapes
            body = import_brep(gmsh, geometry)
            bounds = geometry["bounds_m"]
        gmsh.model.occ.synchronize()
        if geometry is not None:
            box = gmsh.model.getBoundingBox(3, body)
            world = [box[0], box[3], box[1], box[4], box[2], box[5]]
            if (
                any(
                    abs(a - b) > spec["geometry_tolerance_m"]
                    for a, b in zip(world, bounds, strict=True)
                )
                or len(gmsh.model.getEntities(3)) != 1
            ):
                raise ValueError("imported CAD world bounds or solid identity changed")
            if not math.isclose(
                gmsh.model.occ.getMass(3, body),
                geometry["volume_m3"],
                rel_tol=geometry["volume_relative_tolerance"],
            ):
                raise ValueError(
                    "imported BREP-to-CAD solid volume correspondence failed"
                )
            if (
                len(gmsh.model.getEntities(1)) != 12
                or len(gmsh.model.getEntities(0)) != 8
                or any(
                    gmsh.model.getType(1, tag) != "Line"
                    for _, tag in gmsh.model.getEntities(1)
                )
            ):
                raise ValueError(
                    "initial imported reference requires an exact axis-aligned planar box"
                )
        faces = {}
        for dim, tag in gmsh.model.getBoundary([(3, body)], oriented=False):
            if dim != 2:
                raise ValueError("surface topology required")
            box = (
                planar_vertex_bounds(gmsh, dim, tag)
                if exact_planar_vertices
                else gmsh.model.getBoundingBox(dim, tag)
            )
            if geometry is not None and gmsh.model.getType(dim, tag) != "Plane":
                raise ValueError("unsupported curved imported reference face")
            faces[tag] = [box[0], box[3], box[1], box[4], box[2], box[5]]
        names = classify_box_faces(faces, bounds, spec["geometry_tolerance_m"])
        for _, curve in gmsh.model.getEntities(1):
            gmsh.model.mesh.setTransfiniteCurve(curve, spec["resolution"] + 1)
        for name, face in names.items():
            group = gmsh.model.addPhysicalGroup(2, [face])
            gmsh.model.setPhysicalName(2, group, name)
            gmsh.model.mesh.setTransfiniteSurface(face)
            gmsh.model.mesh.setRecombine(2, face)
            axis = "xyz".index(name[0])
            area = math.prod(
                size for index, size in enumerate(lengths) if index != axis
            )
            if not math.isclose(gmsh.model.occ.getMass(2, face), area, rel_tol=1e-10):
                raise ValueError("semantic boundary area changed")
        volume = gmsh.model.addPhysicalGroup(3, [body])
        gmsh.model.setPhysicalName(3, volume, "solid")
        gmsh.model.mesh.setTransfiniteVolume(body)
        gmsh.model.mesh.generate(3)
        types, tags, connectivity = gmsh.model.mesh.getElements(3, body)
        if list(types) != [5] or len(tags[0]) != spec["resolution"] ** 3:
            raise ValueError("structured linear C3D8 hexahedra required")
        node_tags, coordinates, _ = gmsh.model.mesh.getNodes()
        nodes = {
            int(tag): list(map(float, coordinates[3 * i : 3 * i + 3]))
            for i, tag in enumerate(node_tags)
        }
        cells = {
            int(tag): list(map(int, connectivity[0][8 * i : 8 * i + 8]))
            for i, tag in enumerate(tags[0])
        }
        if len(nodes) != (spec["resolution"] + 1) ** 3 or any(
            not all(math.isfinite(v) for v in xyz) for xyz in nodes.values()
        ):
            raise ValueError("finite complete nodal coordinates required")
        gauss, weights = gmsh.model.mesh.getIntegrationPoints(5, "Gauss2")
        _, determinants, _ = gmsh.model.mesh.getJacobians(5, gauss, body)
        if len(determinants) != len(cells) * len(weights) or any(
            not math.isfinite(d) or d <= 0 for d in determinants
        ):
            raise ValueError("positive element Jacobians required")
        integrated_volume = sum(
            d * float(weights[i % len(weights)]) for i, d in enumerate(determinants)
        )
        if not math.isclose(integrated_volume, math.prod(lengths), rel_tol=1e-10):
            raise ValueError("mesh-to-solid volume correspondence failed")
        sets = {
            name: sorted(
                map(int, gmsh.model.mesh.getNodes(2, face, includeBoundary=True)[0])
            )
            for name, face in names.items()
        }
        if any(
            len(values) != (spec["resolution"] + 1) ** 2 for values in sets.values()
        ):
            raise ValueError("complete semantic boundary nodes required")
        gmsh.write("reference.msh")
        atomic_json(
            "mesh.json",
            {
                "schema_version": 1,
                "synthetic": True if geometry is None else geometry["synthetic"],
                "coordinate_unit": "m",
                "nodes": nodes,
                "elements": cells,
                "element_type": "C3D8",
                "boundary_node_sets": sets,
                "semantic_face_selection": "planar bounding box plus area; no ordinal face dependency",
                "positive_gauss_jacobians": True,
                "integrated_volume_m3": integrated_volume,
            },
        )
        return nodes, cells, sets
    finally:
        gmsh.finalize()


def mesh_deck(nodes, cells, sets):
    lines = ["*HEADING", "Harbor synthetic CPU reference; SI units", "*NODE,NSET=NALL"]
    lines += [
        str(tag) + "," + ",".join(f"{x:.17g}" for x in xyz)
        for tag, xyz in sorted(nodes.items())
    ]
    lines.append("*ELEMENT,TYPE=C3D8,ELSET=EALL")
    lines += [
        str(tag) + "," + ",".join(map(str, ids)) for tag, ids in sorted(cells.items())
    ]
    for name, values in sorted(sets.items()):
        lines.append("*NSET,NSET=" + name.upper())
        lines += [
            ",".join(map(str, values[i : i + 16])) for i in range(0, len(values), 16)
        ]
    return lines


def deck(spec, nodes, cells, sets):
    lines = mesh_deck(nodes, cells, sets)
    lines += ["*MATERIAL,NAME=SOLID"]
    t0, t1 = spec["temperatures_k"]
    if spec["mode"] == "thermal_boundary":
        lines += ["*CONDUCTIVITY", f"{spec['conductivity_w_m_k']:.17g}"]
    else:
        lines += [
            "*ELASTIC",
            f"{spec['young_modulus_pa']:.17g},{spec['poisson_ratio']:.17g}",
            f"*EXPANSION,ZERO={t0:.17g}",
            f"{spec['expansion_per_k']:.17g}",
        ]
    lines += ["*SOLID SECTION,ELSET=EALL,MATERIAL=SOLID"]
    if spec["mode"] == "thermal_boundary":
        lines += [
            "*STEP",
            "*HEAT TRANSFER,STEADY STATE,SOLVER=SPOOLES,DIRECT",
            "1.,1.",
            "*BOUNDARY",
            f"XMIN,11,11,{t0:.17g}",
            f"XMAX,11,11,{t1:.17g}",
            "*NODE PRINT,NSET=NALL,FREQUENCY=999999,GLOBAL=YES",
            "NT",
            "*EL PRINT,ELSET=EALL,FREQUENCY=999999,GLOBAL=YES",
            "HFL",
            "*NODE FILE",
            "NT",
            "*EL FILE",
            "HFL",
        ]
    else:
        lines += [
            "*INITIAL CONDITIONS,TYPE=TEMPERATURE",
            f"NALL,{t0:.17g}",
            "*BOUNDARY",
            "XMIN,1,1,0.",
            "YMIN,2,2,0.",
            "ZMIN,3,3,0.",
            "*STEP",
            "*STATIC,SOLVER=SPOOLES,DIRECT",
            "1.,1.",
            "*TEMPERATURE",
            f"NALL,{t1:.17g}",
            "*NODE PRINT,NSET=NALL,FREQUENCY=999999,GLOBAL=YES",
            "U",
            "*EL PRINT,ELSET=EALL,FREQUENCY=999999,GLOBAL=YES",
            "S",
            "*NODE FILE",
            "U",
            "*EL FILE",
            "S",
        ]
    lines.append("*END STEP")
    return "\n".join(lines) + "\n"


def read_dat(text, *, reaction_forces=False):
    definitions = {
        "temperatures": ("temperature", "NALL", 1, 1),
        "displacements (vx,vy,vz)": ("displacement", "NALL", 1, 3),
        "heat flux (elem, integ.pnt.,qx,qy,qz)": ("heat_flux", "EALL", 2, 3),
        "stresses (elem, integ.pnt.,sxx,syy,szz,sxy,sxz,syz)": ("stress", "EALL", 2, 6),
    }
    if reaction_forces:
        definitions["forces (fx,fy,fz)"] = ("reaction_force", "NALL", 1, 3)
    fields, current, ids, count = {}, None, 0, 0
    for raw in text.splitlines():
        line = raw.strip()
        if not line:
            continue
        if re.fullmatch(r"(?:S T E P|INCREMENT)\s+[1-9][0-9]*", line):
            current = None
            continue
        if " for set " in line:
            label, rest = line.split(" for set ", 1)
            if label not in definitions:
                raise ValueError("unsupported native field section")
            key, expected_set, ids, count = definitions[label]
            name, stamp = re.split(r"\s+and time\s+", rest)
            if name.strip() != expected_set:
                raise ValueError("native field references an unrelated set")
            stamp = float(stamp.replace("D", "E"))
            if not math.isfinite(stamp) or stamp < 0:
                raise ValueError("finite nonnegative solver step parameter required")
            current = {"time": stamp, "values": {}}
            fields.setdefault(key, []).append(current)
        else:
            parts = line.split()
            if (
                current is None
                or len(parts) != ids + count
                or any(not p.isdecimal() or int(p) <= 0 for p in parts[:ids])
            ):
                raise ValueError("native field identifiers/components changed")
            identity = tuple(map(int, parts[:ids]))
            values = [float(p.replace("D", "E")) for p in parts[ids:]]
            if identity in current["values"] or not all(
                math.isfinite(v) for v in values
            ):
                raise ValueError("duplicate/nonfinite native field values")
            current["values"][identity] = values
    if any(
        not snapshot["values"]
        for snapshots in fields.values()
        for snapshot in snapshots
    ):
        raise ValueError("empty native field snapshot")
    return fields


def verify(spec, nodes, cells, fields, origin=(0.0, 0.0, 0.0)):
    if len(origin) != 3 or any(not math.isfinite(number(v)) for v in origin):
        raise ValueError("finite explicit reference-origin coordinates required")
    requested = (
        {"temperature", "heat_flux"}
        if spec["mode"] == "thermal_boundary"
        else {"displacement", "stress"}
    )
    if set(fields) != requested or any(
        len(snapshots) != 1 or snapshots[0]["time"] != 1
        for snapshots in fields.values()
    ):
        raise ValueError("one complete final static snapshot required")
    results = {}
    t0, t1 = spec["temperatures_k"]
    for key, snapshots in fields.items():
        values = snapshots[0]["values"]
        expected_ids = (
            {(tag,) for tag in nodes}
            if key in {"temperature", "displacement"}
            else {(tag, ip) for tag in cells for ip in range(1, 9)}
        )
        if set(values) != expected_ids:
            raise ValueError("native fields omit nodes/integration points")
        if key == "temperature":
            error = max(
                abs(
                    v[0]
                    - (
                        t0
                        + (t1 - t0) * (nodes[tag[0]][0] - origin[0]) / spec["size_m"][0]
                    )
                )
                for tag, v in values.items()
            ) / abs(t1 - t0)
            reference, unit = (
                "T(x)=Tleft+(Tright-Tleft)*x/L; other faces adiabatic"
                if not any(origin)
                else "T(x)=Tleft+(Tright-Tleft)*(x-XMIN)/L; approved world-space origin, other faces adiabatic",
                "K",
            )
        elif key == "heat_flux":
            flux = -spec["conductivity_w_m_k"] * (t1 - t0) / spec["size_m"][0]
            error = max(
                max(abs(v[0] - flux), abs(v[1]), abs(v[2])) for v in values.values()
            ) / abs(flux)
            reference, unit = "q=-k grad(T)", "W/m2"
        elif key == "displacement":
            strain = spec["expansion_per_k"] * (t1 - t0)
            error = max(
                abs(v[c] - strain * (nodes[tag[0]][c] - origin[c]))
                for tag, v in values.items()
                for c in range(3)
            ) / (abs(strain) * max(spec["size_m"]))
            reference, unit = (
                "u=alpha*dT*x; symmetry faces remove rigid-body modes"
                if not any(origin)
                else "u=alpha*dT*(x-origin); approved XMIN/YMIN/ZMIN symmetry faces remove rigid-body modes",
                "m",
            )
        else:
            error = max(
                abs(v) for values_row in values.values() for v in values_row
            ) / (spec["young_modulus_pa"] * abs(spec["expansion_per_k"] * (t1 - t0)))
            reference, unit = "zero stress in uniform isotropic free expansion", "Pa"
        if not math.isfinite(error) or error > spec["numerical_tolerance"]:
            raise ValueError(f"{key} analytical gate failed: {error}")
        results[key] = {
            "passed": True,
            "normalized_max_abs_error": error,
            "tolerance": spec["numerical_tolerance"],
            "reference": reference,
            "unit": unit,
            "samples": len(values),
        }
    return results


def cpu_sandbox(policy_variable, expected_policy, closure_path, request_path):
    if os.environ.get(policy_variable):
        if os.environ[policy_variable] != expected_policy:
            raise ValueError("exact CPU solver sandbox policy required")
        mounts = set(read_regular(closure_path, 2 * 1024**2).decode().splitlines())
        if set(map(str, Path("/nix/store").iterdir())) != mounts:
            raise ValueError("FEM sandbox must expose only its operation closure")
        checks = {
            "operation_closure_only": True,
            "no_gpu_nodes": not Path("/dev/dri").exists()
            and not Path("/dev/kfd").exists(),
            "no_sysfs": not Path("/sys").exists(),
            "no_host_home": set(Path("/home").iterdir()) == {Path("/home/worker")},
            "no_session_bus": not Path("/run/user").exists()
            and "DBUS_SESSION_BUS_ADDRESS" not in os.environ,
            "no_worker_socket": not list(Path("/work").glob("*.sock")),
            "network_namespace_isolated": os.readlink("/proc/self/ns/net")
            != os.environ["HARBOR_CAD_HOST_NETNS"],
            "descriptor_readonly": os.statvfs(request_path).f_flag & os.ST_RDONLY != 0,
        }
        if not all(checks.values()):
            raise ValueError("CPU FEM sandbox boundary failed")
        return {"policy": expected_policy, "checks": checks}
    return None


def main():
    if len(sys.argv) != 3 or sys.argv[1] != "reference":
        raise ValueError("usage: harbor-cad-fem reference request.json")
    raw = read_regular(sys.argv[2], 1024**2)
    spec = strict_json(raw)
    validate(spec)
    sandbox = cpu_sandbox(
        "HARBOR_CAD_FEM_POLICY",
        "harbor-cad-fem-cpu-v1",
        "/fem-runtime-closure.txt",
        sys.argv[2],
    )
    if any(Path.cwd().glob("reference.*")) or any(Path.cwd().glob("mesh.json*")):
        raise ValueError("new stage-local FEM directory required")
    start = time.monotonic()
    nodes, cells, sets = mesh(spec)
    Path("reference.inp").write_text(deck(spec, nodes, cells, sets))
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
        solver = subprocess.run(
            ["@calculix@", "-i", "reference"],
            env=environment,
            stdout=log,
            stderr=subprocess.STDOUT,
            timeout=120,
            check=False,
        )
    output = read_regular("calculix.log", 16 * 1024**2).decode()
    if (
        solver.returncode
        or "*ERROR" in output
        or "Version @ccx_version@" not in output
        or "Job finished" not in output
    ):
        raise ValueError("exact native CalculiX reference did not succeed")
    data = read_regular("reference.dat", 32 * 1024**2).decode()
    fields = read_dat(data)
    checks = verify(spec, nodes, cells, fields)
    serialized = {
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
    }
    atomic_json(
        "fields.json",
        {
            "schema_version": 1,
            "static": True,
            "coordinate_unit": "m",
            "fields": serialized,
        },
    )
    receipt = {
        "schema_version": 1,
        "adapter": "CalculiX",
        "backend": "cpu",
        "factorization": "SPOOLES",
        "executed": True,
        "software_fallback": False,
        "synthetic": True,
        "formulation": spec["mode"],
        "precision": "float64",
        "request_sha256": hashlib.sha256(raw).hexdigest(),
        "calculix_version": "@ccx_version@",
        "calculix_source_sha256": "9c88385c10fb04f5dc6c4e98027a51bebdd8aee3920e05190d6c1dd08357d6e7",
        "gmsh_version": "@gmsh_version@",
        "gmsh_source_sha256": "be3f66f225d27ba9fa014f07e83169285da8a051b0e8ab7103d88066b39bdd3e",
        "mesh_sha256": hashlib.sha256(
            read_regular("mesh.json", 32 * 1024**2)
        ).hexdigest(),
        "native_field_sha256": hashlib.sha256(data.encode()).hexdigest(),
        "nodes": len(nodes),
        "elements": len(cells),
        "numerical_verification": checks,
        "wall_seconds": time.monotonic() - start,
        "peak_process_and_child_rss_bytes": 1024
        * max(
            resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
            resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss,
        ),
        "memory_measurement_scope": "maximum individual self/child RSS; aggregate cgroup peak measured separately",
        "convergence": "static analytical consistency only; no transient/refinement claim",
        "physical_validation": "unqualified",
    }
    if sandbox is not None:
        receipt["sandbox"] = sandbox
    atomic_json("fem-reference-receipt.json", receipt)
    print(json.dumps(receipt, allow_nan=False, indent=2))


if __name__ == "__main__":
    main()
