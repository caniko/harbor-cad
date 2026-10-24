"""Nothing structural blocks a feasible enclosure design any more.

One compatible candidate of examples/design-enclosure.json is solved with the
pure-Python reference solver (no CalculiX) on a structured analogue of the
enclosure fixture (housing walls and bosses, lid, pc-uv window, PCB). Every
environment, material, finish and cavity-film value is the cited conservative
end the Rust resolver uses; the window takes the explicit conservative
transparent_solar bound. ONLY the operator inputs (PCB power, board
conductivity, component limit) are SYNTHETIC stand-ins. The analogue omits the
fins (less exterior area) and the gasket; it is a demonstration, not design
evidence.
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
SYNTHETIC_OPERATOR = {
    "pcb_power_w": 3.0,
    "pcb_conductivity_w_m_k": 20.0,
    "component_max_temperature_k": 398.15,
}


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


def end(value, high):
    """Conservative end of a cited range in SI (only the units used here)."""
    assert value["availability"] == "known" and not value["synthetic"]
    side = value["value"]["high" if high else "low"]
    offset = {"degC": 273.15}.get(side["unit"], 0.0)
    assert side["unit"] in {"degC", "1", "W/(m*K)", "W/(m2*K)", "W/m2"}
    return side["value"] + offset


def by_id(items, key):
    return next(item for item in items if item["id"] == key)


def sourced():
    materials, finishes = data("materials.json")["materials"], data("finishes.json")
    conditions = data("conditions.json")
    climate = by_id(conditions["classes"], "etsi-4-1")
    still = next(f for f in conditions["films"] if f["wind_speed"] is None)
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


def material(k, emissivity, absorptance, synthetic, provenance):
    return {
        "source": "declared" if synthetic else "library",
        "conductivity_w_m_k": k,
        "emissivity": emissivity,
        "solar_absorptance": absorptance,
        "provenance": provenance,
        "synthetic": synthetic,
    }


XS = [0, 0.004, 0.07, 0.076, 0.124, 0.13, 0.196, 0.2]
YS = [0, 0.004, 0.045, 0.051, 0.099, 0.105, 0.146, 0.15]
ZS = [0, 0.004, 0.012, 0.014, 0.076, 0.08]


def region(i, j, k):
    """Cell region; None is the internal air cavity (no elements)."""
    inner = 0 < i < len(XS) - 2 and 0 < j < len(YS) - 2
    if k == len(ZS) - 2:
        return "window" if i in (3,) and j in (3,) else "lid"
    if not inner or k == 0:
        return "housing"
    if k == 1 and i in (2, 4) and j in (2, 4):
        return "housing"  # 6 mm bosses under the board corners
    if k == 2 and 2 <= i <= 4 and 2 <= j <= 4:
        return "pcb"
    return None


def analogue():
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
        name = region(i, j, k)
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


def thermal(values, transparent=True):
    metal = values["metal"]
    coat = material(
        metal["k"],
        metal["emissivity"],
        metal["absorptance"],
        False,
        f"W1 library:{CANDIDATE} conservative ends",
    )
    direction = json.loads((ROOT / "examples/design-enclosure.json").read_text())[
        "templates"
    ]["screen"][2]["plan_request"]["request"]["spec"]["solar"]["direction"]
    solar = {"irradiance_w_m2": values["solar_w_m2"], "direction": direction}
    if transparent:
        solar |= {
            "transparent_solar": enclosure.TRANSPARENT_SOLAR,
            "transparent_regions": ["window"],
        }
    return {
        "synthetic": True,
        "materials": {
            "housing": coat,
            "lid": dict(coat),
            "window": material(
                values["glazing"]["k"],
                values["glazing"]["emissivity"],
                None,
                False,
                "W1 library:pc-uv conservative ends",
            ),
            "pcb": material(
                SYNTHETIC_OPERATOR["pcb_conductivity_w_m_k"],
                None,
                None,
                True,
                "SYNTHETIC operator stand-in for the board conductivity",
            ),
        },
        "heat_source_region": "pcb",
        "heat_source_power_w": SYNTHETIC_OPERATOR["pcb_power_w"],
        "ambient_k": values["ambient_k"],
        "film_w_m2_k": values["film"],
        "exposed_regions": ["housing", "lid", "window"],
        "internal_film_w_m2_k": values["internal_film"],
        "solar": solar,
        "radiation": True,
        "energy_balance_tolerance": 1e-3,
        "provenance": {
            "environment": "ETSI EN 300 019-1-4 class 4.1 hot point (data/conditions.json)",
            "internal_film": "film:enclosed-cavity-natural.low (data/conditions.json)",
            "heat_source_power": "SYNTHETIC operator stand-in",
        },
    }


def test_compatible_candidate_is_feasible_on_the_reference_solver_path(tmp_path):
    envelope = json.loads((ROOT / "examples/design-enclosure.json").read_text())
    assert CANDIDATE in envelope["space"]["categorical"][0]["choices"]
    values = sourced()
    rec = analogue()
    names = sorted(rec["regions"])
    # Without the explicit transparent_solar bound the window still refuses.
    with pytest.raises(ValueError, match="absorptance"):
        enclosure.validate_thermal(thermal(values, transparent=False), names)
    model_input = thermal(values)
    enclosure.validate_thermal(model_input, names)
    model = enclosure.boundary_model(rec, model_input, solid)
    _, result = enclosure.evaluate(
        rec, model_input, model, gate.reference_solver(rec, model, model_input), solid
    )
    energy = result["energy_balance"]
    assert energy["relative_error"] <= 1e-3
    assert energy["solar_cavity_w"] > 0 and energy["solar_absorbed_w"] > 0
    assert result["internal_air_k"] > values["ambient_k"]
    housing = result["regions"]["housing"]["max_k"]
    board = result["regions"]["pcb"]["max_k"]
    # The design constraints of the envelope, with the providers' definitions.
    temperature_margin = min(
        values["metal"]["max_k"] - housing, values["cold_k"] - values["metal"]["min_k"]
    )
    component_margin = SYNTHETIC_OPERATOR["component_max_temperature_k"] - board
    assert temperature_margin >= 10.0, (housing, temperature_margin)
    assert component_margin >= 0.0, (board, component_margin)
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
        "quantity": int(envelope["metric_sources"][0]["source"]["input"]["value"]),
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
