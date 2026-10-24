"""Analytical references for the enclosure thermal recipe; CPU, CI-runnable.

Without --ccx an independent pure-Python linear-tetrahedron solver runs the
recipe's own boundary model, secant air-node search, energy balance and
statistics (adapters/enclosure_thermal.py) on structured meshes and must match
four analytic solutions at two refinements: a 1D slab with uniform volumetric
heat and film on both faces, a lumped (Bi=1e-4) cube with film on every face,
a two-region series conductance and a lumped internal-air chain. With
``--ccx PATH --ccx-version V`` (opt-in native gate) the identical CalculiX deck
the stage writes is solved instead and gated against the same references.
"""

import argparse
import importlib.util
import itertools
import json
import math
import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SIGMA = 5.670374419e-8


def adapter(name):
    source = importlib.util.spec_from_file_location(name, ROOT / f"adapters/{name}.py")
    module = importlib.util.module_from_spec(source)
    source.loader.exec_module(module)
    return module


solid = adapter("solid_mesh")
enclosure = adapter("enclosure_thermal")


def box_mesh(origin, size, cells, region_of_x, first_node=1, first_element=1):
    """Kuhn (conforming, 6 tets per hex) structured mesh of an axis box."""
    nx, ny, nz = cells
    nodes, ids = {}, {}
    for i, j, k in itertools.product(range(nx + 1), range(ny + 1), range(nz + 1)):
        ids[(i, j, k)] = first_node + len(nodes)
        nodes[ids[(i, j, k)]] = [
            origin[0] + size[0] * i / nx,
            origin[1] + size[1] * j / ny,
            origin[2] + size[2] * k / nz,
        ]
    elements, regions = {}, {}
    for i, j, k in itertools.product(range(nx), range(ny), range(nz)):
        for order in itertools.permutations(range(3)):
            corner = [i, j, k]
            path = [ids[tuple(corner)]]
            for axis in order:
                corner[axis] += 1
                path.append(ids[tuple(corner)])
            if solid.tet_volume(*(nodes[n] for n in path)) < 0:
                path[1], path[2] = path[2], path[1]
            eid = first_element + len(elements)
            elements[eid] = path
            centre = sum(nodes[n][0] for n in path) / 4
            regions.setdefault(region_of_x(centre), []).append(eid)
    return nodes, elements, regions


def record(nodes, elements, regions, labelled):
    """Mesh record with only the listed boundary faces (others adiabatic)."""
    owners = solid.faces(elements)
    boundary = sorted(owner[0] for owner in owners.values() if len(owner) == 1)
    faces = []
    for eid, face in boundary:
        points = [nodes[elements[eid][i]] for i in solid.FACES[face - 1]]
        label = labelled(points)
        if label is not None:
            faces.append([eid, face, label])
    return {
        "element_type": "C3D4",
        "nodes": nodes,
        "elements": elements,
        "regions": regions,
        "boundary_faces": faces,
    }


def on_plane(axis, value):
    return lambda points: all(abs(p[axis] - value) < 1e-12 for p in points)


def material(k, emissivity=None, absorptance=None):
    return {
        "source": "declared",
        "conductivity_w_m_k": k,
        "emissivity": emissivity,
        "solar_absorptance": absorptance,
        "provenance": "synthetic analytical reference value",
        "synthetic": True,
    }


def thermal(materials, source, power, exposed, film=50.0, internal=10.0):
    return {
        "synthetic": True,
        "materials": materials,
        "heat_source_region": source,
        "heat_source_power_w": power,
        "ambient_k": 300.0,
        "film_w_m2_k": film,
        "exposed_regions": exposed,
        "internal_film_w_m2_k": internal,
        "solar": None,
        "radiation": False,
        "energy_balance_tolerance": 1e-3,
        "provenance": {"all": "synthetic analytical reference inputs"},
    }


def gradients(points):
    a = [solid.sub(points[i], points[0]) for i in (1, 2, 3)]
    det = solid.dot(a[0], solid.cross(a[1], a[2]))
    rows = [
        [v / det for v in solid.cross(a[1], a[2])],
        [v / det for v in solid.cross(a[2], a[0])],
        [v / det for v in solid.cross(a[0], a[1])],
    ]
    return [[-sum(r[k] for r in rows) for k in range(3)], *rows], det / 6


def reference_solver(rec, model_, data):
    """Independent linear-tet Galerkin solve with the recipe's face quadrature.

    Element stiffness uses the diagonal conductivity tensor (isotropic or
    orthotropic). Face terms use the same three edge-midpoint points (weight
    A/3) as the energy balance; the linearized radiation bound adds h_r to the
    film, credited cavity faces take the combined air/radiation-node film, and
    T^4 radiation is linearized by Picard iteration.
    """
    nodes, elements = rec["nodes"], rec["elements"]
    order = sorted(nodes)
    index = {n: i for i, n in enumerate(order)}
    base = [{} for _ in order]
    load = [0.0] * len(order)
    for name, ids in rec["regions"].items():
        k = enclosure.conductivity(data["materials"][name])
        q = model_["power_density_w_m3"] if name == data["heat_source_region"] else 0
        for e in ids:
            grads, volume = gradients([nodes[n] for n in elements[e]])
            for a, na in enumerate(elements[e]):
                load[index[na]] += q * volume / 4
                row = base[index[na]]
                for b, nb in enumerate(elements[e]):
                    row[index[nb]] = row.get(index[nb], 0.0) + volume * sum(
                        k[i] * grads[a][i] * grads[b][i] for i in range(3)
                    )
    for e, face, area, flux in model_["sun"]:
        for i in solid.FACES[face - 1]:
            load[index[elements[e][i]]] += flux * area / 3

    def solve(internal_air_k, radiation_node_k=None):
        temperature = [data["ambient_k"]] * len(order)
        for _ in range(100):
            rows = [dict(r) for r in base]
            rhs = list(load)
            faces = [
                (e, f, a, data["film_w_m2_k"], data["ambient_k"], r)
                for e, f, a, r in model_["film"]
            ] + [
                (e, f, a, h, sink, None)
                for e, f, a, h, sink in enclosure.cavity_films(
                    data, model_, internal_air_k, radiation_node_k
                )
            ]
            for e, face, area, h, sink, region in faces:
                corners = [index[elements[e][i]] for i in solid.FACES[face - 1]]
                for p, q in ((0, 1), (1, 2), (2, 0)):
                    pair = (corners[p], corners[q])
                    coefficient = h
                    if region is not None:
                        coefficient += enclosure.radiation_coefficient(data, region)
                    if region is not None and enclosure.nonlinear(data):
                        t = (temperature[pair[0]] + temperature[pair[1]]) / 2
                        emissivity = data["materials"][region]["emissivity"]
                        coefficient += (
                            emissivity * SIGMA * (t * t + sink * sink) * (t + sink)
                        )
                    w = area / 3 * coefficient
                    for i in pair:
                        rhs[i] += w * sink / 2
                        for j in pair:
                            rows[i][j] = rows[i].get(j, 0.0) + w / 4
            updated = conjugate_gradient(rows, rhs, temperature)
            change = max(abs(a - b) for a, b in zip(updated, temperature, strict=True))
            temperature = updated
            if not enclosure.nonlinear(data) or change < 1e-10 * data["ambient_k"]:
                return {n: temperature[index[n]] for n in order}
        raise ValueError("reference Picard iteration did not converge")

    return solve


def conjugate_gradient(rows, rhs, guess):
    columns = [(list(r), list(r.values())) for r in rows]
    diagonal = [r[i] for i, r in enumerate(rows)]

    def product(x):
        return [
            sum(v * x[j] for j, v in zip(c, vals, strict=True)) for c, vals in columns
        ]

    x = list(guess)
    residual = [b - a for a, b in zip(product(x), rhs, strict=True)]
    z = [r / d for r, d in zip(residual, diagonal, strict=True)]
    p = list(z)
    rz = sum(a * b for a, b in zip(residual, z, strict=True))
    scale = math.sqrt(sum(b * b for b in rhs)) or 1.0
    for _ in range(20 * len(rhs) + 100):
        if math.sqrt(sum(r * r for r in residual)) <= 1e-13 * scale:
            return x
        ap = product(p)
        alpha = rz / sum(a * b for a, b in zip(p, ap, strict=True))
        x = [a + alpha * b for a, b in zip(x, p, strict=True)]
        residual = [a - alpha * b for a, b in zip(residual, ap, strict=True)]
        z = [r / d for r, d in zip(residual, diagonal, strict=True)]
        rz, previous = sum(a * b for a, b in zip(residual, z, strict=True)), rz
        p = [a + rz / previous * b for a, b in zip(z, p, strict=True)]
    raise ValueError("reference conjugate gradient did not converge")


def ccx_solver(executable, version):
    fem = adapter("fem_reference")

    def make(rec, model_, data):
        work = Path(tempfile.mkdtemp(prefix="harbor-enclosure-"))

        def run(internal_air_k):
            previous = Path.cwd()
            try:
                os.chdir(work)
                return enclosure.ccx_runner(
                    rec, data, model_, fem, executable, version, "calculix.log"
                )(internal_air_k)
            finally:
                os.chdir(previous)

        return run

    return make


def slab(n):
    length, width = 0.04, 0.01
    nodes, elements, regions = box_mesh(
        [0, 0, 0], [length, width, width], (n, 2, 2), lambda x: "slab"
    )
    rec = record(
        nodes,
        elements,
        regions,
        lambda p: "ambient" if on_plane(0, 0)(p) or on_plane(0, length)(p) else None,
    )
    q, k, h = 1e5, 2.0, 50.0
    data = thermal({"slab": material(k)}, "slab", q * length * width**2, ["slab"], h)

    def exact(xyz):
        x = xyz[0]
        return 300 + q * length / (2 * h) + q * x * (length - x) / (2 * k)

    return rec, data, exact, q * length / (2 * h) + q * length**2 / (8 * k)


def cube(n):
    side = 0.02
    nodes, elements, regions = box_mesh(
        [0, 0, 0], [side] * 3, (n, n, n), lambda x: "cube"
    )
    rec = record(nodes, elements, regions, lambda p: "ambient")
    data = thermal({"cube": material(1e4)}, "cube", 1.0, ["cube"])
    rise = 1.0 / (50.0 * 6 * side**2)
    return rec, data, lambda xyz: 300 + rise, rise


def series(n):
    pcb, housing, width = 0.01, 0.03, 0.01
    total = pcb + housing
    nodes, elements, regions = box_mesh(
        [0, 0, 0],
        [total, width, width],
        (4 * n, 2, 2),
        lambda x: "pcb" if x < pcb else "housing",
    )
    rec = record(
        nodes,
        elements,
        regions,
        lambda p: "ambient" if on_plane(0, total)(p) else None,
    )
    q, k1, k2, h = 1e5, 0.5, 5.0, 50.0
    data = thermal(
        {"housing": material(k2), "pcb": material(k1)},
        "pcb",
        q * pcb * width**2,
        ["housing"],
        h,
    )
    flux = q * pcb
    interface = 300 + flux / h + flux * housing / k2

    def exact(xyz):
        x = xyz[0]
        if x >= pcb:
            return 300 + flux / h + flux * (total - x) / k2
        return interface + q * (pcb**2 - x * x) / (2 * k1)

    return rec, data, exact, exact([0, 0, 0]) - 300


def air_chain(n):
    """Isothermal source block inside a cavity of an isothermal shell block."""
    a, gap = 0.01, 0.01
    inner = box_mesh([0, 0, 0], [a] * 3, (n, n, n), lambda x: "pcb")
    nodes, elements, regions = inner
    outer = box_mesh(
        [a + gap, 0, 0],
        [a, a, a],
        (n, n, n),
        lambda x: "housing",
        first_node=len(nodes) + 1,
        first_element=len(elements) + 1,
    )
    nodes.update(outer[0])
    elements.update(outer[1])
    regions.update(outer[2])
    facing = on_plane(0, a + gap)

    def label(points):
        if all(p[0] <= a + 1e-12 for p in points):
            return "cavity"
        return "cavity" if facing(points) else "ambient"

    rec = record(nodes, elements, regions, label)
    power, h, hi = 0.2, 50.0, 10.0
    data = thermal(
        {"housing": material(1e4), "pcb": material(1e4)},
        "pcb",
        power,
        ["housing"],
        h,
        hi,
    )
    shell = 300 + power / (h * 5 * a * a)
    air = shell + power / (hi * a * a)
    source = air + power / (hi * 6 * a * a)
    return rec, data, (lambda xyz: source if xyz[0] <= a else shell), source - 300


CASES = {
    "slab_uniform_source_film": (slab, (4, 8), 1e-2),
    "lumped_cube_bi_1e-4": (cube, (2, 4), 2e-3),
    "two_region_series_conductance": (series, (2, 4), 2e-2),
    "lumped_internal_air_chain": (air_chain, (1, 2), 2e-3),
}


def run_case(name, make_solver):
    builder, levels, gate = CASES[name]
    errors = []
    results = []
    for n in levels:
        rec, data, exact, rise = builder(n)
        model_ = enclosure.boundary_model(rec, data, solid)
        temperatures, result = enclosure.evaluate(
            rec, data, model_, make_solver(rec, model_, data), solid
        )
        error = (
            max(
                abs(temperatures[node] - exact(xyz))
                for node, xyz in rec["nodes"].items()
            )
            / rise
        )
        errors.append(error)
        results.append(
            {
                "refinement": n,
                "nodes": len(rec["nodes"]),
                "elements": len(rec["elements"]),
                "normalized_max_abs_error": error,
                "energy_relative_error": result["energy_balance"]["relative_error"],
                "max_k": result["global_max"]["temperature_k"],
                "internal_air_k": result["internal_air_k"],
            }
        )
    # Refinement must not increase the error unless both levels are already
    # discretization-free (lumped cases are bounded by Biot, not mesh size).
    passed = errors[-1] <= gate and (
        errors[-1] <= errors[0] * (1 + 1e-6) or errors[0] <= 0.01 * gate
    )
    if not passed:
        raise ValueError(f"{name} analytical gate failed: {errors}")
    return {"gate": gate, "levels": results, "passed": True}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ccx", type=Path)
    parser.add_argument("--ccx-version")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if (args.ccx is None) != (args.ccx_version is None):
        raise ValueError("--ccx and --ccx-version go together")
    make_solver = (
        reference_solver
        if args.ccx is None
        else ccx_solver(str(args.ccx.resolve(strict=True)), args.ccx_version)
    )
    report = {
        "schema_version": 1,
        "solver": "python-linear-tet-reference" if args.ccx is None else "calculix",
        "recipe": enclosure.FORMULATION,
        "cases": {name: run_case(name, make_solver) for name in CASES},
        "physical_validation": "unqualified",
    }
    text = json.dumps(report, indent=2, allow_nan=False)
    if args.output:
        args.output.write_text(text)
    print(text)
    return report


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
