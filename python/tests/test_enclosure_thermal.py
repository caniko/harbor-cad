"""General-solid meshing and enclosure thermal recipe checks without native tools."""

import copy
import importlib.util
from pathlib import Path
from types import SimpleNamespace

import pytest

ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    source = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(source)
    source.loader.exec_module(module)
    return module


solid = load("solid_mesh", "adapters/solid_mesh.py")
enclosure = load("enclosure_thermal", "adapters/enclosure_thermal.py")
gate = load("verify_enclosure_thermal_cpu", "scripts/verify_enclosure_thermal_cpu.py")


def descriptor():
    region = {
        "brep_sha256": "a" * 64,
        "brep_bytes": 10,
        "bounds_m": [0.0, 0.02, 0.0, 0.01, 0.0, 0.01],
        "volume_m3": 1.5e-6,
        "source_transform": [1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0],
    }
    return {
        "schema_version": 1,
        "synthetic": True,
        "backend": "cpu",
        "formulation": "imported_general_solids",
        "geometry_provenance": "controlled synthetic solids",
        "source_unit": "mm",
        "scale_to_m": 0.001,
        "placement_translation_unit": "mm",
        "regions": [{"name": "housing", **region}, {"name": "pcb", **region}],
        "mesh": {
            "characteristic_length_max_m": 0.004,
            "element_order": 1,
            "max_elements": 1000,
            "volume_relative_tolerance": 0.02,
            "min_quality": 0.05,
            "geometry_tolerance_m": 1e-5,
        },
    }


@pytest.mark.parametrize(
    "change",
    [
        lambda d: d["regions"].reverse(),
        lambda d: d["regions"][0].update(name="../housing"),
        lambda d: d["regions"][0].update(volume_m3=3e-6),
        lambda d: d["mesh"].update(element_order=3),
        lambda d: d["mesh"].update(max_elements=3_000_000),
        lambda d: d["mesh"].update(volume_relative_tolerance=0.5),
        lambda d: d.update(formulation="imported_axis_aligned_box"),
        lambda d: d.update(healing=True),
        lambda d: d["regions"][0]["source_transform"].__setitem__(0, 2.0),
    ],
)
def test_solid_descriptor_rejects_unbound_or_unbounded_inputs(change):
    solid.validate(descriptor())
    changed = copy.deepcopy(descriptor())
    change(changed)
    with pytest.raises(ValueError):
        solid.validate(changed)


def fake_gmsh(dimension=3, volume=1.5e-6):
    calls = []
    occ = SimpleNamespace(
        importShapes=lambda path, highestDimOnly, format: (
            calls.append((path, highestDimOnly, format)) or [(dimension, len(calls))]
        ),
        synchronize=lambda: None,
        getMass=lambda dim, tag: volume,
    )
    options = {}
    return (
        SimpleNamespace(
            option=SimpleNamespace(setNumber=options.__setitem__),
            model=SimpleNamespace(
                occ=occ, getBoundingBox=lambda dim, tag: (0, 0, 0, 0.02, 0.01, 0.01)
            ),
        ),
        calls,
        options,
    )


def test_import_scales_unhealed_and_refuses_open_or_changed_solids():
    gmsh, calls, options = fake_gmsh()
    assert solid.import_regions(gmsh, descriptor(), "/inputs") == [1, 2]
    assert [c[0] for c in calls] == ["/inputs/housing.brep", "/inputs/pcb.brep"]
    assert options["Geometry.OCCScaling"] == 0.001
    assert all(options[f"Geometry.OCC{k}"] == 0 for k in ("SewFaces", "MakeSolids"))
    with pytest.raises(ValueError, match="closed solid"):
        solid.import_regions(fake_gmsh(dimension=2)[0], descriptor(), "/inputs")
    with pytest.raises(ValueError, match="differs"):
        solid.import_regions(fake_gmsh(volume=1.4e-6)[0], descriptor(), "/inputs")


def hollow_cube():
    """3x3x3 unit cells with the centre removed: a closed internal cavity."""
    nodes, elements, _ = gate.box_mesh(
        [0, 0, 0], [0.03] * 3, (3, 3, 3), lambda x: "housing"
    )
    centre = [0.015] * 3
    keep = {
        e: n
        for e, n in elements.items()
        if max(abs(sum(nodes[i][k] for i in n) / 4 - centre[k]) for k in range(3))
        > 0.005
    }
    return nodes, keep, {"housing": sorted(keep)}


def surfaces(nodes, elements):
    keys = {}
    for owner in solid.faces(elements).values():
        if len(owner) == 1:
            eid, face = owner[0]
            corners = [elements[eid][i] for i in solid.FACES[face - 1]]
            key = tuple(sorted(corners))
            points = [nodes[n] for n in corners]
            inner = all(
                0.01 - 1e-12 <= p[k] <= 0.02 + 1e-12 for p in points for k in range(3)
            )
            keys[key] = 1 if inner else 2
    return keys


def spec_for(volume):
    spec = descriptor()
    spec["regions"] = [
        {**spec["regions"][0], "bounds_m": [0, 0.03] * 3, "volume_m3": volume}
    ]
    spec["mesh"]["max_elements"] = 10000
    return spec


def test_exterior_classification_separates_cavity_from_ambient_faces():
    nodes, elements, regions = hollow_cube()
    record, facts = solid.assemble(
        spec_for(26e-6), nodes, elements, regions, surfaces(nodes, elements), [0.5]
    )
    assert facts["boundary_area_m2"]["ambient"]["housing"] == pytest.approx(6 * 9e-4)
    assert facts["boundary_area_m2"]["cavity"]["housing"] == pytest.approx(6 * 1e-4)
    assert {label for *_, label in record["boundary_faces"]} == {"ambient", "cavity"}
    assert facts["regions"]["housing"]["relative_volume_error"] < 1e-12
    with pytest.raises(ValueError, match="volume"):
        solid.assemble(
            spec_for(27e-6), nodes, elements, regions, surfaces(nodes, elements), [0.5]
        )
    strict = spec_for(26e-6)
    strict["mesh"]["min_quality"] = 0.9
    with pytest.raises(ValueError, match="quality"):
        solid.assemble(
            strict, nodes, elements, regions, surfaces(nodes, elements), [0.5]
        )
    flipped = dict(elements)
    first = next(iter(flipped))
    flipped[first] = [flipped[first][1], flipped[first][0], *flipped[first][2:]]
    with pytest.raises(ValueError, match="inverted"):
        solid.assemble(
            spec_for(26e-6), nodes, flipped, regions, surfaces(nodes, elements), [0.5]
        )


def thermal():
    return gate.thermal(
        {"housing": gate.material(150.0, 0.8, 0.5), "pcb": gate.material(5.0)},
        "pcb",
        2.0,
        ["housing"],
    )


def test_thermal_model_refuses_undeclared_optics_and_weak_gates():
    enclosure.validate_thermal(thermal(), ["housing", "pcb"])
    for change, message in [
        (lambda t: t["materials"]["housing"].update(solar_absorptance=None), None),
        (lambda t: t["materials"]["housing"].update(emissivity=None), "emissivity"),
        (lambda t: t.update(energy_balance_tolerance=1e-2), None),
        (lambda t: t.update(exposed_regions=["pcb", "housing"]), None),
        (lambda t: t.update(heat_source_region="lid"), None),
        (lambda t: t["materials"].pop("pcb"), None),
    ]:
        changed = copy.deepcopy(thermal())
        changed["radiation"] = True
        changed["solar"] = {"irradiance_w_m2": 800.0, "direction": [0.0, 0.0, 1.0]}
        enclosure.validate_thermal(copy.deepcopy(changed), ["housing", "pcb"])
        change(changed)
        with pytest.raises(ValueError, match=message):
            enclosure.validate_thermal(changed, ["housing", "pcb"])


def series_case(radiation, solar):
    rec, data, _, _ = gate.series(2)
    data["materials"]["housing"]["emissivity"] = 0.8
    data["materials"]["housing"]["solar_absorptance"] = 0.5
    data["radiation"] = radiation
    if solar:
        data["solar"] = {"irradiance_w_m2": 600.0, "direction": [1.0, 0.0, 0.0]}
    return rec, data


def test_deck_uses_derived_faces_regions_and_declared_loads():
    rec, data = series_case(radiation=True, solar=True)
    model = enclosure.boundary_model(rec, data, solid)
    text = enclosure.deck(rec, data, model, 300.0)
    assert "*ELEMENT,TYPE=C3D4,ELSET=E_PCB" in text
    density = enclosure.ccx_number(model["power_density_w_m3"])
    assert f"E_PCB,BF,{density}" in text
    assert enclosure.ccx_number(-1.734723475976807e-18) == "-1.7347234759768e-18"
    assert len(enclosure.ccx_number(-7.123456789012345e-306)) <= 20
    assert "*RADIATE" in text and "STEFAN BOLTZMANN" in text
    assert "SOLVER=SPOOLES,DIRECT" not in text
    film = [
        line
        for line in text.splitlines()
        if ",F" in line and line.count(",") == 3 and not line.startswith("*")
    ]
    assert len(film) == len(model["film"]) > 0
    assert len(model["sun"]) == len(model["film"])
    plain = enclosure.deck(*series_case(radiation=False, solar=False), model, 300.0)
    assert "*RADIATE" not in plain and "SOLVER=SPOOLES,DIRECT" in plain


def test_reference_solver_conserves_energy_with_radiation_and_solar():
    rec, data = series_case(radiation=True, solar=True)
    model = enclosure.boundary_model(rec, data, solid)
    _, result = enclosure.evaluate(
        rec, data, model, gate.reference_solver(rec, model, data), solid
    )
    energy = result["energy_balance"]
    assert energy["radiation_w"] > 0 and energy["solar_absorbed_w"] > 0
    assert energy["relative_error"] < 1e-8
    assert result["regions"]["pcb"]["max_k"] == result["global_max"]["temperature_k"]
    assert result["global_max"]["regions"] == ["pcb"]


def test_energy_gate_refuses_an_unconverged_field():
    rec, data = series_case(radiation=False, solar=False)
    model = enclosure.boundary_model(rec, data, solid)
    exact = gate.reference_solver(rec, model, data)
    with pytest.raises(ValueError, match="energy balance"):
        enclosure.evaluate(
            rec,
            data,
            model,
            lambda air: {n: t + 1.0 for n, t in exact(air).items()},
            solid,
        )


def test_ccx_runner_refuses_failed_solver_logs(tmp_path, monkeypatch):
    rec, data = series_case(radiation=False, solar=False)
    model = enclosure.boundary_model(rec, data, solid)
    monkeypatch.chdir(tmp_path)
    Path("reference.dat").write_text("stale")
    fem = SimpleNamespace(
        read_regular=lambda path, maximum: Path(path).read_bytes(),
        read_dat=lambda text: {},
    )

    def fake(command, **kwargs):
        assert not Path("reference.dat").exists()
        assert Path("reference.inp").read_text().startswith("*HEADING")
        kwargs["stdout"].write(b"Version 2.23\n*ERROR in film\nJob finished\n")
        return SimpleNamespace(returncode=0)

    monkeypatch.setattr(enclosure.subprocess, "run", fake)
    runner = enclosure.ccx_runner(rec, data, model, fem, "ccx", "2.23", "calculix.log")
    with pytest.raises(ValueError, match="did not succeed"):
        runner(300.0)


def test_analytical_cpu_references_pass_at_two_refinements(capsys):
    report = gate.main([])
    assert set(report["cases"]) == set(gate.CASES)
    for case in report["cases"].values():
        assert case["passed"] and len(case["levels"]) == 2
        assert case["levels"][-1]["normalized_max_abs_error"] <= case["gate"]
    air = report["cases"]["lumped_internal_air_chain"]["levels"][-1]
    assert air["internal_air_k"] > 300.0
