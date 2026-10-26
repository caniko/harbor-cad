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

- Conductivity per region, constant and isotropic, or orthotropic for a
  declared plate (below). A `library` material takes a
  W1 `Library::builtin()` id, a design `route` `MATERIAL__PROCESS` or a triple
  `MATERIAL__PROCESS__FINISH` (the finish named once, here or in `finish`), and
  uses the conservative range ends: conductivity low, emissivity low, solar
  absorptance high. `declared` materials are `PhysicalInput`s.
- Orthotropic plate (optional, e.g. the PCB): a `declared` material with
  `in_plane_conductivity` uses `conductivity` along the region's thinnest world
  axis and the in-plane value along the other two. Planning refuses unless the
  CAD volume fills the world bounding box within 1e-9 (axis-aligned box) and
  the thickness is at most half of each other extent. The deck writes
  `*CONDUCTIVITY,TYPE=ORTHO` (data line `k11,k22,k33`) and an explicit identity
  `*ORIENTATION,NAME=OR_<REGION>` (`1.,0.,0.,0.,1.,0.`: point a on local x,
  point b in local xy) referenced by `*SOLID SECTION,...,ORIENTATION=` (CalculiX
  manual, `*CONDUCTIVITY`, `*ORIENTATION`, `*SOLID SECTION`). The reference
  solver uses the same diagonal tensor in its tetra stiffness. The pilot uses
  the cited low ends, 8.1 in-plane and 0.26 W/(m·K) through-plane. Both are
  lower bounds, the in-plane one only for boards with continuous copper planes
  (a board requirement; see [design conditions](design-conditions.md)).
- Heat source: `heat_source_power` (W) in `heat_source_region` as `*DFLUX BF`,
  power divided by the meshed corner-node volume.
- Exterior: `*FILM` at `film_coefficient` and `ambient_temperature` on ambient
  faces of `exposed_regions`. Other ambient faces are adiabatic.
- Solar (optional): `*DFLUX S` = absorptance × irradiance × max(0, n·s) on the
  same faces. A missing absorptance for any exposed region refuses planning.
- Transparent glazing (`pc`, `pc-uv`, `pmma`) has transmittance semantics and no
  sourced absorptance. Under solar it refuses unless the spec sets
  `solar.transparent_solar: "deposit_incident_in_cavity"`, a conservative
  bound (modelling assumption): the full irradiance incident on the pane's
  sun-facing exterior faces, G × max(0, n·s) × A, is added to the internal air
  node. Nothing is reflected, absorbed in the pane or re-emitted outward, so
  interior solar gain is an upper bound; the pane's own temperature is not
  bounded. It requires a cavity. The receipt's `applicability.transparent_solar`
  and the design export's `conservative_bounds` carry the
  `conservative_bound` label. No sourced solar direct transmittance/absorptance
  split (EN 410) was found for the library glazing; datasheets give light
  transmission, shading coefficient or total g only, so no physically split
  option exists.
- Radiation (optional): `true` writes `*RADIATE` to ambient with each exposed
  region's emissivity; missing emissivity refuses.
- Linearized radiation (`radiation: "linearized_lower_bound"`, conservative
  bound): each exposed region's `*FILM` coefficient becomes h + h_r with
  h_r = 4εσTa³ (low-end ε; a region without a sourced ε gets no credit, never
  a default). For T ≥ Ta, T⁴ − Ta⁴ = (T − Ta)(T³ + T²Ta + TTa² + Ta³) ≥
  4Ta³(T − Ta), and by convexity of T⁴ it holds for any T. Surroundings colder
  than the ambient air (sky) only add cooling, so the ambient sink understates
  cooling too. The model stays linear. The receipt reports `exterior_min_k`
  (coldest node on exposed exterior faces); the adapter and Rust refuse the
  claim if it is below ambient. `applicability.radiation` and the design
  export's `conservative_bounds` carry the `conservative_bound` label.
- Power sweep (optional `power_sweep: true`): the design loop superposes solves
  at two heat-source powers. That is exact only for a linear model, so
  planning refuses `power_sweep` together with `radiation: true`; the
  linearized bound is allowed. Constant films, constant solar and the air-node
  coupling are linear, so every nodal temperature is affine in power. Dropping
  radiation, or crediting only its linearized lower bound, is conservative.
  The cavity radiation bound (below) is linear too, so it is allowed.
- Cavity radiation (optional `cavity_radiation: "linearized_lower_bound"`,
  conservative bound): credits board-to-wall radiation inside the cavity as a
  provable lower bound, or credits nothing.
  - Geometry: the board is `heat_source_region`. Planning refuses unless it is
    a convex axis-aligned box (CAD volume fills its bounding box within 1e-9),
    and the adapter refuses unless every board boundary face is a cavity face.
    The walls are all other cavity faces, which assumes one cavity, as the air
    node does. Because the board is convex and fully enclosed, the view factor
    board → walls is 1.
  - Coefficient: the two-surface grey diffuse enclosure gives
    ε_eff = 1/(1/ε_b + (A_b/A_w)(1/ε_w − 1)). It uses low-end *interior*
    emissivities. ε_w is the minimum over the wall regions, and a finished
    library material takes the lower of finish and bare material, because
    interiors may be uncoated (aluminium + white powder coat: 0.03). Without a
    sourced ε for the board or any wall region there is no credit, never a
    default. The credited coefficient is h_cr = 4·ε_eff·σ·Ta³.
  - Proof: in the two-surface model the exchange is spread over face pairs in
    proportion to area, so the net board → wall heat is
    Σ_ij ε_eff·σ·a_i·b_j/A_w·(T_i⁴ − T_j⁴). For T_i ≥ T_j, the tangent of the
    convex T⁴ at T_j gives T_i⁴ − T_j⁴ ≥ 4T_j³(T_i − T_j). If also T_j ≥ Ta,
    this is ≥ 4Ta³(T_i − T_j). Summing over pairs, the exchange is at least
    h_cr·A_b·(T̄_b − T̄_w), with area-mean temperatures. The adapter realises
    exactly this term through one radiation node: board faces couple to it at
    2·h_cr and wall faces at 2·h_cr·A_b/A_w. Equal total conductances put the
    node at (T̄_b + T̄_w)/2, so it carries h_cr·A_b·(T̄_b − T̄_w). The term is
    linear, so superposition stays exact.
  - Postcondition: the pair inequality needs T_i ≥ T_j ≥ Ta for *every* pair.
    FE surfaces are not isothermal, so the receipt checks the extremes: the
    minimum board-surface node must be ≥ the maximum wall node, and the
    minimum wall node ≥ Ta. Nodes are corners plus C3D10 midsides, the
    quadrature points. This is sufficient for every pair; comparing area means
    would only bound the idealised isothermal-surface model, not the FE field.
    If the check fails, the credit is invalid. The adapter and Rust both
    refuse, so no receipt is published and the design metric stays unknown,
    never optimistic. In a power sweep, nodal temperatures are affine in
    power, so (min board − max wall) and (min wall − Ta) are concave. A check
    that passes at both ends therefore holds across the whole interval.
  - CalculiX: a later `*FILM` on the same face replaces the earlier one (ccx
    2.23 manual, §7.59 `*FILM`). Each credited cavity face therefore gets one
    combined film: h_a + h_r to the sink (h_a·T_air + h_r·T_rad)/(h_a + h_r),
    which is exactly h_a(T − T_air) + h_r(T − T_rad). As with the air node,
    the radiation node is solved outside CalculiX, by a Broyden search over
    (T_air, T_rad); the start Jacobian is diagonal, and in 1D this is the
    secant search. Forced-convection fluid nodes (`FxFC`) were not used: they
    need network elements, and no native run can verify them yet.
  - In this geometry the credit rarely qualifies, for two reasons. First, a
    board mounted on wall regions (the bosses) shares contact nodes with them,
    and the gradient across the contact puts the board's coldest surface node
    below the boss's hottest one. Second, at low power the sun-heated lid and
    walls exceed the board's coldest node. Even when it qualifies, the credit
    is small: with ε_b 0.15 and an interior ε_w 0.03, ε_eff ≈ 0.11, so
    h_cr ≈ 0.77 W/(m²·K) at the hot point. That is below the 1.2 W/(m²·K)
    cavity film, about 5.8 mW/K on the analogue's 7.5e-3 m² board.
- Internal air (modelling assumption): every cavity face exchanges heat with
  one well-mixed air node through `internal_film_coefficient`. The adapter
  runs a secant search on the node temperature (a Broyden search together
  with the radiation node when cavity radiation is credited) until each net
  heat flow is below a tenth of the energy gate. With the fixture's bosses, the PCB also
  conducts through conformal contact. There is no CFD and no contact resistance.

Every physical value must be `known` with provenance; a `missing` one refuses
planning with its reason. Nothing is defaulted. The resolved model, with
per-input provenance and a synthetic flag, is part of the approved plan.

## Outputs

`fem-imported-receipt.json` (with `mesh.json`, `reference.inp`, `reference.dat`)
reports per-region `max_k`, `min_k` and volume-weighted `mean_k`, the global
maximum node, position and regions, the internal air temperature and secant
history, and the energy balance: heat source + absorbed solar + glazing
solar deposited in the cavity (`solar_cavity_w`) against film + radiation, all integrated with three edge-midpoint points per face. The
relative error must be ≤ `energy_balance_tolerance` (≤ 1e-3), as must the
air-node residual and, when cavity radiation is credited, the radiation-node
residual. Rust recomputes the statistics and every energy term from
the native field and mesh before publication. The receipt includes an
`applicability` statement: steady state, no internal convection, effective
internal coefficient, perfect interfaces, no condensation, moisture or mounting
conduction. With `cavity_radiation`, the receipt also carries a block with
board and wall areas, emissivities, ε_eff, h_cr, the radiation node and the
postcondition extremes; Rust recomputes all of them from the mesh and field.
`applicability.cavity_radiation` and the design export's `conservative_bounds`
carry the `conservative_bound` label.

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
two `plan_enclosure_thermal` solves at 1 W and 15 W PCB power in
`power_sweep` mode with the linearized radiation bound and the orthotropic
board. Their receipts (`/regions/pcb/max_k`,
`/internal_air_k`, `/regions/housing/max_k`) feed `max_supported_pcb_power_w`
by linear superposition (see [design loop](design-loop.md)). On the refine
tier, a third solve at that power must match the superposed temperatures
within 0.5 K. `temperature_margin_k` compares the 1 W housing peak and the
cold point with the route material's conservative service limits.
Ambient, irradiance and exterior film come from the cited ETSI EN 300 019-1-4
class 4.1 hot design point, and the minimum service check uses its cold point
(see [design conditions](design-conditions.md)). The internal film is the
cited conservative low end of `enclosed-cavity-natural` (1.2 W/(m²·K)). The
component limit (85 °C) and the board conductivities (8.1 in-plane,
0.26 W/(m·K) through-plane) are cited conservative defaults, not product data.
The export's `requirements` carries the continuous-copper-plane board
requirement and the +85 °C component rating with the winner.
The single categorical `route` lists only compatible `MATERIAL__PROCESS__FINISH`
triples; the finish's sourced optics override the material's. The pc-uv window
uses the `transparent_solar` conservative bound.
`python/tests/test_enclosure_feasibility.py` checks the superposition against
the pure-Python reference solver on a structured analogue of the fixture.
With the radiation off or linearized, the chords hold within 0.1 K, and with
T⁴ radiation they do not. Supported power of the aluminium/white-powder-coat
candidate (verified by a third solve within 0.5 K; the PCB peak binds):

| Credit | With window | Opaque lid |
|---|---|---|
| none (isotropic 0.26, no radiation) | none (air 373.6 K at 1 W) | 2.04 W |
| linearized radiation | 10.11 W | 10.77 W |
| orthotropic board | none | 2.08 W |
| both (pilot) | 10.42 W | 11.10 W |

The window cavity deposit and the 1.2 W/(m²·K) cavity film stay. The
analogue's uniform board source and coarse board mesh leave little for
in-plane spreading to do, so the orthotropic gain there is small.

The pilot leaves `cavity_radiation` off. On the analogue with both credits
on, the strict postcondition fails (minimum board-surface node against
maximum wall node, reference solver):

| Case | Board surface min | Wall max |
|---|---|---|
| window, 1 W | 325.28 K | 325.81 K |
| window, 15 W | 345.97 K | 351.34 K |
| opaque lid, 1 W | 323.27 K | 323.96 K |

Every credited solve is refused, so enabling it would turn 10.42 W / 11.10 W
into unknown. At 15 W the wall maximum is a boss node in contact with the
board; at 1 W the walls warmed by solar gain are hotter than the board's
coldest node. `test_cavity_radiation_postcondition_fails_on_the_analogue`
pins these values.
`examples/design-enclosure-demo.json` uses `synthetic: true` values and a
declared window. `design status`/`export` then list `synthetic_inputs`, and
the export statement starts with `SYNTHETIC`.

## Limits

This recipe is not qualified. The native Gmsh/CalculiX worker gate waits on the
security hold, and no CalculiX run has been observed. It does not cover
transients, internal convection, view-factor radiation (the cavity bound uses
the two-surface enclosure model), sky temperature
(the linearized bound only ignores sky cooling, which is conservative),
condensation, temperature-dependent properties, contact resistance or
mounting conduction. Process success is not physical validation.

Development smoke (not qualification): the local Gmsh 4.15.2 store path meshed
the five-region enclosure fixture BREP (FreeCAD 1.1.3, outside the sandbox) at
8, 6 and 4 mm. These runs gave 22,783, 36,826 and 71,950 C3D4 elements, with a
min shape ratio of 0.056, 0.097 and 0.19, deterministic bytes and a housing
volume error of 2e-4. With synthetic inputs, the Python reference solver (no
CalculiX) closed the energy balance to 8e-6.
