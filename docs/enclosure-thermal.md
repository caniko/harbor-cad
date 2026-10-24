# Enclosure thermal on imported general solids

A steady, conjugate-free conduction recipe for the outdoor enclosure: Gmsh
meshes the registered closed solids conformally, CalculiX solves steady heat
transfer, and Rust recomputes the receipt from the native field. It removes
the two design-loop blockers: the box-only mesh/FEM allowlist and the imported
FEM receipt without a temperature.

## Plans

Both plans carry one exclusive field, `cad_solids` (`src/enclosure_thermal.rs`);
the box path (`cad_source`, `imported_fem`) is unchanged.

| Request | Operation (CLI / chain) | Plan |
|---|---|---|
| `{source_job, regions, mesh}` | `cad solid-mesh` / `plan_cad_solid_mesh` | v7, stage `mesh` (`cad_mesh`) |
| `{mesh_job, spec}` | `enclosure-thermal` / `plan_enclosure_thermal` | v8, stage `fem-imported` (`fem_imported`) |

`regions` is a sorted list of 1–8 registered region names of an authorized CAD
job (original or v2 sheet variant). `mesh` is explicit:
`characteristic_length_max_m` (Gmsh `Mesh.CharacteristicLengthMax`),
`element_order` 1 (C3D4) or 2 (C3D10), `max_elements` (≤ 1,000,000, and at least
the regular-tetrahedron count for the volume), `volume_relative_tolerance`
(≤ 0.1), `min_quality` and `geometry_tolerance_m`. Planning resolves every BREP
from registered evidence and refuses unmapped regions; submission retains
distinct-inode copies under `retained-solids/` like the box source.

The thermal plan binds a succeeded mesh job: its plan digest, its registered
`mesh.json` digest and an unchanged re-resolved CAD source. The thermal stage
re-meshes with the same pinned single-thread Gmsh and refuses unless the bytes
reproduce that digest.

## Mesh (`adapters/solid_mesh.py`)

Each region BREP is imported with the unhealed uniform OCC scaling of
[CAD mesh correspondence](cad-mesh.md) and must be exactly one closed solid
whose OCC volume and bounds match the CAD evidence. `occ.fragment` makes
touching regions share conformal interfaces; overlapping regions refuse. HXT
(`Mesh.Algorithm3D=10`) with Gmsh and Netgen optimization produces the tetrahedra.
The receipt (`cad-mesh-receipt.json`) reports per-region CAD and mesh volume,
element counts, min/mean shape ratio `6√2·V/l_rms³`, Gmsh min/mean SICN,
interface areas and ambient/cavity areas. Rust re-derives orientation,
quality, per-region volume, the exact set of unshared faces and interface
areas from `mesh.json`, and checks the element budget.

Boundary faces are labelled per CAD surface: `ambient` when at least 4 of 64
deterministic hemisphere rays from its largest triangle escape every meshed
solid, otherwise `cavity`. A surface is labelled as a whole, so a partly
shaded surface takes one label; the receipt's areas make this auditable.

## Model (`adapters/enclosure_thermal.py`)

- Conductivity per region, constant and isotropic. A `library` material takes a
  W1 `Library::builtin()` id or a design `route` `MATERIAL__PROCESS` and uses the
  conservative range ends: conductivity low, emissivity low, solar absorptance
  high. `declared` materials are `PhysicalInput`s.
- Heat source: `heat_source_power` (W) in `heat_source_region` as `*DFLUX BF`,
  power divided by the meshed corner-node volume.
- Exterior: `*FILM` at `film_coefficient` and `ambient_temperature` on ambient
  faces of `exposed_regions`. Other ambient faces are adiabatic.
- Solar (optional): `*DFLUX S` = absorptance × irradiance × max(0, n·s) on the
  same faces. A missing absorptance for any exposed region refuses planning.
- Radiation (optional): `*RADIATE` to ambient with each exposed region's
  emissivity; missing emissivity refuses.
- Internal air (modelling assumption): every cavity face exchanges heat with
  one well-mixed air node through `internal_film_coefficient`. The adapter
  runs a secant search on the node temperature until its net heat flow is
  below a tenth of the energy gate. With the fixture's bosses, the PCB also
  conducts through conformal contact. There is no CFD and no contact resistance.

Every physical value must be `known` with provenance; a `missing` one refuses
planning with its reason. Nothing is defaulted. The resolved model, with
per-input provenance and a synthetic flag, is part of the approved plan.

## Outputs

`fem-imported-receipt.json` (with `mesh.json`, `reference.inp`, `reference.dat`)
reports per-region `max_k`, `min_k` and volume-weighted `mean_k`, the global
maximum node, position and regions, the internal air temperature and secant
history, and the energy balance: heat source + absorbed solar against film +
radiation, all integrated with three edge-midpoint points per face. The
relative error must be ≤ `energy_balance_tolerance` (≤ 1e-3), as must the
air-node residual. Rust recomputes the statistics and every energy term from
the native field and mesh before publication. The receipt includes an
`applicability` statement: steady state, no internal convection, effective
internal coefficient, perfect interfaces, no condensation, moisture or mounting
conduction.

## Verification

`scripts/verify_enclosure_thermal_cpu.py` runs the recipe's boundary model,
secant search, energy balance and statistics against four analytic references,
each at two refinements: a 1D slab with uniform heat and film on both faces, a
lumped cube (Bi = 1e-4), a two-region series conductance and a lumped
internal-air chain. In CI it uses an independent pure-Python linear-tetrahedron
solver. `--ccx PATH --ccx-version V` is the opt-in native gate: it solves the
same CalculiX deck that the stage writes. The structured-mesh error converges
at O(h), because the transverse resolution is fixed. Gates are 1e-2 (slab),
2e-2 (series) and 2e-3 (lumped cases).

## Design loop

`examples/design-enclosure.json` chains variant → `plan_cad_solid_mesh` →
`plan_enclosure_thermal`. `max_temperature_k` reads
`/regions/housing/max_k` from the thermal receipt, and `temperature_margin_k`
compares it with the route material's conservative service limit.
Ambient, irradiance and exterior film come from the cited ETSI EN 300 019-1-4
class 4.1 hot design point, and the minimum service check uses its cold point
(see [design conditions](design-conditions.md)). A library material may name a
`finish` from `data/finishes.json`; its sourced absorptance and emissivity
override the material's, and missing finish values fall back to it. The
categorical `finish` gives PA12 a sourced absorptance (black pigment) and the
aluminium routes RAL 7035 powder coat. PCB power, board conductivity, the
internal cavity film and the component limit are product-specific operator
inputs declared `missing`, so the thermal child is blocked and those constraints
stay unknown. The pc-uv window is transparent glazing. Its solar load is
transmittance, which this model does not represent, so its solar-loaded solve
refuses.
`examples/design-enclosure-demo.json` uses `synthetic: true` values and a
declared window. `design status`/`export` then list `synthetic_inputs`, and
the export statement starts with `SYNTHETIC`.

## Limits

This recipe is not qualified. The native Gmsh/CalculiX worker gate waits on the
security hold, and no CalculiX run has been observed. It does not cover
transients, internal convection, view-factor radiation, sky temperature,
condensation, temperature-dependent properties, contact resistance or
mounting conduction. Process success is not physical validation.

Development smoke (not qualification): the local Gmsh 4.15.2 store path meshed
the five-region enclosure fixture BREP (FreeCAD 1.1.3, outside the sandbox) at
8, 6 and 4 mm. These runs gave 22,783, 36,826 and 71,950 C3D4 elements, with a
min shape ratio of 0.056, 0.097 and 0.19, deterministic bytes and a housing
volume error of 2e-4. With synthetic inputs, the Python reference solver (no
CalculiX) closed the energy balance to 8e-6.
