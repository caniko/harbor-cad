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
- `templates`: named sets of 1–4 `{role, plan}` children.
- `metric_sources`: at most 8 `{metric, unit, source}`.

## Templates

A template is a complete existing plan JSON. Any object
`{"$param": NAME}` or `{"$param": NAME, "unit": UNIT}` is replaced: continuous
parameters become `{"value": v, "unit": param.unit}` Quantities, categorical
parameters become the chosen string. Substitution is strict: unknown
placeholders, extra placeholder keys and unit mismatches reject, and every
parameter must be used by a template or a metric source. Each resolved plan
must pass the unchanged `ExecutionPlan` validation under the effective policy;
a wrong physical dimension is rejected by the plan's own unit conversion.

Children of one candidate are independent jobs. Chains whose later plan is
derived from an earlier child's registered output (CAD variant → mesh → FEM)
need a planning-request template kind and are not yet supported.

## Metrics

- `job {role, artifact, pointer}`: a finite number at a JSON pointer inside a
  registered, checksum-verified JSON artifact of a succeeded child.
- `parameter {param}`: the candidate's own continuous value.
- `provider {name, params}`: a local `MetricProvider` (see
  `design::PROVIDERS`) that receives only the declared parameters and the
  extracted job/parameter metrics.

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
`prescribed` (parameter) or `unknown`.

Process success is not physical validation. Simulated constraints inherit
only their child jobs' numerical scope; the example envelope is a synthetic
analytical channel, not an enclosure model.
