# Source-bound retained wetting distribution

`results retain-wetting REQUEST.json` and results/all MCP
`results_retain_wetting` prepare the same bounded read-only one-way extrusion of
an exact retained native wetting observation. The source must be a succeeded,
execution-bound version-9 native job whose complete registered originals still
pass the independent phase-mass and contact-angle gates. The response carries
identities and reductions; complete phase/velocity arrays stay in the original
CSV artifacts and the normal offline export bundle.

## Request and conservative map

```json
{
  "schema_version": 1,
  "source_job": "00000000-0000-0000-0000-000000000001",
  "physical_time_s": 0.0,
  "extrusion": { "value": 1.0, "unit": "mm" },
  "extrusion_provenance": "explicit synthetic depth, not measured droplet volume",
  "destination_region": "retained_phase",
  "destination_origin_m": [0.0, 0.0, 0.0],
  "maximum_relative_conservation_error": 1e-10
}
```

Use an actual source job ID and an **exact registered time**, including zero if
retained. The two-dimensional wetting solver supplies no extrusion depth. It
must be explicit, positive, finite, at most 1 m, and accompanied by provenance.
The destination is a named translated congruent control grid with unchanged
orientation `[0,0,1]`. Rotations, resampling, unretained times and temperature
injection are not supported by this operation.

Every native material-1 point maps to one extruded nodal control volume
`dx² * depth`. Material-2 wall nodes carry no retained phase mass. Original
coordinates, phase and velocity retain their point identities; the descriptor
provides the explicit destination cell association and translation. No
nearest-neighbour matching, phase thresholding, clipping, node averaging or
uniform-reservoir redistribution is performed.

For the pinned wetting formulation, the signed liquid-phase amount is `1-phi`.
Its density is `rho_liquid * (1-phi)` in kg/m³ and its complete extruded integral
is `rho_liquid * depth * sum((1-phi)*dx²)` in kg. Rust compares the independent
complete source area reduction with the per-cell extruded mass reduction using
compensated accumulation. The approved relative conservation limit cannot
exceed `1e-10`. This conservation check is separate from the native wetting
evolution's approved `1e-3` phase-mass gate.

The native control-volume convention includes all x points and interior y
points, matching the authoritative native phase-area calculation. For native
shape `[nx,ny]`, source control bounds are
`[-dx/2,(nx-1/2)*dx] × [dx/2,(ny-3/2)*dx] × [0,depth]`, before translation.
They are recorded explicitly rather than clipped to a different nominal box.
Translations that cannot preserve resolved Float64 control geometry reject.
Native lattice velocity converts to `[u,v,0] * dx/dt` in m/s using the exact
source descriptor; the original lattice components remain authoritative.

## Cooling prerequisites and scope

This is a **synthetic equal-density/equal-viscosity diffuse phase distribution**,
not qualified real water-air retention, resolved ingress or a waterproofness
conclusion. Native phase overshoots remain signed, with their range, nonphysical
cell count and integrated negative/excess phase amounts reported explicitly.
Overshoots block interpretation as a bounded physical liquid initialization;
the transfer does not silently repair them.

The wetting source contains no temperature or enthalpy field. Reports therefore
retain `temperature_state: missing_from_native_wetting_source`, explicit cooling
prerequisites, `executed: false` and `physical_validation: unqualified`. A cooling
consumer needs a declared thermal initial state, heat capacity/conductivity,
latent heat, material temperature domain, boundary histories and a supported
phase-change formulation. The existing Stefan reference accepts a different
uniform-liquid conduction geometry; this descriptor alone does not authorize
arbitrary wetting distributions in that solver.

The initialization identity binds the request, original registered field/receipt,
scientific/execution/binding identities, complete mass result and exact extruded
geometry. Query failures do not change source jobs or artifacts.

## Verification

Rust tests cover complete synthetic cap fields, SI extrusion scaling,
translation/control bounds, original Float64 center values, signed overshoots,
truncated/duplicate nodes, unavailable times, invalid quantities, changed gates
and unexecuted fixture rejection. Real protocol tests exercise generated schemas,
profile exposure and typed errors through the same worker.

The dedicated original-field CLI/MCP gate is:

```sh
python3 scripts/verify_wetting_retention.py \
  --binary CLI --mcp MCP --source QUALIFIED_WETTING_WORKER_DIRECTORY \
  --output NEW_DIRECTORY
```

It runs under the normal host runtime lease and bounded service, copies only a
closed source state for read-only queries, independently integrates every
retained native CSV at two explicit depths, compares CLI/MCP reports and tests
corruption/unsupported-input rejection. It verifies the original source tree
and database record counts remain unchanged. `--development` explicitly labels
a source-built CLI/Python-module diagnostic as package-unqualified. Exact
packaged result qualification and downstream native cooling integration remain
separate gates.
