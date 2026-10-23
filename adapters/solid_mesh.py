"""Conformal multi-region tetrahedral meshes of imported closed BREP solids.

Scope: one closed solid per approved named region, imported with the same
unhealed uniform OCC scaling as cad_mesh, fragmented so touching regions share
conformal interfaces, a bounded element count and quality/volume receipts.
This operation is meshing, not a solve. Contract: docs/enclosure-thermal.md.
"""

import hashlib
import json
import math
import os
import re
from pathlib import Path

FORMULATION = "imported_general_solids"
# CalculiX C3D4/C3D10 faces F1..F4 (corner indices) and their C3D10 midside nodes.
FACES = ((0, 1, 2), (0, 3, 1), (1, 3, 2), (2, 3, 0))
FACE_MIDS = ((4, 5, 6), (7, 8, 4), (8, 9, 5), (9, 7, 6))
EDGES = ((0, 1), (1, 2), (2, 0), (0, 3), (1, 3), (2, 3))
# Gmsh tet10 edge nodes are 01,12,20,30,32,31; CalculiX expects 12,23,31,14,24,34.
GMSH_TET10 = (0, 1, 2, 3, 4, 5, 6, 7, 9, 8)
RAYS = 64
AMBIENT_MIN_ESCAPES = 4
CLASSIFICATION = (
    "per CAD surface: ambient when at least 4 of 64 deterministic hemisphere rays "
    "from its largest boundary triangle escape every meshed solid, else cavity"
)
MAX_MESH_BYTES = 96 * 1024**2
GMSH_SOURCE_SHA256 = "be3f66f225d27ba9fa014f07e83169285da8a051b0e8ab7103d88066b39bdd3e"
TOKEN = re.compile("[A-Za-z0-9_-]{1,128}")


def number(value):
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
    ):
        raise ValueError("finite explicit SI numeric input required")
    return float(value)


def rigid(transform):
    if (
        not isinstance(transform, list)
        or len(transform) != 16
        or [number(v) for v in transform[12:]] != [0.0, 0.0, 0.0, 1.0]
    ):
        raise ValueError("explicit affine source placement required")
    list(map(number, transform))
    r = [transform[i : i + 3] for i in (0, 4, 8)]
    for i in range(3):
        for j in range(3):
            if abs(sum(r[i][k] * r[j][k] for k in range(3)) - (i == j)) > 1e-12:
                raise ValueError("rigid original FreeCAD placement required")
    a, b, c = r
    det = (
        a[0] * (b[1] * c[2] - b[2] * c[1])
        - a[1] * (b[0] * c[2] - b[2] * c[0])
        + a[2] * (b[0] * c[1] - b[1] * c[0])
    )
    if abs(det - 1.0) > 1e-12:
        raise ValueError("proper original FreeCAD placement required")


def validate(spec):
    keys = {
        "schema_version",
        "synthetic",
        "backend",
        "formulation",
        "geometry_provenance",
        "source_unit",
        "scale_to_m",
        "placement_translation_unit",
        "regions",
        "mesh",
    }
    if (
        not isinstance(spec, dict)
        or set(spec) != keys
        or type(spec["schema_version"]) is not int
        or spec["schema_version"] != 1
        or type(spec["synthetic"]) is not bool
        or spec["backend"] != "cpu"
        or spec["formulation"] != FORMULATION
        or spec["source_unit"] != "mm"
        or number(spec["scale_to_m"]) != 0.001
        or spec["placement_translation_unit"] != "mm"
        or not isinstance(spec["geometry_provenance"], str)
        or not spec["geometry_provenance"].strip()
        or len(spec["geometry_provenance"].encode()) > 4096
    ):
        raise ValueError("exact imported general-solid CPU mesh descriptor required")
    regions = spec["regions"]
    if not isinstance(regions, list) or not 1 <= len(regions) <= 8:
        raise ValueError("one to eight approved named solids required")
    names = []
    for region in regions:
        if (
            not isinstance(region, dict)
            or set(region)
            != {
                "name",
                "brep_sha256",
                "brep_bytes",
                "bounds_m",
                "volume_m3",
                "source_transform",
            }
            or not isinstance(region["name"], str)
            or not TOKEN.fullmatch(region["name"])
            or not isinstance(region["brep_sha256"], str)
            or not re.fullmatch("[0-9a-f]{64}", region["brep_sha256"])
            or type(region["brep_bytes"]) is not int
            or not 1 <= region["brep_bytes"] <= 64 * 1024**2
            or not isinstance(region["bounds_m"], list)
            or len(region["bounds_m"]) != 6
        ):
            raise ValueError(
                "exact named region BREP identity and world bounds required"
            )
        bounds = list(map(number, region["bounds_m"]))
        sizes = [bounds[i + 1] - bounds[i] for i in (0, 2, 4)]
        volume = number(region["volume_m3"])
        if min(sizes) <= 0 or not 0 < volume <= math.prod(sizes) * (1 + 1e-9):
            raise ValueError("positive solid volume inside its world bounds required")
        rigid(region["source_transform"])
        names.append(region["name"])
    if names != sorted(set(names)):
        raise ValueError("unique sorted region names required")
    mesh = spec["mesh"]
    if (
        not isinstance(mesh, dict)
        or set(mesh)
        != {
            "characteristic_length_max_m",
            "element_order",
            "max_elements",
            "volume_relative_tolerance",
            "min_quality",
            "geometry_tolerance_m",
        }
        or not 1e-4 <= number(mesh["characteristic_length_max_m"]) <= 1.0
        or type(mesh["element_order"]) is not int
        or mesh["element_order"] not in (1, 2)
        or type(mesh["max_elements"]) is not int
        or not 1 <= mesh["max_elements"] <= 2_000_000
        or not 0 < number(mesh["volume_relative_tolerance"]) <= 0.1
        or not 0 < number(mesh["min_quality"]) < 1
        or not 1e-9 <= number(mesh["geometry_tolerance_m"]) <= 1e-3
    ):
        raise ValueError("bounded explicit mesh size, order, budget and gates required")


def sub(a, b):
    return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]


def dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def cross(a, b):
    return [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]


def tet_volume(a, b, c, d):
    return dot(sub(b, a), cross(sub(c, a), sub(d, a))) / 6.0


def shape_ratio(points):
    """6*sqrt(2)*V/l_rms**3 from corner nodes: 1 for a regular tetrahedron."""
    rms = math.sqrt(
        sum(dot(sub(points[i], points[j]), sub(points[i], points[j])) for i, j in EDGES)
        / 6.0
    )
    return 6.0 * math.sqrt(2.0) * tet_volume(*points[:4]) / rms**3


def face_area_normal(nodes, element, face):
    """Area and unit outward normal of corner face `face` (0-based) of a tet."""
    a, b, c = (nodes[element[i]] for i in FACES[face])
    opposite = nodes[element[6 - sum(FACES[face])]]
    n = cross(sub(b, a), sub(c, a))
    length = math.sqrt(dot(n, n))
    if length == 0:
        raise ValueError("degenerate boundary face")
    sign = -1.0 if dot(n, sub(opposite, a)) > 0 else 1.0
    return length / 2.0, [sign * v / length for v in n]


def face_points(element, face, order):
    """Node ids at the face's three edge midpoints (pairs for linear faces)."""
    corners = FACES[face]
    if order == 2:
        return [(element[m],) for m in FACE_MIDS[face]]
    return [
        (element[corners[0]], element[corners[1]]),
        (element[corners[1]], element[corners[2]]),
        (element[corners[2]], element[corners[0]]),
    ]


def faces(elements):
    """Corner-sorted face key -> [(element id, CalculiX face index 1..4)]."""
    result = {}
    for eid, element in elements.items():
        for index, corners in enumerate(FACES):
            key = tuple(sorted(element[i] for i in corners))
            result.setdefault(key, []).append((eid, index + 1))
    if any(len(owners) > 2 for owners in result.values()):
        raise ValueError("non-manifold mesh face shared by more than two elements")
    return result


def hemisphere(normal, count=RAYS):
    """Deterministic equal-solid-angle Fibonacci directions about `normal`."""
    helper = [1.0, 0.0, 0.0] if abs(normal[0]) < 0.9 else [0.0, 1.0, 0.0]
    t1 = cross(normal, helper)
    scale = math.sqrt(dot(t1, t1))
    t1 = [v / scale for v in t1]
    t2 = cross(normal, t1)
    golden = math.pi * (3.0 - math.sqrt(5.0))
    result = []
    for i in range(count):
        z = 1.0 - (i + 0.5) / count
        r = math.sqrt(max(0.0, 1.0 - z * z))
        x, y = r * math.cos(i * golden), r * math.sin(i * golden)
        result.append([x * t1[k] + y * t2[k] + z * normal[k] for k in range(3)])
    return result


class RayGrid:
    """Uniform-grid ray occlusion against triangles (3D DDA + Moller-Trumbore)."""

    def __init__(self, triangles, cells=48):
        self.triangles = triangles
        self.low = [min(p[k] for t in triangles for p in t) for k in range(3)]
        high = [max(p[k] for t in triangles for p in t) for k in range(3)]
        span = max(h - lo for h, lo in zip(high, self.low, strict=True))
        self.size = span / cells
        self.count = [int((high[k] - self.low[k]) / self.size) + 1 for k in range(3)]
        self.eps = 1e-9 * span
        self.cells = {}
        for index, triangle in enumerate(triangles):
            lo = [self.cell(min(p[k] for p in triangle), k) for k in range(3)]
            hi = [self.cell(max(p[k] for p in triangle), k) for k in range(3)]
            for i in range(lo[0], hi[0] + 1):
                for j in range(lo[1], hi[1] + 1):
                    for k in range(lo[2], hi[2] + 1):
                        self.cells.setdefault((i, j, k), []).append(index)

    def cell(self, value, axis):
        index = int((value - self.low[axis]) / self.size)
        return min(max(index, 0), self.count[axis] - 1)

    def blocked(self, origin, direction):
        index = [self.cell(origin[k], k) for k in range(3)]
        step, boundary, delta = [], [], []
        for k in range(3):
            if direction[k] > 0:
                step.append(1)
                edge = self.low[k] + (index[k] + 1) * self.size
            else:
                step.append(-1)
                edge = self.low[k] + index[k] * self.size
            if direction[k] == 0:
                boundary.append(math.inf)
                delta.append(math.inf)
            else:
                boundary.append((edge - origin[k]) / direction[k])
                delta.append(self.size / abs(direction[k]))
        tested = set()
        while all(0 <= index[k] < self.count[k] for k in range(3)):
            for t in self.cells.get(tuple(index), ()):
                if t not in tested:
                    tested.add(t)
                    if self.hit(origin, direction, self.triangles[t]):
                        return True
            axis = min(range(3), key=boundary.__getitem__)
            index[axis] += step[axis]
            boundary[axis] += delta[axis]
        return False

    def hit(self, origin, direction, triangle):
        a, b, c = triangle
        e1, e2 = sub(b, a), sub(c, a)
        p = cross(direction, e2)
        det = dot(e1, p)
        if abs(det) < 1e-300:
            return False
        inverse = 1.0 / det
        s = sub(origin, a)
        u = dot(s, p) * inverse
        if u < 0.0 or u > 1.0:
            return False
        q = cross(s, e1)
        v = dot(direction, q) * inverse
        if v < 0.0 or u + v > 1.0:
            return False
        return dot(e2, q) * inverse > self.eps


def classify(nodes, elements, boundary, surface_of):
    """Label every boundary face's CAD surface `ambient` or `cavity`.

    ponytail: one sample point per CAD surface and a fixed escape threshold; a
    surface that is partly shaded is labelled as a whole. Split surfaces or
    weight by view factor if a geometry needs finer exterior selection.
    """
    triangles = [
        [nodes[elements[eid][i]] for i in FACES[face - 1]] for eid, face in boundary
    ]
    grid = RayGrid(triangles)
    largest = {}
    for (eid, face), triangle in zip(boundary, triangles, strict=True):
        area, normal = face_area_normal(nodes, elements[eid], face - 1)
        surface = surface_of[(eid, face)]
        if surface not in largest or area > largest[surface][0]:
            centroid = [sum(p[k] for p in triangle) / 3.0 for k in range(3)]
            largest[surface] = (area, centroid, normal)
    labels = {}
    for surface, (_, centroid, normal) in sorted(largest.items()):
        origin = [centroid[k] + grid.eps * 100 * normal[k] for k in range(3)]
        escapes = sum(
            not grid.blocked(origin, direction) for direction in hemisphere(normal)
        )
        labels[surface] = "ambient" if escapes >= AMBIENT_MIN_ESCAPES else "cavity"
    return labels


def import_regions(gmsh, spec, inputs):
    # Same unhealed uniform gp_Trsf import scaling as cad_mesh (docs/cad-mesh.md).
    for name in (
        "FixDegenerated",
        "FixSmallEdges",
        "FixSmallFaces",
        "SewFaces",
        "MakeSolids",
    ):
        gmsh.option.setNumber("Geometry.OCC" + name, 0)
    gmsh.option.setNumber("Geometry.OCCScaling", spec["scale_to_m"])
    tolerance = spec["mesh"]["geometry_tolerance_m"]
    tags = []
    for region in spec["regions"]:
        bodies = gmsh.model.occ.importShapes(
            str(Path(inputs) / f"{region['name']}.brep"),
            highestDimOnly=True,
            format="brep",
        )
        if len(bodies) != 1 or bodies[0][0] != 3:
            raise ValueError(
                f"region {region['name']} must be exactly one closed solid"
            )
        gmsh.model.occ.synchronize()
        tag = bodies[0][1]
        box = gmsh.model.getBoundingBox(3, tag)
        world = [box[0], box[3], box[1], box[4], box[2], box[5]]
        if any(
            abs(a - b) > tolerance
            for a, b in zip(world, region["bounds_m"], strict=True)
        ) or not math.isclose(
            gmsh.model.occ.getMass(3, tag), region["volume_m3"], rel_tol=1e-6
        ):
            raise ValueError(f"region {region['name']} BREP differs from CAD evidence")
        tags.append(tag)
    return tags


def generate(spec, gmsh, inputs="/inputs"):
    """Mesh the approved solids; return the mesh record and receipt facts."""
    mesh_spec = spec["mesh"]
    order = mesh_spec["element_order"]
    gmsh.initialize(["harbor-cad-solid-mesh", "-nopopup"], readConfigFiles=False)
    try:
        gmsh.option.setNumber("General.Terminal", 0)
        gmsh.option.setNumber("General.NumThreads", 1)
        for option in ("MaxNumThreads1D", "MaxNumThreads2D", "MaxNumThreads3D"):
            gmsh.option.setNumber("Mesh." + option, 1)
        gmsh.option.setNumber(
            "Mesh.CharacteristicLengthMax", mesh_spec["characteristic_length_max_m"]
        )
        gmsh.option.setNumber("Mesh.ElementOrder", order)
        gmsh.option.setNumber("Mesh.Optimize", 1)
        gmsh.option.setNumber("Mesh.OptimizeNetgen", 1)
        gmsh.option.setNumber("Mesh.Algorithm3D", 10)
        gmsh.model.add("imported-solids")
        tags = import_regions(gmsh, spec, inputs)
        _, mapping = gmsh.model.occ.fragment(
            [(3, tags[0])], [(3, tag) for tag in tags[1:]]
        )
        gmsh.model.occ.synchronize()
        owner = {}
        for region, pieces in zip(spec["regions"], mapping, strict=True):
            for dim, tag in pieces:
                if dim != 3 or owner.setdefault(tag, region["name"]) != region["name"]:
                    raise ValueError(
                        "approved regions overlap; no shared volume allowed"
                    )
        if set(owner) != {tag for _, tag in gmsh.model.getEntities(3)}:
            raise ValueError("fragmentation produced an unowned volume")
        gmsh.model.mesh.generate(3)
        node_tags, coordinates, _ = gmsh.model.mesh.getNodes()
        renumber = {int(t): i + 1 for i, t in enumerate(sorted(map(int, node_tags)))}
        position = {
            int(t): [float(v) for v in coordinates[3 * i : 3 * i + 3]]
            for i, t in enumerate(node_tags)
        }
        nodes = {renumber[t]: xyz for t, xyz in position.items()}
        expected_type, width = (4, 4) if order == 1 else (11, 10)
        elements, regions, native = {}, {}, []
        for region in spec["regions"]:
            ids = []
            for volume in sorted(
                t for t, name in owner.items() if name == region["name"]
            ):
                types, element_tags, connectivity = gmsh.model.mesh.getElements(
                    3, volume
                )
                if list(types) != [expected_type]:
                    raise ValueError("only tetrahedra of the approved order allowed")
                for i, tag in enumerate(element_tags[0]):
                    raw = connectivity[0][width * i : width * i + width]
                    ordered = raw if order == 1 else [raw[j] for j in GMSH_TET10]
                    eid = len(elements) + 1
                    elements[eid] = [renumber[int(n)] for n in ordered]
                    native.append(int(tag))
                    ids.append(eid)
                    if len(elements) > mesh_spec["max_elements"]:
                        raise ValueError("mesh exceeds the approved element budget")
            regions[region["name"]] = ids
        sicn = list(gmsh.model.mesh.getElementQualities(native, "minSICN"))
        surface_of_key = {}
        for _, surface in gmsh.model.getEntities(2):
            _, _, connectivity = gmsh.model.mesh.getElements(2, surface)
            step = 3 if order == 1 else 6
            for block in connectivity:
                for i in range(0, len(block), step):
                    key = tuple(sorted(renumber[int(n)] for n in block[i : i + 3]))
                    surface_of_key[key] = surface
    finally:
        gmsh.finalize()
    return assemble(spec, nodes, elements, regions, surface_of_key, sicn)


def assemble(spec, nodes, elements, regions, surface_of_key, sicn):
    """Quality, volume, interface and exterior facts for a renumbered mesh."""
    mesh_spec = spec["mesh"]
    order = mesh_spec["element_order"]
    region_of = {eid: name for name, ids in regions.items() for eid in ids}
    ratios, volumes = [], dict.fromkeys(regions, 0.0)
    for eid, element in elements.items():
        points = [nodes[n] for n in element]
        volume = tet_volume(*points[:4])
        if not volume > 0:
            raise ValueError("inverted or degenerate tetrahedron")
        if order == 2:
            for (a, b), mid in zip(EDGES, points[4:], strict=True):
                centre = [(points[a][k] + points[b][k]) / 2 for k in range(3)]
                if math.dist(centre, mid) > 0.25 * math.dist(points[a], points[b]):
                    raise ValueError("C3D10 midside node ordering changed")
        ratios.append(shape_ratio(points))
        volumes[region_of[eid]] += volume
    owners = faces(elements)
    boundary = sorted(owner[0] for owner in owners.values() if len(owner) == 1)
    interfaces = {}
    for key, owner in owners.items():
        if len(owner) == 2:
            first, second = (region_of[eid] for eid, _ in owner)
            if first != second:
                pair = "|".join(sorted((first, second)))
                area, _ = face_area_normal(
                    nodes, elements[owner[0][0]], owner[0][1] - 1
                )
                interfaces[pair] = interfaces.get(pair, 0.0) + area
    surface_of = {}
    for eid, face in boundary:
        key = tuple(sorted(elements[eid][i] for i in FACES[face - 1]))
        if key not in surface_of_key:
            raise ValueError("boundary face is not on an imported CAD surface")
        surface_of[(eid, face)] = surface_of_key[key]
    labels = classify(nodes, elements, boundary, surface_of)
    boundary_faces = [
        [eid, face, labels[surface_of[(eid, face)]]] for eid, face in boundary
    ]
    exposure = {"ambient": {}, "cavity": {}}
    for eid, face, label in boundary_faces:
        area, _ = face_area_normal(nodes, elements[eid], face - 1)
        name = region_of[eid]
        exposure[label][name] = exposure[label].get(name, 0.0) + area
    if not exposure["ambient"]:
        raise ValueError("no ambient exterior surface was identified")
    min_ratio = min(ratios)
    if min_ratio < mesh_spec["min_quality"] or min(sicn) <= 0:
        raise ValueError("mesh quality below the approved gate")
    region_facts = {}
    for region in spec["regions"]:
        name = region["name"]
        error = abs(volumes[name] - region["volume_m3"]) / region["volume_m3"]
        if error > mesh_spec["volume_relative_tolerance"] or not regions[name]:
            raise ValueError(f"region {name} mesh volume differs from CAD volume")
        region_facts[name] = {
            "brep_sha256": region["brep_sha256"],
            "cad_volume_m3": region["volume_m3"],
            "mesh_volume_m3": volumes[name],
            "relative_volume_error": error,
            "elements": len(regions[name]),
        }
    record = {
        "schema_version": 1,
        "synthetic": spec["synthetic"],
        "coordinate_unit": "m",
        "element_type": "C3D4" if order == 1 else "C3D10",
        "nodes": nodes,
        "elements": elements,
        "regions": regions,
        "boundary_faces": boundary_faces,
        "classification": CLASSIFICATION,
    }
    facts = {
        "regions": region_facts,
        "nodes": len(nodes),
        "elements": len(elements),
        "element_type": record["element_type"],
        "quality": {
            "min_shape_ratio": min_ratio,
            "mean_shape_ratio": sum(ratios) / len(ratios),
            "gmsh_min_sicn": min(sicn),
            "gmsh_mean_sicn": sum(sicn) / len(sicn),
        },
        "interfaces_m2": dict(sorted(interfaces.items())),
        "boundary_area_m2": exposure,
        "classification": CLASSIFICATION,
    }
    return record, facts


def write_mesh(record):
    data = json.dumps(record, allow_nan=False, separators=(",", ":")).encode()
    if len(data) > MAX_MESH_BYTES:
        raise ValueError("mesh record exceeds its bounded output size")
    partial = Path("mesh.json.partial")
    with partial.open("xb") as stream:
        stream.write(data)
    partial.replace("mesh.json")
    return hashlib.sha256(data).hexdigest()


def check_sources(spec, fem, sandbox, inputs="/inputs"):
    if sandbox is not None:
        names = {f"{r['name']}.brep" for r in spec["regions"]}
        sandbox["checks"]["source_brep_readonly"] = all(
            os.statvfs(Path(inputs) / name).f_flag & os.ST_RDONLY != 0 for name in names
        )
        sandbox["checks"]["named_source_only"] = {
            p.name for p in Path(inputs).iterdir()
        } == names
        if not all(sandbox["checks"].values()):
            raise ValueError("imported solid source sandbox boundary failed")
    originals = {}
    for region in spec["regions"]:
        raw = fem.read_regular(
            str(Path(inputs) / f"{region['name']}.brep"), 64 * 1024**2
        )
        if (
            len(raw) != region["brep_bytes"]
            or hashlib.sha256(raw).hexdigest() != region["brep_sha256"]
        ):
            raise ValueError(f"approved BREP bytes for {region['name']} changed")
        originals[region["name"]] = raw
    return originals


def recheck_sources(spec, fem, originals, inputs="/inputs"):
    for name, raw in originals.items():
        if fem.read_regular(str(Path(inputs) / f"{name}.brep"), 64 * 1024**2) != raw:
            raise ValueError("original imported BREP changed during the stage")


def receipt(spec, raw, mesh_sha256, facts, sandbox):
    return {
        "schema_version": 1,
        "adapter": "Gmsh",
        "backend": "cpu",
        "executed": True,
        "software_fallback": False,
        "precision": "float64",
        "formulation": FORMULATION,
        "gmsh_version": "@gmsh_version@",
        "gmsh_source_sha256": GMSH_SOURCE_SHA256,
        "request_sha256": hashlib.sha256(raw).hexdigest(),
        "mesh_sha256": mesh_sha256,
        "synthetic": spec["synthetic"],
        "geometry_provenance": spec["geometry_provenance"],
        "source_unit": "mm",
        "coordinate_unit": "m",
        "scale_to_m": 0.001,
        "placement_translation_unit": "mm",
        "fragmented": True,
        **facts,
        "gap_healing": False,
        "physical_validation": "unqualified",
        **({"sandbox": sandbox} if sandbox is not None else {}),
    }


def main(raw, spec, fem, request_path):
    """CadMesh stage entry for the general-solid descriptor (from cad_mesh.py)."""
    validate(spec)
    sandbox = fem.cpu_sandbox(
        "HARBOR_CAD_CAD_MESH_POLICY",
        "harbor-cad-cad-mesh-cpu-v1",
        "/cad-mesh-runtime-closure.txt",
        request_path,
    )
    originals = check_sources(spec, fem, sandbox)
    if any(p.name != "mesh.log" for p in Path.cwd().iterdir()):
        raise ValueError("new empty stage-local CAD mesh directory required")
    record, facts = generate(spec, fem.gmsh_api())
    mesh_sha256 = write_mesh(record)
    recheck_sources(spec, fem, originals)
    fem.atomic_json(
        "cad-mesh-receipt.json", receipt(spec, raw, mesh_sha256, facts, sandbox)
    )
