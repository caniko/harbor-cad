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


def test_transparent_solar_is_explicit_and_needs_a_cavity_air_node():
    rec, data = series_case(radiation=False, solar=True)
    data["materials"]["housing"]["solar_absorptance"] = None
    with pytest.raises(ValueError, match="absorptance"):
        enclosure.validate_thermal(data, ["housing", "pcb"])
    for regions in (["pcb"], [], ["housing", "housing"]):
        changed = copy.deepcopy(data)
        changed["solar"] |= {"transparent_solar": "deposit_incident_in_cavity"}
        changed["solar"]["transparent_regions"] = regions
        with pytest.raises(ValueError, match="transparent_solar"):
            enclosure.validate_thermal(changed, ["housing", "pcb"])
    data["solar"] |= {
        "transparent_solar": "deposit_incident_in_cavity",
        "transparent_regions": ["housing"],
    }
    enclosure.validate_thermal(data, ["housing", "pcb"])
    # The series case has no cavity: the deposit has nowhere to go.
    with pytest.raises(ValueError, match="cavity"):
        enclosure.boundary_model(rec, data, solid)
    statement = enclosure.applicability(data)["transparent_solar"]
    assert statement.startswith("conservative_bound")
    # Rust gates the receipt on the same label text.
    assert f'"{statement}"' in (ROOT / "src/enclosure_thermal.rs").read_text()


def test_linearized_radiation_coefficient_is_a_proven_lower_bound():
    # T^4 - Ta^4 = (T - Ta)(T^3 + T^2 Ta + T Ta^2 + Ta^3) >= 4 Ta^3 (T - Ta)
    # for T >= Ta (each factor term >= Ta^3), and for T < Ta by convexity of
    # T^4 (its tangent at Ta lies below it): the credited flux never exceeds
    # the grey exchange with surroundings at ambient.
    for emissivity in (0.03, 0.5, 0.95):
        for ambient in (230.0, 273.15, 313.15, 330.0):
            h_r = 4 * emissivity * enclosure.SIGMA * ambient**3
            for rise in (-80.0, -1.0, -1e-3, 1e-3, 0.5, 5.0, 50.0, 200.0):
                t = ambient + rise
                exact = emissivity * enclosure.SIGMA * (t**4 - ambient**4)
                assert h_r * rise <= exact
                if rise > 0:
                    assert h_r <= exact / rise  # coefficient form
    data = thermal()
    data["radiation"] = enclosure.LINEARIZED
    data["materials"]["pcb"]["emissivity"] = None
    enclosure.validate_thermal(data, ["housing", "pcb"])
    assert enclosure.radiation_coefficient(data, "housing") == pytest.approx(
        4 * 0.8 * enclosure.SIGMA * 300.0**3
    )
    assert enclosure.radiation_coefficient(data, "pcb") == 0.0  # no credit
    for mode in ("linearized", 1, None):
        data["radiation"] = mode
        with pytest.raises(ValueError, match="radiation"):
            enclosure.validate_thermal(data, ["housing", "pcb"])


def test_linearized_radiation_deck_receipt_and_precondition():
    rec, data = series_case(radiation=enclosure.LINEARIZED, solar=True)
    model = enclosure.boundary_model(rec, data, solid)
    text = enclosure.deck(rec, data, model, 300.0)
    h_r = enclosure.radiation_coefficient(data, "housing")
    assert f",300.0,{enclosure.ccx_number(50.0 + h_r)}" in text
    assert "*RADIATE" not in text and "STEFAN BOLTZMANN" not in text
    assert "SOLVER=SPOOLES,DIRECT" in text and "FREQUENCY=999999" in text
    exact = gate.reference_solver(rec, model, data)
    _, result = enclosure.evaluate(rec, data, model, exact, solid)
    energy = result["energy_balance"]
    assert energy["radiation_w"] > 0 and energy["relative_error"] < 1e-8
    assert result["exterior_min_k"] >= data["ambient_k"]
    statement = enclosure.applicability(data)["radiation"]
    assert statement == enclosure.LINEARIZED_STATEMENT
    assert f'"{statement}"' in (ROOT / "src/enclosure_thermal.rs").read_text()

    # An exterior face below ambient voids the bound: refused before the gate.
    def cold(air):
        return {n: t - 60.0 for n, t in exact(air).items()}

    with pytest.raises(ValueError, match="exterior surfaces >= ambient"):
        enclosure.evaluate(rec, data, model, cold, solid)


def test_orthotropic_deck_keywords_and_validation():
    rec, data = series_case(radiation=False, solar=False)
    data["materials"]["pcb"][enclosure.AXES] = [8.1, 8.1, 0.26]
    enclosure.validate_thermal(data, ["housing", "pcb"])
    text = enclosure.deck(rec, data, enclosure.boundary_model(rec, data, solid), 300.0)
    assert (
        "*MATERIAL,NAME=M_PCB\n*CONDUCTIVITY,TYPE=ORTHO\n8.1,8.1,0.26\n"
        "*ORIENTATION,NAME=OR_PCB\n1.,0.,0.,0.,1.,0.\n"
        "*SOLID SECTION,ELSET=E_PCB,MATERIAL=M_PCB,ORIENTATION=OR_PCB\n"
    ) in text
    assert "*MATERIAL,NAME=M_HOUSING\n*CONDUCTIVITY\n5.0\n" in text
    for axes in ([8.1, 8.1], [8.1, 8.1, 0.0], "8.1"):
        data["materials"]["pcb"][enclosure.AXES] = axes
        with pytest.raises(ValueError, match="conductivit"):
            enclosure.validate_thermal(data, ["housing", "pcb"])


@pytest.mark.parametrize("axis", [0, 1, 2])
def test_orthotropic_tetra_stiffness_matches_1d_conduction_along_each_axis(axis):
    # Slab along `axis` with uniform heat and film on both end faces; the other
    # axes (and the scalar, so ignoring the tensor fails) carry a 1000x
    # different conductivity that a 1D field never sees.
    length, width, n = 0.04, 0.01, 8
    size, cells = [width] * 3, [2] * 3
    size[axis], cells[axis] = length, n
    nodes, elements, regions = gate.box_mesh([0, 0, 0], size, cells, lambda x: "slab")

    def ends(points):
        on_end = gate.on_plane(axis, 0)(points) or gate.on_plane(axis, length)(points)
        return "ambient" if on_end else None

    rec = gate.record(nodes, elements, regions, ends)
    q, k, h = 1e5, 2.0, 50.0
    material = gate.material(1000.0 * k)
    material[enclosure.AXES] = [1000.0 * k] * 3
    material[enclosure.AXES][axis] = k
    data = gate.thermal({"slab": material}, "slab", q * length * width**2, ["slab"], h)
    model = enclosure.boundary_model(rec, data, solid)
    temperatures, _ = enclosure.evaluate(
        rec, data, model, gate.reference_solver(rec, model, data), solid
    )
    rise = q * length / (2 * h) + q * length**2 / (8 * k)
    for node, xyz in nodes.items():
        x = xyz[axis]
        exact = 300 + q * length / (2 * h) + q * x * (length - x) / (2 * k)
        assert abs(temperatures[node] - exact) <= 1e-2 * rise


def floating_board(emissivity=0.9, power=0.5, solar=None, housing_k=150.0):
    """Hollow 50 mm housing (10 mm walls) with a free 20x20x4 mm board inside."""
    nodes, elements, _ = gate.box_mesh([0, 0, 0], [0.05] * 3, (5, 5, 5), lambda x: "h")
    shell = {
        e: n
        for e, n in elements.items()
        if max(abs(sum(nodes[i][k] for i in n) / 4 - 0.025) for k in range(3)) > 0.015
    }
    board_nodes, board, _ = gate.box_mesh(
        [0.015, 0.015, 0.02],
        [0.02, 0.02, 0.004],
        (2, 2, 1),
        lambda x: "pcb",
        1000,
        10000,
    )
    regions = {"housing": sorted(shell), "pcb": sorted(board)}
    nodes = {n: nodes[n] for n in {i for e in shell.values() for i in e}}
    outer = [gate.on_plane(a, v) for a in range(3) for v in (0, 0.05)]
    rec = gate.record(
        nodes | board_nodes,
        shell | board,
        regions,
        lambda p: "ambient" if any(on(p) for on in outer) else "cavity",
    )
    data = gate.thermal(
        {"housing": gate.material(housing_k, 0.9, 0.9), "pcb": gate.material(5.0)},
        "pcb",
        power,
        ["housing"],
        film=10.0,
        internal=2.0,
    )
    data["solar"] = solar
    data[enclosure.CAVITY] = {
        "mode": enclosure.LINEARIZED,
        "emissivity": {"housing": 0.9, "pcb": emissivity},
    }
    return rec, data


def test_cavity_radiation_pair_bound_holds_only_under_the_postcondition():
    # T_b^4 - T_w^4 >= 4 T_w^3 (T_b - T_w) >= 4 Ta^3 (T_b - T_w) for
    # T_b >= T_w >= Ta: tangent of the convex T^4 at T_w, then T_w^3 >= Ta^3.
    ambient = 313.15
    for wall in (ambient, ambient + 0.5, ambient + 20.0):
        for rise in (0.0, 1e-3, 1.0, 30.0, 150.0):
            board = wall + rise
            assert board**4 - wall**4 >= 4 * ambient**3 * (board - wall)
    # Without the postcondition the linear credit is optimistic: a wall
    # hotter than the board heats it more than credited, and a pair below
    # ambient exchanges less than credited.
    board, wall = ambient + 1.0, ambient + 3.0
    assert board**4 - wall**4 < 4 * ambient**3 * (board - wall)
    board, wall = ambient - 5.0, ambient - 10.0
    assert board**4 - wall**4 < 4 * ambient**3 * (board - wall)
    # Equal board and wall conductances (2h*A_b each) put the radiation node
    # at the mean of the area-weighted means; it transfers h*A_b*(Tb - Tw).
    h, a_b, a_w, t_b, t_w = 1.7, 0.002, 0.03, 340.0, 320.0
    node = (2 * h * a_b * t_b + 2 * h * a_b / a_w * a_w * t_w) / (4 * h * a_b)
    assert 2 * h * a_b * (t_b - node) == pytest.approx(h * a_b * (t_b - t_w))


def test_cavity_radiation_deck_solve_receipt_and_superposition():
    rec, data = floating_board()
    enclosure.validate_thermal(data, ["housing", "pcb"])
    model = enclosure.boundary_model(rec, data, solid)
    cavity = model["cavity_radiation"]
    ratio = cavity["board_area_m2"] / cavity["wall_area_m2"]
    eff = 1 / (1 / 0.9 + ratio * (1 / 0.9 - 1))
    assert cavity["effective_emissivity"] == pytest.approx(eff)
    h = 4 * eff * enclosure.SIGMA * 300.0**3
    assert cavity["h_w_m2_k"] == pytest.approx(h)
    assert cavity["board_area_m2"] == pytest.approx(2 * 0.02**2 + 4 * 0.02 * 0.004)
    assert cavity["wall_area_m2"] == pytest.approx(6 * 0.03**2)
    # One combined *FILM per credited face: (h_a + h_r)(T - sink).
    e, f, _ = cavity["board"][0]
    text = enclosure.deck(rec, data, model, 310.0, 320.0)
    h_total = 2.0 + cavity["films"][e, f]
    sink = (2.0 * 310.0 + cavity["films"][e, f] * 320.0) / h_total
    line = f"{e},F{f},{enclosure.ccx_number(sink)},{enclosure.ccx_number(h_total)}"
    assert line in text.splitlines()
    assert cavity["films"][e, f] == pytest.approx(2 * h)
    exact = gate.reference_solver(rec, model, data)
    _, result = enclosure.evaluate(rec, data, model, exact, solid)
    block = result["cavity_radiation"]
    energy = result["energy_balance"]
    assert energy["relative_error"] < 1e-6
    assert energy["radiation_node_relative_error"] <= 1e-4
    assert len(result["air_iterations"]) <= 6
    assert block["board_surface_min_k"] >= block["wall_max_k"] >= block["wall_min_k"]
    assert block["wall_min_k"] >= data["ambient_k"]
    assert block["h_w_m2_k"] == pytest.approx(h)
    statement = enclosure.applicability(data)["cavity_radiation"]
    assert statement == enclosure.CAVITY_STATEMENT
    assert f'"{statement}"' in (ROOT / "src/enclosure_thermal.rs").read_text()
    # The credit only cools the board.
    plain = copy.deepcopy(data)
    del plain[enclosure.CAVITY]
    plain_model = enclosure.boundary_model(rec, plain, solid)
    _, uncredited = enclosure.evaluate(
        rec, plain, plain_model, gate.reference_solver(rec, plain_model, plain), solid
    )
    assert result["regions"]["pcb"]["max_k"] < uncredited["regions"]["pcb"]["max_k"]
    # Linear model: the air and radiation nodes are affine in power.
    nodes = []
    for power in (0.5, 1.0, 1.5):
        rec, data = floating_board(power=power)
        model = enclosure.boundary_model(rec, data, solid)
        _, swept = enclosure.evaluate(
            rec, data, model, gate.reference_solver(rec, model, data), solid
        )
        cavity = swept["cavity_radiation"]
        nodes.append([swept["internal_air_k"], cavity["radiation_node_k"]])
    for low, mid, high in zip(*nodes, strict=True):
        assert abs(mid - (low + high) / 2) < 1e-3


def test_cavity_radiation_violation_is_refused_and_missing_emissivity_gets_no_credit():
    # A sun-heated low-conductivity lid and a 10 mW board: the hottest wall
    # exceeds the coldest board surface, so the credit is invalid and nothing
    # is claimed.
    sun = {"irradiance_w_m2": 1000.0, "direction": [0.0, 0.0, 1.0]}
    rec, data = floating_board(power=0.01, solar=sun, housing_k=0.2)
    model = enclosure.boundary_model(rec, data, solid)
    with pytest.raises(ValueError, match="cavity radiation lower bound"):
        enclosure.evaluate(
            rec, data, model, gate.reference_solver(rec, model, data), solid
        )
    # No sourced board emissivity: zero credit, no radiation node, no check.
    rec, data = floating_board(emissivity=None, power=0.01, solar=sun, housing_k=0.2)
    model = enclosure.boundary_model(rec, data, solid)
    assert model["cavity_radiation"]["h_w_m2_k"] == 0.0
    _, result = enclosure.evaluate(
        rec, data, model, gate.reference_solver(rec, model, data), solid
    )
    assert result["cavity_radiation"]["radiation_node_k"] is None
    assert "radiation_node_k" not in result["air_iterations"][-1]
    # A board with an exterior face is not enclosed (view factor < 1), and a
    # board without cavity faces has nothing to radiate to.
    rec, data = floating_board()
    pcb = set(rec["regions"]["pcb"])
    face = next(f for f in rec["boundary_faces"] if f[0] in pcb)
    face[2] = "ambient"
    with pytest.raises(ValueError, match="enclosed"):
        enclosure.boundary_model(rec, data, solid)
    rec, data = series_case(radiation=False, solar=False)
    data[enclosure.CAVITY] = {
        "mode": enclosure.LINEARIZED,
        "emissivity": {"housing": 0.8, "pcb": 0.9},
    }
    with pytest.raises(ValueError, match="board cavity faces"):
        enclosure.boundary_model(rec, data, solid)
    for cavity in (
        {"mode": "linearized", "emissivity": {"housing": 0.8, "pcb": 0.9}},
        {"mode": enclosure.LINEARIZED, "emissivity": {"pcb": 0.9}},
        {"mode": enclosure.LINEARIZED, "emissivity": {"housing": 0.0, "pcb": 0.9}},
    ):
        data[enclosure.CAVITY] = cavity
        with pytest.raises(ValueError, match="emissivity"):
            enclosure.validate_thermal(data, ["housing", "pcb"])
