"""Supported PCB power of an enclosure candidate on the reference solver path.

One compatible candidate of examples/design-enclosure.json is solved with the
pure-Python reference solver (no CalculiX) on a structured analogue of the
enclosure fixture (housing walls and bosses, lid, pc-uv window, PCB). Every
environment, material, finish, cavity-film, component-limit and board
conductivity value is the cited conservative end the Rust resolver uses; the
window takes the explicit conservative transparent_solar bound. PCB power is
swept: two linear solves (power_sweep mode, radiation off or its linearized
lower bound) at the swept ends give the maximum supported power by linear
superposition, mirrored here from design_providers::supported_power. The
board may be orthotropic (cited in-plane and through-plane low ends).
Nothing is synthetic. The analogue omits
the fins (less exterior area) and the gasket; it is a demonstration, not
design evidence.
"""

import importlib.util
import itertools
import json
import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CANDIDATE = "al-6061-t6__cnc-3axis-al__powder-coat-white"
AFFINITY_TOLERANCE_K = 0.5  # design_providers::AFFINITY_TOLERANCE_K


def load(name, path):
    source = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(source)
    source.loader.exec_module(module)
    return module


solid = load("solid_mesh", "adapters/solid_mesh.py")
enclosure = load("enclosure_thermal", "adapters/enclosure_thermal.py")
gate = load("verify_enclosure_thermal_cpu", "scripts/verify_enclosure_thermal_cpu.py")


def data(name):
    return json.loads((ROOT / "data" / name).read_text())


def envelope():
    return json.loads((ROOT / "examples/design-enclosure.json").read_text())


def end(value, high):
    """Conservative end of a cited range in SI (only the units used here)."""
    assert value["availability"] == "known" and not value["synthetic"]
    side = value["value"]["high" if high else "low"]
    offset = {"degC": 273.15}.get(side["unit"], 0.0)
    assert side["unit"] in {"degC", "1", "W/(m*K)", "W/(m2*K)", "W/m2"}
    return side["value"] + offset


def by_id(items, key):
    return next(item for item in items if item["id"] == key)


def declared(name):
    source = next(m for m in envelope()["metric_sources"] if m["metric"] == name)
    return source["source"]["input"]["value"]


def sourced():
    materials, finishes = data("materials.json")["materials"], data("finishes.json")
    conditions = data("conditions.json")
    climate = by_id(conditions["classes"], "etsi-4-1")
    still = next(f for f in conditions["films"] if f["wind_speed"] is None)
    electronics = conditions["electronics"]
    material, _, finish = CANDIDATE.split("__")
    metal = by_id(materials, material)
    coat = by_id(finishes["finishes"], finish)
    glazing = by_id(materials, "pc-uv")
    assert "pc-uv" in finishes["transparent_materials"]
    return {
        "ambient_k": end(climate["high_air_temperature"], True),
        "cold_k": end(climate["low_air_temperature"], False),
        "solar_w_m2": end(climate["solar_irradiance"], True),
        "film": end(still["coefficient"], False),
        "internal_film": end(
            by_id(conditions["internal_films"], "enclosed-cavity-natural")[
                "coefficient"
            ],
            False,
        ),
        # Cited conservative defaults, not product data (the envelope's keys).
        "component_k": end(
            by_id(electronics, "component-limit-industrial")["value"], False
        ),
        "board_k": end(
            by_id(electronics, "pcb-fr4-copper-through-plane")["value"], False
        ),
        "board_in_plane_k": end(
            by_id(electronics, "pcb-fr4-copper-in-plane")["value"], False
        ),
        "metal": {
            "k": end(metal["thermal_conductivity"], False),
            "max_k": end(metal["max_service_temperature"], False),
            "min_k": end(metal["min_service_temperature"], True),
            "emissivity": end(coat["emissivity"], False),
            "absorptance": end(coat["solar_absorptance"], True),
        },
        "glazing": {
            "k": end(glazing["thermal_conductivity"], False),
            "emissivity": end(glazing["emissivity"], False),
        },
        "pa12_min_k": end(
            by_id(materials, "pa12-mjf")["min_service_temperature"], True
        ),
    }


def material(k, emissivity, absorptance, provenance):
    return {
        "source": "library",
        "conductivity_w_m_k": k,
        "emissivity": emissivity,
        "solar_absorptance": absorptance,
        "provenance": provenance,
        "synthetic": False,
    }


XS = [0, 0.004, 0.07, 0.076, 0.124, 0.13, 0.196, 0.2]
YS = [0, 0.004, 0.045, 0.051, 0.099, 0.105, 0.146, 0.15]
ZS = [0, 0.004, 0.012, 0.014, 0.076, 0.08]


def region(i, j, k, window):
    """Cell region; None is the internal air cavity (no elements)."""
    inner = 0 < i < len(XS) - 2 and 0 < j < len(YS) - 2
    if k == len(ZS) - 2:
        return "window" if window and i in (3,) and j in (3,) else "lid"
    if not inner or k == 0:
        return "housing"
    if k == 1 and i in (2, 4) and j in (2, 4):
        return "housing"  # 6 mm bosses under the board corners
    if k == 2 and 2 <= i <= 4 and 2 <= j <= 4:
        return "pcb"
    return None


def analogue(window=True):
    """Kuhn (conforming) tetrahedra on the rectilinear grid, cavity removed."""
    ids = {
        c: n + 1
        for n, c in enumerate(
            itertools.product(range(len(XS)), range(len(YS)), range(len(ZS)))
        )
    }
    nodes = {n: [XS[c[0]], YS[c[1]], ZS[c[2]]] for c, n in ids.items()}
    elements, regions = {}, {}
    cells = itertools.product(
        range(len(XS) - 1), range(len(YS) - 1), range(len(ZS) - 1)
    )
    for i, j, k in cells:
        name = region(i, j, k, window)
        if name is None:
            continue
        for order in itertools.permutations(range(3)):
            corner = [i, j, k]
            path = [ids[tuple(corner)]]
            for axis in order:
                corner[axis] += 1
                path.append(ids[tuple(corner)])
            if solid.tet_volume(*(nodes[n] for n in path)) < 0:
                path[1], path[2] = path[2], path[1]
            elements[len(elements) + 1] = path
            regions.setdefault(name, []).append(len(elements))
    used = {n for path in elements.values() for n in path}
    nodes = {n: xyz for n, xyz in nodes.items() if n in used}
    bounds = [(XS[0], XS[-1]), (YS[0], YS[-1]), (ZS[0], ZS[-1])]

    def label(points):
        outer = any(
            all(abs(p[a] - v) < 1e-12 for p in points)
            for a in range(3)
            for v in bounds[a]
        )
        return "ambient" if outer else "cavity"

    return gate.record(nodes, elements, regions, label)


def thermal(
    values, power, transparent=True, radiation=False, window=True, orthotropic=False
):
    metal = values["metal"]
    coat = material(
        metal["k"],
        metal["emissivity"],
        metal["absorptance"],
        f"W1 library:{CANDIDATE} conservative ends",
    )
    spec = envelope()["templates"]["screen"][2]["plan_request"]["request"]["spec"]
    solar = {
        "irradiance_w_m2": values["solar_w_m2"],
        "direction": spec["solar"]["direction"],
    }
    materials = {
        "housing": coat,
        "lid": dict(coat),
        "pcb": material(
            values["board_k"],
            None,
            None,
            "electronics:pcb-fr4-copper-through-plane.low (cited conservative default)",
        ),
    }
    if orthotropic:
        # The analogue board is the axis-aligned 60x60x2 mm plate at k == 2:
        # in-plane x/y and through-plane z, both cited low ends.
        k_in, k_through = values["board_in_plane_k"], values["board_k"]
        materials["pcb"][enclosure.AXES] = [k_in, k_in, k_through]
    exposed = ["housing", "lid"]
    if window:
        materials["window"] = material(
            values["glazing"]["k"],
            values["glazing"]["emissivity"],
            None,
            "W1 library:pc-uv conservative ends",
        )
        exposed.append("window")
        if transparent:
            solar |= {
                "transparent_solar": enclosure.TRANSPARENT_SOLAR,
                "transparent_regions": ["window"],
            }
    return {
        "synthetic": False,
        "materials": materials,
        "heat_source_region": "pcb",
        "heat_source_power_w": power,
        "ambient_k": values["ambient_k"],
        "film_w_m2_k": values["film"],
        "exposed_regions": exposed,
        "internal_film_w_m2_k": values["internal_film"],
        "solar": solar,
        "radiation": radiation,
        "energy_balance_tolerance": 1e-3,
        "provenance": {
            "environment": "ETSI EN 300 019-1-4 class 4.1 hot point (data/conditions.json)",
            "internal_film": "film:enclosed-cavity-natural.low (data/conditions.json)",
            "heat_source_power": "swept design power (examples/design-enclosure.json)",
        },
    }


def solve(rec, values, power, **options):
    """[PCB peak, internal air, housing peak] (the design_providers criteria)."""
    model_input = thermal(values, power, **options)
    enclosure.validate_thermal(model_input, sorted(rec["regions"]))
    model = enclosure.boundary_model(rec, model_input, solid)
    _, result = enclosure.evaluate(
        rec, model_input, model, gate.reference_solver(rec, model, model_input), solid
    )
    assert result["energy_balance"]["relative_error"] <= 1e-3
    regions = result["regions"]
    peaks = [regions["pcb"]["max_k"], result["internal_air_k"]]
    return peaks + [regions["housing"]["max_k"]], result


def supported_power(powers, lo, hi, limits):
    """Mirror of design_providers::supported_power (superposition, clamped)."""
    p_lo, p_hi = powers
    best = p_hi
    for t_lo, t_hi, limit in zip(lo, hi, limits):
        slope = (t_hi - t_lo) / (p_hi - p_lo)
        if t_lo > limit or slope <= 0:
            return None
        best = min(best, p_lo + (limit - t_lo) / slope)
    return best


def superposed(powers, lo, hi, power):
    t = (power - powers[0]) / (powers[1] - powers[0])
    return [a + t * (b - a) for a, b in zip(lo, hi)]


def sweep(rec, values, **options):
    powers = [declared("pcb_power_low_w"), declared("pcb_power_high_w")]
    assert powers == [1.0, 15.0]
    lo, low = solve(rec, values, powers[0], **options)
    hi, _ = solve(rec, values, powers[1], **options)
    # Components sit between board and air: both stay under the component
    # limit (separate criteria: each is affine, their maximum is not).
    component = values["component_k"]
    housing = values["metal"]["max_k"] - declared("housing_margin_required_k")
    limits = [component, component, housing]
    return powers, lo, hi, limits, low


def test_superposition_is_affine_without_radiation_and_not_with_it():
    values = sourced()
    rec = analogue()
    powers, mid = [1.0, 15.0], 8.0
    for radiation in (False, enclosure.LINEARIZED, True):
        lo, hi, actual = (
            solve(rec, values, p, radiation=radiation)[0] for p in (*powers, mid)
        )
        # Chord minus solve at the midpoint, per criterion.
        gap = [c - a for c, a in zip(superposed(powers, lo, hi, mid), actual)]
        if radiation is not True:
            # Linear model (the linearized radiation bound is a constant
            # film): the single air node is exactly affine; a peak (maximum
            # over nodes) is convex, so the chord bounds it from above, here
            # well inside the 0.5 K affinity tolerance.
            assert abs(gap[1]) < 1e-6, gap
            assert all(-1e-6 <= g <= 0.1 for g in gap), gap
        else:
            # Radiation (T^4) bends the response, so superposition would be
            # inexact; power_sweep refuses radiation (Rust resolver).
            assert abs(gap[1]) > 0.05, gap


def test_aluminium_candidate_max_supported_power_on_the_reference_solver_path(
    tmp_path,
):
    env = envelope()
    assert CANDIDATE in env["space"]["categorical"][0]["choices"]
    values = sourced()
    assert values["component_k"] == pytest.approx(358.15)
    assert values["board_k"] == 0.26
    rec = analogue()
    names = sorted(rec["regions"])
    # Without the explicit transparent_solar bound the window still refuses.
    with pytest.raises(ValueError, match="absorptance"):
        enclosure.validate_thermal(thermal(values, 1.0, transparent=False), names)
    # Ablation of the two lower-bound credits (window cavity deposit and the
    # 1.2 W/(m2*K) cavity film stay): no credit, linearized exterior radiation,
    # orthotropic board (8.1 in-plane / 0.26 through-plane, cited low ends),
    # both. Each supported power is verified like the refine tier by a third
    # solve reaching the binding limit (the PCB peak) within 0.5 K.
    expected_power = {
        # window: the stacked bounds put the internal air at 373.6 K at 1 W.
        (True, False, False): None,
        (True, True, False): 10.11,
        (True, False, True): None,
        (True, True, True): 10.42,
        (False, False, False): 2.04,
        (False, True, False): 10.77,
        (False, False, True): 2.08,
        (False, True, True): 11.10,
    }
    meshes = {True: rec, False: analogue(window=False)}
    for (window, radiating, orthotropic), want in expected_power.items():
        options = {
            "window": window,
            "radiation": enclosure.LINEARIZED if radiating else False,
            "orthotropic": orthotropic,
        }
        mesh = meshes[window]
        powers, lo, hi, limits, low = sweep(mesh, values, **options)
        # The linearized bound's precondition holds on every exterior face.
        assert low["exterior_min_k"] >= values["ambient_k"]
        if window:
            assert low["energy_balance"]["solar_cavity_w"] > 0
        power = supported_power(powers, lo, hi, limits)
        if want is None:
            assert low["internal_air_k"] > values["component_k"], options
            assert power is None, options
            continue
        assert power == pytest.approx(want, abs=0.01), (options, power)
        check, _ = solve(mesh, values, power, **options)
        expected = superposed(powers, lo, hi, power)
        assert all(abs(c - e) <= AFFINITY_TOLERANCE_K for c, e in zip(check, expected))
        binding = min(range(3), key=lambda i: limits[i] - check[i])
        assert abs(check[binding] - limits[binding]) <= AFFINITY_TOLERANCE_K
        assert binding == 0, options  # the component limit on the PCB peak
    # PA12 stays infeasible: -40 degC service limit vs the -33 degC cold point.
    assert values["cold_k"] - values["pa12_min_k"] == pytest.approx(7.0)
    # Manufacturability and cost through the Rust screen on the analogue housing.
    volume = sum(
        solid.tet_volume(*(rec["nodes"][n] for n in rec["elements"][e]))
        for e in rec["regions"]["housing"]
    )
    request = {
        "geometry": {
            "volume_m3": volume,
            "bbox_m": [XS[-1], YS[-1], ZS[-2]],
            "min_wall_m": 0.004,
            "surface_area_m2": None,
        },
        "material_id": CANDIDATE.split("__")[0],
        "process_id": CANDIDATE.split("__")[1],
        "quantity": int(env["metric_sources"][0]["source"]["input"]["value"]),
    }
    path = tmp_path / "cost.json"
    path.write_text(json.dumps(request))
    report = json.loads(
        subprocess.check_output(
            [os.environ["HARBOR_CAD_TEST_BINARY"], "cost", "estimate", str(path)]
        )
    )
    assert report["manufacturability"] == []
    assert report["estimate"]["high"] is not None
