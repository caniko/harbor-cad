# Registered native result sampling and comparison

## Explicit coupled thermal histories

`results sample-thermal`, `compare-thermal` and `moisture` accept
`thermal_stage: 0` (the approved lower history) or `thermal_stage: 1` (upper)
for a succeeded version-11 thermal/contact job. Each source keeps its own
`thermal-lower` / `thermal-upper` native DAT, full mesh and retained temperature
history. The worker verifies that selected stage's original execution and
numerical evidence before reading its fields; it checks every original native
time/node, not only the requested sample.

The selector is required for a coupled source. Standalone thermal jobs omit it
and retain their previous request/report serialization. A supplied index on a
standalone source, absent index on a coupled source, foreign stage/path and
unretained time refuse. Coupled responses carry the selected index and exact
original artifact hashes. `compare-thermal` may compare explicitly selected
histories only under the existing same-mesh/association gate, without numerical
or physical acceptance inference.

Moisture screening derives the full geometrically identified source surface
minimum from that history, then applies the explicit air input, justified
inapplicability or missing-input record. Local thermal-block coordinates remain
local coordinates; projection into contact mechanics is not re-applied to a
surface query. These are read-only original-field assessments, with physical
validation remaining unqualified.

`scripts/verify_coupled_thermal_results.py` exercises both independently
qualified native coupling jobs, both histories, all retained times, all six
surfaces and all three moisture branches. It compares CLI/MCP, independently
selects geometrical face minima, rejects stage/time/path and original-DAT
substitutions, and verifies source/database immutability. An explicit
`--development` diagnostic remains package-unqualified.

`results sample REQUEST.json` and results-profile MCP `results_sample` use the
same read-only Rust worker. The version-1 request contains `job_id`, `field` and
`locations`. The initial allowlist is registered succeeded v5/v8 static FEM:
`temperature`/`displacement` at explicit `{"association":"node","node_id":7}`,
or `heat_flux`/`stress` at
`{"association":"integration_point","element_id":42,"integration_point":1}`.
Requests contain 1–64 distinct locations and reject arbitrary artifact paths,
unmapped times, coordinate interpolation and unsupported fields/associations.

Sampling binds registered JSON, authoritative CalculiX DAT and mesh bytes, recorded science and
execution identities and passing scoped native numerical evidence. Results
retain units, component counts, explicit native IDs, coordinate unit, source
artifact identity and an absent physical time for static data. Arrays remain in
registered artifacts; the response includes only the selected bounded values.
Each registered mesh, JSON field or DAT object is bounded to 32 MiB for these
read-only result operations; larger native objects require a separately budgeted
sampling capability.
The requested complete JSON field is independently matched to its final native
DAT section and exact node/integration-point set before any value is returned.
The pinned `serde_json` parser uses `float_roundtrip`: the default best-effort
float parser changed a retained near-zero stress (`-5.293956e-23`) by one bit.
Native comparisons retain exact Float64 equality rather than adding a tolerance.

The packaged `results-3` gate passed on 2026-10-06 with eight fields from the
qualified v5/v8 conduction/free-expansion sources, CLI/MCP equality, exact
native DAT coverage and 40 input/byte/identity rejection cases. The report is
`/data/scratch/tmp/opencode/harbor-cad-authority-20261005/results-3/verification.json`,
SHA-256 `d092b8d4f09b4847db339d9e064a32666b75fc8e0286116c7f2bfec5a07283f9`.
Package revision: `55f18c30f55fc6092c70f39c7330856b9b968969`; qualifier revision:
`284d92d8f66d81c937260d19e7f66ec8b3f636f2`. This gate preserved original source
bytes and database counts; same-job comparisons produced exact zero signed
differences. Broader fields, interpolation and mesh-mismatched comparison remain
separate capabilities.

`results compare REQUEST.json`/MCP `results_compare` accept version 1 with
`left` and `right` sampling requests. They require the same field, locations,
units, components and exact mesh bytes. They report `right-left` differences
and the maximum absolute difference. Comparisons have no implicit registration,
resampling, relative-error denominator, numerical acceptance or physical
validation claim. Different geometry or native node/integration associations
reject; material/science/runtime identities stay explicit on both sides.

## Retained thermal time samples

`results sample-thermal REQUEST.json` and results-profile MCP
`results_sample_thermal` accept an independent strict version-1 request:

```json
{
  "schema_version": 1,
  "job_id": "a05f78ac-a7ce-4aed-a458-e4a4cbf0b9fc",
  "field": "temperature",
  "physical_time_s": 60.0,
  "locations": [{ "association": "node", "node_id": 7 }]
}
```

Only succeeded registered v6 thermal reference jobs with passing native numerical
evidence are supported. The time must match an approved retained output exactly;
requests for unretained time, spatial/temporal interpolation, integration-point
temperature or other fields reject. The response retains K, m, science/execution
bindings and all three registered artifact hashes. Every native snapshot and
node is checked against its approved native output schedule, and every retained
JSON observation is matched to DAT before returning the selected 1–64 distinct
nodes. Float64 temperatures match DAT exactly. DAT's denser energy-balance
schedule does not authorize extra field-observation requests.

CalculiX serializes DAT times with seven significant digits. The existing thermal
verification permits only `1e-7 * duration_s` seconds of serialization
error. Reports expose both the approved `physical_time_s` and original
`native_time_s`, plus `time_serialization_tolerance_s`; this does not authorize
interpolation or an approximate request time.

`results compare-thermal` / MCP `results_compare_thermal` take version 1 with
explicit `left`/`right` thermal sampling requests. The exact mesh and ordered
node locations must match; both selected times and provenance remain in the
report. Signed `right-left` values compare those explicitly selected states,
including different retained times. They do not establish numerical acceptance,
component boot reliability or physical validation. Packaged `thermal-results-2`
passes eight field checks and 52 rejections; combined `moisture-results-1` passes
44 thermal/surface checks and 62 rejections through CLI and the real results MCP.
Exact package/report hashes are in [result evidence](evidence/thermal-results-cpu.json).

## Native surface moisture screening

`results moisture REQUEST.json` / results-profile MCP `results_moisture` derive
the minimum temperature across a complete registered planar-box surface at an
exact retained thermal observation. Requests contain `schema_version: 1`,
`job_id`, `physical_time_s`, `surface_region` (`xmin`, `xmax`, `ymin`, `ymax`,
`zmin`, `zmax`) and `moisture_risk`. Named surface IDs are recomputed from native
SI coordinates and matched to the complete registered semantic node set.
Surface temperature is derived from verified native data; caller-supplied
surface temperatures or ordinal face names reject.

`moisture_risk` explicitly selects one of:

- `{"assessment":"missing","reason":"humidity unavailable"}`;
- `{"assessment":"inapplicable","justification":"declared dry reference"}`;
- `{"assessment":"dew_point_screening","air_temperature":{"value":20,"unit":"degC"},"relative_humidity":0.5,"provenance":"explicit air input"}`.

The source report identifies the minimum native node and temperature; it retains
the mesh/field/DAT hashes, approved and observed time, and science/execution
bindings. Magnus screening uses air 0–50 °C and `0 < RH <= 1`; subzero surfaces
remain `unsupported_screening` pending a justified ice/frost model. Missing air
data remains missing; justified inapplicability remains explicit. This is a
synthetic planar-box screening result, with no condensate mass, moisture
transport, frost, ingress, sealing or physical-validation inference.

The packaged moisture gate checks all six complete surfaces for both retained
native thermal jobs and all three assessment branches. Original source trees,
jobs, events and artifact registrations remain unchanged.

## Conservative native temperature projection

`results transfer-temperature REQUEST.json` and results-profile MCP
`results_transfer_temperature` derive a uniform destination-block temperature
from a complete registered version-6 thermal field at an exact approved time.
The source DAT, JSON and native mesh are independently verified before mapping.
No caller temperatures, material properties, ordinal faces or interpolated times
are accepted.

```json
{
  "schema_version": 1,
  "source_job": "00000000-0000-0000-0000-000000000001",
  "physical_time_s": 120,
  "destination": {
    "region": "lower",
    "size_m": [0.02, 0.01, 0.01],
    "origin_m": [0, 0, 0]
  },
  "maximum_projection_error_k": 1,
  "maximum_relative_conservation_error": 1e-12
}
```

The destination is an explicitly translated congruent whole-box uniform model.
Both origin and dimensions enter its geometry identity; rotation/scaling and
unresolved translations reject. Complete native C3D8 cells must tile the approved
box with positive affine Jacobians. Each cell contributes `rho * cp * volume / 8`
to each native node's lumped thermal capacitance. The projection conserves the
constant-property capacitance-weighted temperature integral, using the typed
conservative map. A node-count average is insufficient at boundaries.

The report retains source science/execution/mesh/DAT/field identities and native
time, source material provenance, temperature range, destination capacitance,
the mapped Kelvin temperature, conservation receipt and maximum absolute
pointwise projection error. The caller explicitly approves that error (at most
1 K) and a conservation tolerance at most `1e-10`. Scientific source fields stay
unchanged. Projection is a separately quantified uniform-model approximation;
its receipt establishes neither a destination solve nor coupled-model validation.
