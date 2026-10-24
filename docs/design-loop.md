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
- `templates`: named sets of 1–5 `{role, plan}` or chained
  `{role, plan_request, after?, max_artifact_bytes}` children.
- `metric_sources`: at most 32 `{metric, unit, source}`.
- `pareto_metric` (optional): a declared metric that replaces the minimum
  constraint margin as the second (maximized) Pareto axis.

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
JSON artifact. `{"$metric": METRIC[, "si": DIMENSION]}` becomes a metric
(in its declared unit) computed from the upstream roles' job metrics plus the
parameter, declared, sourced and provider metrics, at screen semantics (a
derived request sees only upstream results); unknown refuses the request.
Derivation binds only upstream (after-chain) jobs, so it is reproducible.
Validation substitutes dummy upstreams and checks the request schema, the
allowlist and the placeholders.

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
  registered, checksum-verified JSON artifact of a succeeded child. The role
  must exist in at least one fidelity; where it does not, the metric is
  unknown (e.g. a refine-only verification solve).
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

`max_supported_pcb_power_w` (W, `estimated`) is the swept PCB dissipation a
candidate supports at the hot point. With radiation off or linearized (its
lower bound is a constant film) and constant films and solar, the steady
model is linear, so nodal temperatures are affine in power.
Two solves at `pcb_power_low_w` and `pcb_power_high_w` give per-criterion
chords. The criteria are the PCB peak and the internal air against
`component_max_temperature_k` (components sit between them), and the housing
peak against the route material's conservative service maximum less
`housing_margin_required_k`. They are kept separate because each is
(near-)affine while their maximum switches branch. A peak is a maximum over
nodes and therefore convex, so the chord is an upper bound inside the swept
interval, which makes the result conservative. The result is the smallest
power at which a chord reaches its limit, clamped to the high power. It is
unknown when the low power already exceeds a limit or a temperature does not
rise with power. Metric names are unprefixed for the low solve, `hi_` for the
high one and `check_` for verification. On the refine tier, a third solve at
that power (planned through `$metric`) must match the chords within 0.5 K
(`AFFINITY_TOLERANCE_K`), otherwise the metric is unknown. Providers see the
candidate's fidelity through `MetricProvider::evaluate_at`.

A missing metric (failed, cancelled or interrupted child, absent artifact,
provider error) is unknown. Unknown constraints never pass.

## Proposer

Deterministic and seeded; no surrogate model. For each categorical
combination it screens `2d+1` Latin-hypercube points snapped to the grid
(fewer when combinations would exceed the budget). It then runs multi-start
pattern searches from the screening points, ranked feasibility-first (unknown
points never start one). Up to three starts split the remaining screen budget
in cumulative equal quotas, but only as many as can each afford a convergence
check: one compass poll per level down to the step floor plus the floor's
diagonals, `2d(levels + d − 1)` polls. A start that converges early passes its
rest on, and further ranked points start once the first ones converge. Each
search is an opportunistic compass poll. Steps start at an eighth of the range
(about half a screening cell), are kept after an improvement and halved after a
failed poll. At the step floor the poll order is compass, edge-following, then
pairwise diagonals. The edge step follows an active constraint. For an
infeasible unit poll `c ± e_i`, it takes an axis `j` whose unit poll raised the
minimum margin and walks the line `c ± e_i + t·e_j` by a secant on the known
margins (regula falsi once a feasible point brackets it). It proposes the first
grid point predicted feasible. Remaining budget then refines the top-k screened
candidates.

Every proposal is a pure function of the envelope and the stored candidates.
Each search is replayed over the evaluated points, and each evaluated point is
charged to the first search that visits it. A restarted worker therefore
regenerates the same sequence, and no point is proposed twice
(`proposals_regenerate_from_persisted_history_after_restart`). A design whose
searches do not all converge ends `budget_exhausted` rather than `completed`.

Ranking is feasibility-first: feasible candidates by objective, infeasible by
total normalized violation, unknown last. The incumbent is the best feasible
refined candidate, else the best feasible screened one.

Measured on the CI example (optimum on a curved `mean_velocity ≥ 0.1`
boundary), seeds 0–39, 32 evaluations, gap of the incumbent to the
brute-force grid optimum
(`constrained_example_seed_sweep_beats_the_single_start_baseline`):

| proposer | worst gap | median gap | exact optimum |
| --- | --- | --- | --- |
| single-start compass (trunk `c8a66b5`) | 27.7 % | 5.3 % | 6/40 |
| multi-start + edge-following | 11.7 % | 0 % | 34/40 |

Most of the gain comes from the edge step and the finer first level. On this
2-D example only one start fits a convergence check, so later starts run only
after the first search converges. Forcing three equal splits trims the worst
gap (9.6 %) but loses the rest: median 2.1 %, 5/40 exact, and the bowl no longer
converges, because each start stops before the step floor. All six misses run
out of budget while still following the boundary.
The unconstrained bowl still converges exactly for all 24 seeds. Swap for a
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
set, a two-objective Pareto set and budget use. The Pareto set is objective
vs minimum constraint margin, or, with `pareto_metric`, objective vs that
metric (maximized) over feasible candidates. `design export` repeats it as
`{index, fidelity, objective, value}` points. `design export` reports the
winner, its child job IDs and labels each constraint `simulated`, `calibrated`/`estimated` (provider),
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
route × finish pair. The screen chain is `variant` (`plan_cad_variant` of the
enclosure fixture's `Params` sheet, with all five thermal regions) → `mesh`
(`plan_cad_solid_mesh`, a conformal tetrahedral mesh of `gasket`, `housing`,
`lid`, `pcb` and `window`, 6 mm) → `thermal_lo` and `thermal_hi`
(`plan_enclosure_thermal` at 1 W and 15 W in linear `power_sweep` mode with
`radiation: "linearized_lower_bound"`; see
[enclosure thermal](enclosure-thermal.md)). The refine tier
(`refine_top_k` 1) remeshes at 4 mm, chains `thermal_lo` → `thermal_hi` →
`thermal_check`, and runs the check at the superposed supported power. The
objective minimizes `unit_cost_high_usd` at a declared quantity of 50. The
constraints are `temperature_margin_k ≥ 10 K` (housing at 1 W and the class
4.1 cold point), `manufacturability_violations ≤ 0` and
`max_supported_pcb_power_w ≥ 1 W`. `pareto_metric` reports cost vs supported
power. PCB power is no longer an operator input. The swept range (1–15 W) and
the 10 K housing margin are declared requirements. The component limit
(`electronics:component-limit-industrial.low`, 85 °C) and the orthotropic
board conductivity (`electronics:pcb-fr4-copper-in-plane.low`, 8.1 W/(m·K),
and `electronics:pcb-fr4-copper-through-plane.low`, 0.26 W/(m·K)) are cited
conservative defaults, not product data (see
[design conditions](design-conditions.md)). The minimum wall is checked
against the conservative end of the process's minimum-wall range.

Ambient, solar irradiance and the exterior film are `sourced` metrics. They
come from the ETSI EN 300 019-1-4 class 4.1 hot design point (see
[design conditions](design-conditions.md)). A metric with
`{"kind": "sourced", "key": "condition:<class>.<hot|cold>.<field>"}`,
`"finish:<id>.<field>"`, `"film:<id>.<low|high>"` or
`"electronics:<id>.<low|high>"` carries a cited value in its SI unit.
Templates read it through `$input`, like a declared input. The internal cavity
film is the sourced `film:enclosed-cavity-natural.low` (1.2 W/(m²·K)). The
pc-uv window has no sourced absorptance, so its solar load uses the explicit
conservative bound `transparent_solar: "deposit_incident_in_cavity"` (all
incident irradiance enters the internal air node); `design export` lists it, and the
linearized radiation bound, under `conservative_bounds` and prefixes the
statement with `CONSERVATIVE BOUND`.

`python/tests/test_enclosure_feasibility.py` solves the
`al-6061-t6__cnc-3axis-al__powder-coat-white` candidate on a structured
analogue of the fixture with the pure-Python reference solver (not CalculiX),
using only cited values. With radiation off or linearized, the midpoint
solve matches the chords within 0.1 K, and the air node matches exactly. With
T⁴ radiation the air deviates by 0.16 K, which is why `power_sweep` refuses
it. Findings:

- With the window and both lower-bound credits (pilot), the candidate supports
  10.42 W (opaque lid 11.10 W), bound by the PCB peak; the verification solve
  matches the chords within 0.04 K. The ablation is in
  [enclosure thermal](enclosure-thermal.md).
- Without the credits (no radiation, isotropic 0.26 W/(m·K) board), the
  window case supports no swept power: 373.6 K internal air and a 358.8 K PCB
  peak at 1 W, above the 358.15 K limit; the opaque lid supports 2.04 W.
- Native FreeCAD/Gmsh/CalculiX runtime qualification waits on the security
  hold.
- PA12 stays a candidate but can never pass. Its sourced minimum service
  temperature is -40 °C, and the class 4.1 cold point is -33 °C, so the cold
  margin is 7 K, below the 10 K requirement, whatever the thermal result.

`examples/design-enclosure-demo.json` runs the earlier single-solve chain
(fixed PCB power, radiation on) with explicitly
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
