# Bounded design loop

A design envelope turns one human approval into at most 32 ordinary immutable
jobs. The loop proposes parameter candidates, materializes each candidate into
existing execution plans, waits for those jobs, extracts declared scalar
metrics and ranks the candidates. It never schedules or executes anything
itself: every child is submitted through the worker's normal `submit` path
(binding, staging, authority and shared admission unchanged) and keeps normal
logs, cancellation, qualification and export.

```sh
harbor-cad design init > envelope.json          # CI-runnable example, known optimum
harbor-cad design validate envelope.json        # strict substitution; no worker
harbor-cad design plan envelope.json            # {approval_digest, plan, prepared}
harbor-cad design submit envelope.json --approve DIGEST --idempotency-key KEY
harbor-cad design propose DESIGN_ID candidate.json
harbor-cad design status DESIGN_ID
harbor-cad design export DESIGN_ID
```

MCP profiles `simulation` and `all` expose `design_plan`, `design_submit`,
`design_propose`, `design_status` and `design_export` over the same operations.

## Approval model

The approval digest is the SHA-256 of the canonical envelope JSON, computed
exactly like a plan approval. The worker additionally binds its effective
policy and host profile. Before any intent is written, the lowest-corner plan
of every template set passes the ordinary submission checks. A durable SQLite
intent (`designs`) binds the envelope and profile digests; each candidate row
(`design_candidates`) is written before its children are submitted. Child
idempotency keys derive from the design UUID, envelope digest, candidate index
and template role, so a crash or restart resubmits nothing twice. The same key
with a changed envelope or profile is an idempotency conflict.

## Envelope (schema 1)

- `space.continuous`: at most 8 `{name, low, high, step, unit}` on an integral
  step grid. `step` is the manufacturing grid and the search step floor.
- `space.categorical`: at most 2 `{name, choices}` with 1–6 token choices.
- `objective`: `minimize` a weighted sum of declared metrics.
- `constraints`: `{metric, op: le|ge, limit, unit, safety_factor?}`; units
  must equal the metric's declared unit and safety factors (≥ 1) tighten.
- `budget`: `max_evaluations` ≤ 32, `max_wall_seconds`, and
  `max_artifact_bytes`, which must cover every evaluation's unchanged child
  output allowance.
- `fidelity`: the `screen` template set, plus an optional `refine` set with
  `refine_top_k` ≤ 3 re-evaluations of the best screened candidates.
- `templates`: named sets of 1–4 `{role, plan}` or chained
  `{role, plan_request, after?, max_artifact_bytes}` children.
- `metric_sources`: at most 24 `{metric, unit, source}`.

## Templates

A template is a complete existing plan JSON. Any object
`{"$param": NAME}` or `{"$param": NAME, "unit": UNIT}` is replaced: continuous
parameters become `{"value": v, "unit": param.unit}` Quantities, categorical
parameters become the chosen string. Substitution is strict: unknown
placeholders, extra placeholder keys and unit mismatches reject, and every
parameter must be used by a template or a metric source. Each resolved plan
must pass the unchanged `ExecutionPlan` validation under the effective policy;
a wrong physical dimension is rejected by the plan's own unit conversion.

`{"$param": NAME, "si": DIMENSION}` emits the bare SI number instead, for plan
fields without a unit (`convection_w_m2_k`, `temperatures_k`); a wrong dimension
rejects. `{"$input": METRIC[, "si": DIMENSION]}` takes a `declared` metric
source's value. A missing declared input refuses the plan that needs it; it is
never defaulted.

## Chained templates

A chained child is a planning request instead of a plan:

```json
{"role": "mesh", "after": "variant", "max_artifact_bytes": 67108864,
 "plan_request": {"operation": "plan_cad_mesh",
   "request": {"source_job": {"$job": "variant"}, "region_name": "housing",
               "resolution": 8, "geometry_tolerance_m": 1e-5}}}
```

`plan_request` is a worker request whose `operation` is on the allowlist
`design::CHAIN_OPERATIONS`: `plan`, `plan_thermal_reference`,
`plan_cad_variant`, `plan_cad_mesh`, `plan_fem_imported`, `plan_cad_solid_mesh`
and `plan_enclosure_thermal`. Other operations
are refused when the envelope is validated. `after` names an earlier role in
the same set; without it the request is planned as soon as the candidate is
created. `{"$job": ROLE}` becomes the job id of an upstream role (`after` or its
own upstream chain), and `{"$job": ROLE, "artifact": A, "pointer": P[, "unit": U]}`
becomes a finite number read from that job's registered, checksum-verified
JSON artifact. Validation substitutes dummy upstreams and checks the request
schema, the allowlist and the placeholders.

When `after` has succeeded, the worker's design advance resolves the request
and calls the same `dispatch` that the CLI and MCP planning calls use. Planning
logic is not duplicated, and authority requirements are unchanged. The derived
plan must pass the unchanged `ExecutionPlan::validate()` under the effective
policy and stay within the template's approved `max_artifact_bytes`. It is
then submitted through the ordinary `Submit` path, with `submission_binding`,
staging and admission all unchanged. The envelope digest approves the request.
The concrete plan is bound by an idempotency key over (design, envelope
digest, candidate index, role, upstream job id), and the candidate row records
its plan digest and upstream job before it is submitted. After a restart the
plan is derived again by the same key and must match the recorded digest.

If the upstream job fails, is cancelled or is interrupted, the dependents are
never submitted. The same applies when the request is refused (for example a
missing declared input, a geometry outside an operation's allowlist, or an
oversized allowance). The child is recorded as `blocked` with the reason, and
its metrics stay unknown. A host-capability refusal (`unqualified`, such as a
worker without authority for native planning) fails the design. The CI-runnable
example of a chain is two analytical `plan` stages, where the second reads the
first job's `validation.json` (`tests/design.rs`).

## Metrics

- `job {role, artifact, pointer}`: a finite number at a JSON pointer inside a
  registered, checksum-verified JSON artifact of a succeeded child.
- `parameter {param}`: the candidate's own continuous value.
- `provider {name, params}`: a local `MetricProvider` (see
  `design::PROVIDERS`) that receives only the declared parameters and the
  extracted job, parameter and declared metrics.
- `declared {input}`: an explicit `PhysicalInput` (`known` with provenance, or
  `missing` with a reason). A missing input is unknown.
- `correlation {role, request}`: the pooled RMSE (`correlation::objective`, K)
  of `results correlate` against the role's succeeded thermal job. `request`
  is a correlation request without `job_id`, and its CSV resolves under the
  bound profile's input root.

### Manufacturing providers

`src/design_providers.rs` registers estimate-only providers backed by
`manufacturing.rs` (evidence `estimated`): `unit_cost_low_usd`,
`unit_cost_high_usd` (USD), `mass_kg` (high end of the density range),
`manufacturability_violations` (count of `violation` and `marginal`
findings; any `unknown` check makes the count unknown, never 0),
`temperature_margin_k` (conservative service limit minus `max_temperature_k`,
and the tighter of that and the minimum-temperature margin when
`min_temperature_k` is known) and `stress_margin` (from `max_von_mises_pa` and
a declared `stress_safety_factor`). Material and process come from a
categorical `route` choice `MATERIAL__PROCESS`, or from separate `material`
and `process` choices. A triple `MATERIAL__PROCESS__FINISH` is parsed once; its
finish only affects the thermal template. Geometry comes from the metrics `volume_m3`,
`xmin_m`…`zmax_m` (the variant receipt's `after.<region>.bounds_m`) and
optionally `min_wall_m`. Quantity comes from a `quantity` metric (a parameter
or a declared input). Any missing input, missing sourced data or incompatible
route gives an unknown value.

A missing metric (failed, cancelled or interrupted child, absent artifact,
provider error) is unknown. Unknown constraints never pass.

## Proposer

Deterministic and seeded; no surrogate model. For each categorical
combination it screens `2d+1` Latin-hypercube points snapped to the grid
(fewer when combinations would exceed the budget). It then runs a compass
search around each combination's incumbent, best combination first: steps
start at a quarter of the range, are kept after an improvement and halved
after a failed poll down to the step floor, where pairwise diagonals are also
polled. Remaining budget then refines the top-k screened candidates.

Ranking is feasibility-first: feasible candidates by objective, infeasible by
total normalized violation, unknown last. The incumbent is the best feasible
refined candidate, else the best feasible screened one. Pattern search can stop
short on a diagonal constraint boundary within 32 evaluations; swap for a
surrogate if the budget binds.

External candidates (`design propose`) must assign every parameter on its grid
within bounds and carry provenance. They consume the same budget and are
evaluated identically; resubmitting an already screened point returns it.

## Loop driver and limits

The worker advances running designs after each reconcile: it resubmits any
missing child by key, extracts metrics when all children are terminal and
proposes the next candidate only when nothing is pending. A design ends as
`completed`, `budget_exhausted`, `wall_time_exhausted`,
`artifact_budget_exhausted` or `failed` (non-transient error, recorded).
`design status` reports iterations, metrics, feasibility, incumbent, feasible
set, a two-objective Pareto set (objective vs minimum constraint margin) and
budget use. `design export` reports the winner, its child job IDs and labels
each constraint `simulated`, `calibrated`/`estimated` (provider),
`prescribed` (parameter or declared input), `measured` (correlation) or
`unknown`. When the objective minimizes a correlation metric, the winner also
carries `calibration` entries labelled `calibrated`, with the approved
measurement path and test provenance.

## Enclosure and calibration examples

`examples/design-enclosure.json` covers the outdoor enclosure pilot. Its
continuous parameters are wall, fin count, fin height, length and height on
manufacturing grids. Its single categorical `route` lists only compatible
`MATERIAL__PROCESS__FINISH` triples (process and finish `compatible_materials`
in `data/processes.json` and `data/finishes.json`; `tests/design.rs` checks
each): white and RAL 7035 powder coat on CNC 6061-T6 and on laser/brake
5052-H32, and black-pigment MJF PA12. No evaluation is spent on a refused
route × finish pair. The chain is `variant`
(`plan_cad_variant` of the enclosure fixture's `Params` sheet, with all five
thermal regions) → `mesh` (`plan_cad_solid_mesh`, a conformal tetrahedral mesh of
`gasket`, `housing`, `lid`, `pcb` and `window`) → `thermal`
(`plan_enclosure_thermal`, steady conduction; see
[enclosure thermal](enclosure-thermal.md)). The objective minimizes
`unit_cost_high_usd` at a declared quantity of 50. The constraints are
`temperature_margin_k ≥ 10 K`, `manufacturability_violations ≤ 0` and
`component_temperature_margin_k ≥ 0 K`. The temperature margin compares
`max_temperature_k`, read from the thermal receipt (`/regions/housing/max_k`),
and the class 4.1 cold-point ambient (`min_temperature_k`) with the route
material's conservative service limits. The component margin is the operator's
`component_max_temperature_k` minus the simulated `pcb_max_temperature_k`. The
minimum wall is checked against the conservative end of the process's
minimum-wall range.

Ambient, solar irradiance and the exterior film are `sourced` metrics. They
come from the ETSI EN 300 019-1-4 class 4.1 hot design point (see
[design conditions](design-conditions.md)). A metric with
`{"kind": "sourced", "key": "condition:<class>.<hot|cold>.<field>"}` or
`"finish:<id>.<field>"` carries a cited value in its SI unit. Templates read it
through `$input`, like a declared input. The internal cavity film is the
sourced `film:enclosed-cavity-natural.low` (1.2 W/(m²·K), see
[design conditions](design-conditions.md)). The pc-uv window has no sourced
absorptance, so its solar load uses the explicit conservative bound
`transparent_solar: "deposit_incident_in_cavity"` (all incident irradiance
enters the internal air node); `design export` lists it under
`conservative_bounds` and prefixes the statement with `CONSERVATIVE BOUND`.
PCB power, board conductivity and the component limit are product-specific and
are declared `missing` ("product-specific: supplied by operator").

Nothing structural blocks feasibility once those three inputs are supplied:
`python/tests/test_enclosure_feasibility.py` solves the
`al-6061-t6__cnc-3axis-al__powder-coat-white` candidate on a structured analogue
of the fixture with the pure-Python reference solver (not CalculiX), with
synthetic stand-ins for only the operator inputs, and finds both thermal
margins and the Rust manufacturability screen satisfied. What remains:

- Native FreeCAD/Gmsh/CalculiX runtime qualification waits on the security
  hold.
- Until the operator supplies the three inputs, the thermal plan refuses, the
  child is `blocked` and the thermal constraints stay `unknown`.
- PA12 stays a candidate but can never pass. Its sourced minimum service
  temperature is -40 °C, and the class 4.1 cold point is -33 °C, so the cold
  margin is 7 K, below the 10 K requirement, whatever the thermal result.

`examples/design-enclosure-demo.json` runs the same chain with explicitly
`synthetic: true` inputs and a declared window. `design status` and
`design export` list those inputs in `synthetic_inputs`, and the export
statement starts with `SYNTHETIC`.

Calibration needs no extra command. `examples/design-calibrate.json` is an
envelope whose parameters are the uncertain inputs (film coefficient and
conductivity of the synthetic plane-wall thermal reference, planned through
`plan_thermal_reference` with `si` placeholders). Its objective is a
`correlation` metric against a measured CSV. Replace the spec, probe map, CSV
and test provenance with the real test article. The calibrated values hold only
for that rig, test and sensor placement.

Process success is not physical validation. Simulated constraints inherit
only their child jobs' numerical scope. The `design init` envelope is a
synthetic analytical channel, not an enclosure model.
