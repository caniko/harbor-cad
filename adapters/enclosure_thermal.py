"""Steady enclosure conduction on imported general solids: Gmsh mesh + CalculiX.

Model, inputs, assumptions and limits: docs/enclosure-thermal.md. Every
physical value arrives resolved and provenance-bound in the descriptor; this
adapter never defaults one. It re-meshes the approved solids, requires the
approved mesh digest, solves with CalculiX and gates the energy balance.
"""

import hashlib
import importlib.util
import math
import subprocess
import sys
from pathlib import Path

FORMULATION = "enclosure_steady_conduction"
SIGMA = 5.670374419e-8
CALCULIX_SOURCE_SHA256 = (
    "9c88385c10fb04f5dc6c4e98027a51bebdd8aee3920e05190d6c1dd08357d6e7"
)
MAX_AIR_RUNS = 8
APPLICABILITY = {
    "regime": "steady state",
    "heat_transfer": "solid conduction with declared isotropic constant conductivities",
    "internal_air": "one well-mixed internal air node coupled to every cavity face by the declared effective surface coefficient (modelling assumption; no internal convection or CFD)",
    "exterior": "uniform declared film coefficient and ambient temperature on ambient faces of exposed regions; other ambient faces adiabatic",
    "solar": "absorbed flux alpha*G*max(0, n.s) on ambient faces of exposed regions; no self-shadowing, no transmission",
    "radiation": "grey emission to surroundings at ambient temperature; no view factors or sky temperature",
    "interfaces": "conformal perfect contact between touching regions; no contact resistance",
    "excluded": "transients, condensation, moisture, phase change, mounting conduction",
}
TRANSPARENT_SOLAR = "deposit_incident_in_cavity"
# Same text as enclosure_thermal::TRANSPARENT_SOLAR_STATEMENT; Rust gates it.
TRANSPARENT_SOLAR_STATEMENT = (
    "conservative_bound: the full solar irradiance incident on sun-facing exterior"
    " faces of transparent glazing is deposited into the internal air node (no"
    " reflection, no absorption in the pane, no re-emission credit); an upper bound"
    " on interior solar gain, not on the pane temperature"
)
THERMAL_KEYS = {
    "synthetic",
    "materials",
    "heat_source_region",
    "heat_source_power_w",
    "ambient_k",
    "film_w_m2_k",
    "exposed_regions",
    "internal_film_w_m2_k",
    "solar",
    "radiation",
    "energy_balance_tolerance",
    "provenance",
}
MATERIAL_KEYS = {
    "source",
    "conductivity_w_m_k",
    "emissivity",
    "solar_absorptance",
    "provenance",
    "synthetic",
}


def load_bridge(name, path):
    source = importlib.util.spec_from_file_location(name, path)
    bridge = importlib.util.module_from_spec(source)
    source.loader.exec_module(bridge)
    return bridge


def bounded(value, low, high, text):
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
        or not low < value <= high
    ):
        raise ValueError(text)
    return float(value)


def text(value):
    return isinstance(value, str) and value.strip() and len(value.encode()) <= 4096


def validate_thermal(thermal, names):
    if not isinstance(thermal, dict) or set(thermal) != THERMAL_KEYS:
        raise ValueError("exact resolved enclosure thermal model required")
    materials = thermal["materials"]
    if not isinstance(materials, dict) or sorted(materials) != sorted(names):
        raise ValueError("one resolved material per approved region required")
    for material in materials.values():
        if (
            not isinstance(material, dict)
            or set(material) != MATERIAL_KEYS
            or not text(material["source"])
            or not text(material["provenance"])
            or type(material["synthetic"]) is not bool
        ):
            raise ValueError("explicit material source and provenance required")
        bounded(material["conductivity_w_m_k"], 0, 1e4, "positive conductivity")
        for key in ("emissivity", "solar_absorptance"):
            if material[key] is not None:
                bounded(material[key], 0, 1, f"{key} in (0, 1] required")
    if thermal["heat_source_region"] not in names:
        raise ValueError("heat source must be an approved region")
    bounded(thermal["heat_source_power_w"], 0, 1e5, "positive heat source power")
    bounded(thermal["ambient_k"], 150, 500, "absolute ambient temperature in K")
    bounded(thermal["film_w_m2_k"], 0, 1e4, "positive exterior film coefficient")
    bounded(thermal["internal_film_w_m2_k"], 0, 1e4, "positive internal coefficient")
    bounded(thermal["energy_balance_tolerance"], 0, 1e-3, "energy gate <= 1e-3")
    exposed = thermal["exposed_regions"]
    if (
        not isinstance(exposed, list)
        or not exposed
        or exposed != sorted(set(exposed))
        or not set(exposed) <= set(names)
    ):
        raise ValueError("sorted unique exposed approved regions required")
    if type(thermal["radiation"]) is not bool or type(thermal["synthetic"]) is not bool:
        raise ValueError("explicit radiation and synthetic flags required")
    if thermal["radiation"] and any(
        materials[name]["emissivity"] is None for name in exposed
    ):
        raise ValueError("radiation requires a sourced emissivity per exposed region")
    solar = thermal["solar"]
    transparent = []
    if solar is not None:
        if not isinstance(solar, dict) or set(solar) - {
            "transparent_solar",
            "transparent_regions",
        } != {"irradiance_w_m2", "direction"}:
            raise ValueError("exact solar load required")
        if "transparent_solar" in solar or "transparent_regions" in solar:
            transparent = solar.get("transparent_regions")
            if (
                solar.get("transparent_solar") != TRANSPARENT_SOLAR
                or not isinstance(transparent, list)
                or not transparent
                or transparent != sorted(set(transparent))
                or not set(transparent) <= set(exposed)
                or any(
                    materials[n]["solar_absorptance"] is not None for n in transparent
                )
            ):
                raise ValueError(
                    "transparent_solar needs sorted exposed glazing regions without an absorptance"
                )
        bounded(solar["irradiance_w_m2"], 0, 2000, "solar irradiance in (0, 2000]")
        direction = solar["direction"]
        if (
            not isinstance(direction, list)
            or len(direction) != 3
            or not all(
                isinstance(v, (int, float))
                and not isinstance(v, bool)
                and math.isfinite(v)
                for v in direction
            )
            or abs(math.sqrt(sum(v * v for v in direction)) - 1) > 1e-9
        ):
            raise ValueError("unit solar direction required")
        if any(
            materials[name]["solar_absorptance"] is None
            for name in exposed
            if name not in transparent
        ):
            raise ValueError(
                "solar load requires a sourced absorptance per exposed region"
            )
    provenance = thermal["provenance"]
    if (
        not isinstance(provenance, dict)
        or not provenance
        or not all(text(k) and text(v) for k, v in provenance.items())
    ):
        raise ValueError("explicit boundary-input provenance required")


def validate(spec, solid):
    if (
        not isinstance(spec, dict)
        or set(spec) != {"schema_version", "geometry", "mesh_sha256", "thermal"}
        or type(spec["schema_version"]) is not int
        or spec["schema_version"] != 2
        or not isinstance(spec["mesh_sha256"], str)
        or len(spec["mesh_sha256"]) != 64
    ):
        raise ValueError("exact enclosure thermal descriptor required")
    solid.validate(spec["geometry"])
    names = [r["name"] for r in spec["geometry"]["regions"]]
    validate_thermal(spec["thermal"], names)


def boundary_model(record, thermal, solid):
    """Film/radiation/solar faces, cavity faces and the heat-source density.

    Sun-facing faces of `transparent_regions` deposit their full incident
    irradiance into the internal air node (conservative bound) instead of a
    surface flux.
    """
    nodes, elements = record["nodes"], record["elements"]
    region_of = {e: n for n, ids in record["regions"].items() for e in ids}
    exposed = set(thermal["exposed_regions"])
    solar = thermal["solar"]
    transparent = set((solar or {}).get("transparent_regions", []))
    film, cavity, sun, deposit, adiabatic = [], [], [], [], 0.0
    for eid, face, label in record["boundary_faces"]:
        area, normal = solid.face_area_normal(nodes, elements[eid], face - 1)
        region = region_of[eid]
        if label == "cavity":
            cavity.append((eid, face, area))
        elif region not in exposed:
            adiabatic += area
        else:
            film.append((eid, face, area, region))
            if solar is not None:
                cosine = solid.dot(normal, solar["direction"])
                if cosine > 0 and region in transparent:
                    deposit.append((eid, face, area, solar["irradiance_w_m2"] * cosine))
                elif cosine > 0:
                    absorptance = thermal["materials"][region]["solar_absorptance"]
                    sun.append(
                        (
                            eid,
                            face,
                            area,
                            absorptance * solar["irradiance_w_m2"] * cosine,
                        )
                    )
    if deposit and not cavity:
        raise ValueError("transparent solar deposit needs an internal cavity air node")
    source = thermal["heat_source_region"]
    volume = sum(
        solid.tet_volume(*(nodes[n] for n in elements[e][:4]))
        for e in record["regions"][source]
    )
    return {
        "region_of": region_of,
        "film": film,
        "cavity": cavity,
        "sun": sun,
        "cavity_solar": deposit,
        "adiabatic_ambient_area_m2": adiabatic,
        "source_volume_m3": volume,
        "power_density_w_m3": thermal["heat_source_power_w"] / volume,
    }


def ccx_number(value):
    """Shortest round-trip text, shortened to at most 20 characters per field."""
    text, precision = repr(float(value)), 17
    while len(text) > 20:
        precision -= 1
        text = f"{value:.{precision}g}"
    return text


def deck(record, thermal, model, internal_air_k):
    """CalculiX input; *FILM/*RADIATE/*DFLUX S use derived exterior faces."""
    lines = [
        "*HEADING",
        "Harbor enclosure steady conduction; SI m W K",
        "*NODE,NSET=NALL",
    ]
    lines += [
        f"{n}," + ",".join(ccx_number(v) for v in xyz)
        for n, xyz in sorted(record["nodes"].items())
    ]
    for name, ids in sorted(record["regions"].items()):
        lines.append(f"*ELEMENT,TYPE={record['element_type']},ELSET=E_{name.upper()}")
        lines += [f"{e}," + ",".join(map(str, record["elements"][e])) for e in ids]
    for name, ids in sorted(record["regions"].items()):
        material = thermal["materials"][name]
        lines += [
            f"*MATERIAL,NAME=M_{name.upper()}",
            "*CONDUCTIVITY",
            ccx_number(material["conductivity_w_m_k"]),
            f"*SOLID SECTION,ELSET=E_{name.upper()},MATERIAL=M_{name.upper()}",
        ]
    if thermal["radiation"]:
        lines.append(f"*PHYSICAL CONSTANTS,ABSOLUTE ZERO=0.,STEFAN BOLTZMANN={SIGMA!r}")
    ambient = ccx_number(thermal["ambient_k"])
    lines += ["*INITIAL CONDITIONS,TYPE=TEMPERATURE", f"NALL,{ambient}", "*STEP"]
    if thermal["radiation"]:
        lines += ["*HEAT TRANSFER,STEADY STATE,SOLVER=SPOOLES", "1.,1."]
    else:
        lines += ["*HEAT TRANSFER,STEADY STATE,SOLVER=SPOOLES,DIRECT", "1.,1."]
    lines += [
        "*DFLUX",
        f"E_{thermal['heat_source_region'].upper()},BF,"
        + ccx_number(model["power_density_w_m3"]),
    ]
    if model["sun"]:
        lines.append("*DFLUX")
        lines += [f"{e},S{f},{ccx_number(q)}" for e, f, _, q in model["sun"]]
    lines.append("*FILM")
    film = ccx_number(thermal["film_w_m2_k"])
    lines += [f"{e},F{f},{ambient},{film}" for e, f, _, _ in model["film"]]
    if model["cavity"]:
        air = ccx_number(internal_air_k)
        inner = ccx_number(thermal["internal_film_w_m2_k"])
        lines += [f"{e},F{f},{air},{inner}" for e, f, _ in model["cavity"]]
    if thermal["radiation"]:
        lines.append("*RADIATE")
        for e, f, _, region in model["film"]:
            emissivity = ccx_number(thermal["materials"][region]["emissivity"])
            lines.append(f"{e},R{f},{ambient},{emissivity}")
    frequency = 1 if thermal["radiation"] else 999999
    lines += [
        f"*NODE PRINT,NSET=NALL,FREQUENCY={frequency},GLOBAL=YES",
        "NT",
        "*END STEP",
    ]
    return "\n".join(lines) + "\n"


def face_temperatures(record, temperatures, eid, face, solid):
    order = 2 if record["element_type"] == "C3D10" else 1
    return [
        sum(temperatures[n] for n in point) / len(point)
        for point in solid.face_points(record["elements"][eid], face - 1, order)
    ]


def balance(record, thermal, model, temperatures, internal_air_k, solid):
    """Heat in (source + solar) against film + radiation out, midside quadrature."""
    ambient = thermal["ambient_k"]
    film = radiation = 0.0
    for eid, face, area, region in model["film"]:
        values = face_temperatures(record, temperatures, eid, face, solid)
        film += thermal["film_w_m2_k"] * area / 3 * sum(t - ambient for t in values)
        if thermal["radiation"]:
            emissivity = thermal["materials"][region]["emissivity"]
            radiation += (
                emissivity * SIGMA * area / 3 * sum(t**4 - ambient**4 for t in values)
            )
    air = 0.0
    for eid, face, area in model["cavity"]:
        values = face_temperatures(record, temperatures, eid, face, solid)
        air += (
            thermal["internal_film_w_m2_k"]
            * area
            / 3
            * sum(t - internal_air_k for t in values)
        )
    solar = sum(q * area for _, _, area, q in model["sun"])
    deposit = sum(q * area for _, _, area, q in model["cavity_solar"])
    # Net heat into the air node: cavity faces plus the deposited solar.
    air += deposit
    source = model["power_density_w_m3"] * model["source_volume_m3"]
    heat_in, heat_out = source + solar + deposit, film + radiation
    return {
        "heat_source_w": source,
        "solar_absorbed_w": solar,
        "solar_cavity_w": deposit,
        "film_w": film,
        "radiation_w": radiation,
        "heat_in_w": heat_in,
        "heat_out_w": heat_out,
        "relative_error": abs(heat_in - heat_out) / heat_in,
        "tolerance": thermal["energy_balance_tolerance"],
        "internal_air_residual_w": air,
        "internal_air_relative_error": abs(air) / heat_in,
    }


def statistics(record, temperatures, solid):
    nodes, elements = record["nodes"], record["elements"]
    result = {}
    for name, ids in sorted(record["regions"].items()):
        used = {n for e in ids for n in elements[e]}
        weighted = volume = 0.0
        for e in ids:
            v = solid.tet_volume(*(nodes[n] for n in elements[e][:4]))
            weighted += v * sum(temperatures[n] for n in elements[e][:4]) / 4
            volume += v
        values = [temperatures[n] for n in used]
        result[name] = {
            "max_k": max(values),
            "min_k": min(values),
            "mean_k": weighted / volume,
            "volume_m3": volume,
        }
    hottest = max(sorted(temperatures), key=temperatures.__getitem__)
    owners = sorted(
        name
        for name, ids in record["regions"].items()
        if any(hottest in elements[e] for e in ids)
    )
    return result, {
        "temperature_k": temperatures[hottest],
        "node": hottest,
        "position_m": nodes[hottest],
        "regions": owners,
    }


def solve(record, thermal, model, runner, solid):
    """Secant search for the internal air node so its net heat flow vanishes.

    `runner(internal_air_k)` returns nodal temperatures for a fixed air node.
    Linear models converge on the third evaluation; radiation adds a few.
    """
    ambient = thermal["ambient_k"]
    if not model["cavity"]:
        temperatures = runner(ambient)
        return temperatures, None, []
    target = 0.1 * thermal["energy_balance_tolerance"] * thermal["heat_source_power_w"]
    history = []
    conductance = thermal["internal_film_w_m2_k"] * sum(
        a for _, _, a in model["cavity"]
    )
    x = ambient
    previous = None
    for _ in range(MAX_AIR_RUNS):
        temperatures = runner(x)
        residual = balance(record, thermal, model, temperatures, x, solid)[
            "internal_air_residual_w"
        ]
        history.append({"internal_air_k": x, "residual_w": residual})
        if abs(residual) <= target:
            return temperatures, x, history
        if previous is None or residual == previous[1]:
            step = residual / conductance
        else:
            step = -residual * (x - previous[0]) / (residual - previous[1])
        previous = (x, residual)
        x += step
        if not 150 < x < 1000:
            break
    raise ValueError("internal air balance did not converge within the run budget")


def ccx_runner(record, thermal, model, fem, calculix, version, log):
    def run(internal_air_k):
        for stale in Path.cwd().glob("reference.*"):
            stale.unlink()
        Path("reference.inp").write_text(deck(record, thermal, model, internal_air_k))
        environment = {
            "OMP_NUM_THREADS": "1",
            "CCX_NPROC_RESULTS": "1",
            "CCX_NPROC_EQUATION_SOLVER": "1",
            "OPENBLAS_NUM_THREADS": "1",
            "MKL_NUM_THREADS": "1",
            "HOME": "/nonexistent",
            "LC_ALL": "C",
        }
        with Path(log).open("ab") as stream:
            process = subprocess.run(
                [calculix, "-i", "reference"],
                env=environment,
                stdout=stream,
                stderr=subprocess.STDOUT,
                timeout=1800,
                check=False,
            )
        output = fem.read_regular(log, 64 * 1024**2).decode()
        if (
            process.returncode
            or "*ERROR" in output
            or f"Version {version}" not in output
            or "Job finished" not in output.rsplit("Version", 1)[-1]
        ):
            raise ValueError("exact native CalculiX enclosure solve did not succeed")
        fields = fem.read_dat(fem.read_regular("reference.dat", 256 * 1024**2).decode())
        snapshots = fields.get("temperature", [])
        if (
            set(fields) != {"temperature"}
            or not snapshots
            or snapshots[-1]["time"] != 1
        ):
            raise ValueError("one converged final steady temperature field required")
        values = snapshots[-1]["values"]
        temperatures = {key[0]: value[0] for key, value in values.items()}
        if set(temperatures) != set(record["nodes"]) or not all(
            math.isfinite(t) and t > 0 for t in temperatures.values()
        ):
            raise ValueError("complete finite absolute native temperatures required")
        return temperatures

    return run


def evaluate(record, thermal, model, runner, solid):
    """Solve, then gate and summarize; shared by the stage and verification."""
    temperatures, internal_air_k, history = solve(record, thermal, model, runner, solid)
    energy = balance(
        record,
        thermal,
        model,
        temperatures,
        thermal["ambient_k"] if internal_air_k is None else internal_air_k,
        solid,
    )
    if energy["relative_error"] > energy["tolerance"] or (
        energy["internal_air_relative_error"] > energy["tolerance"]
    ):
        raise ValueError(f"steady energy balance gate failed: {energy}")
    regions, hottest = statistics(record, temperatures, solid)
    return temperatures, {
        "regions": regions,
        "global_max": hottest,
        "internal_air_k": internal_air_k,
        "air_iterations": history,
        "energy_balance": energy,
        "boundary_area_m2": {
            "film": sum(a for _, _, a, _ in model["film"]),
            "solar": sum(a for _, _, a, _ in model["sun"]),
            "solar_cavity": sum(a for _, _, a, _ in model["cavity_solar"]),
            "cavity": sum(a for _, _, a in model["cavity"]),
            "adiabatic_ambient": model["adiabatic_ambient_area_m2"],
        },
        "heat_source_volume_m3": model["source_volume_m3"],
    }


def applicability(thermal):
    if (thermal["solar"] or {}).get("transparent_solar") is None:
        return APPLICABILITY
    return {**APPLICABILITY, "transparent_solar": TRANSPARENT_SOLAR_STATEMENT}


def main(raw, spec, fem, request_path):
    """FemImported stage entry for the version-2 enclosure descriptor."""
    solid = load_bridge("harbor_cad_solid_mesh", "@solid_mesh_bridge@")
    validate(spec, solid)
    sandbox = fem.cpu_sandbox(
        "HARBOR_CAD_FEM_IMPORTED_POLICY",
        "harbor-cad-fem-imported-cpu-v1",
        "/fem-imported-runtime-closure.txt",
        request_path,
    )
    geometry, thermal = spec["geometry"], spec["thermal"]
    originals = solid.check_sources(geometry, fem, sandbox)
    if any(p.name != "fem-imported.log" for p in Path.cwd().iterdir()):
        raise ValueError("new empty stage-local enclosure thermal output required")
    record, facts = solid.generate(geometry, fem.gmsh_api())
    mesh_sha256 = solid.write_mesh(record)
    if mesh_sha256 != spec["mesh_sha256"]:
        raise ValueError("re-meshed solids differ from the approved mesh job")
    model = boundary_model(record, thermal, solid)
    runner = ccx_runner(
        record, thermal, model, fem, "@calculix@", "@ccx_version@", "calculix.log"
    )
    _, result = evaluate(record, thermal, model, runner, solid)
    solid.recheck_sources(geometry, fem, originals)
    data = fem.read_regular("reference.dat", 256 * 1024**2)
    fem.atomic_json(
        "fem-imported-receipt.json",
        {
            "schema_version": 1,
            "adapter": "CalculiX",
            "backend": "cpu",
            "factorization": "SPOOLES",
            "executed": True,
            "software_fallback": False,
            "synthetic": thermal["synthetic"] or geometry["synthetic"],
            "precision": "float64",
            "formulation": FORMULATION,
            "calculix_version": "@ccx_version@",
            "gmsh_version": "@gmsh_version@",
            "calculix_source_sha256": CALCULIX_SOURCE_SHA256,
            "gmsh_source_sha256": solid.GMSH_SOURCE_SHA256,
            "request_sha256": hashlib.sha256(raw).hexdigest(),
            "mesh_sha256": mesh_sha256,
            "native_field_sha256": hashlib.sha256(data).hexdigest(),
            "element_type": facts["element_type"],
            "nodes": facts["nodes"],
            "elements": facts["elements"],
            **result,
            "temperature_unit": "K",
            "applicability": applicability(thermal),
            "gap_healing": False,
            "physical_validation": "unqualified",
            **({"sandbox": sandbox} if sandbox is not None else {}),
        },
    )


if __name__ == "__main__":
    sys.exit("enclosure_thermal is loaded by the fem-imported adapter")
