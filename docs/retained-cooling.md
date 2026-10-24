# Retained-distribution cooling: native formulation contract

This continuation implements the retained-water recipe in
[the implementation specification](implementation-spec.md#5-engineering-recipes-and-boundaries)
after the independently qualified wetting, conservative extrusion and Stefan
reference slices. Its acceptance is distinct from those prerequisites.

## Supported synthetic model

The initial model is stationary fixed-volume, equal-density/equal-heat-capacity/
equal-conductivity conduction. Every original active wetting nodal control volume
retains its water amount `f = 1 - phi`, original position, source identity and
explicit extrusion. The background carries the same sensible thermal properties
and no latent heat; the water contribution carries explicitly supplied latent
heat. Native cell enthalpy is
`cp * (T - Tcold) + f * L * liquid_fraction` in J/kg of the complete control.
Water mass is `rho * volume * f`; it is conserved independently of total
background-plus-water thermal mass. These equal properties are synthetic inputs,
not water/air substitutions or physical qualification.

Require the complete recorded native source and exact retained observation.
Original water fractions must lie in `[0,1]` without clipping or thresholding.
Stationary conduction requires exactly zero original velocity. Moving states
and phase overshoots remain available as retained evidence and refuse this
native initialization. No wetting result supplies temperature: initial liquid
water at an explicitly prescribed melting temperature, cold-boundary temperature,
material domain and provenance are independent approved inputs. Moisture-risk
inputs remain explicit.

The lower y wall is an explicit cold Dirichlet boundary, the upper wall insulated
and x periodic. Walls carry no transferred mass. The original active controls,
including all original x points, define the domain. Native geometry may neither
discard an endpoint nor silently substitute the earlier Stefan strip geometry.

## Native source and thin driver

Use the existing immutable OpenLB `145cd54810b468f4b6fd3ed86b10644264841578`,
Float64 CPU, D2Q5 total-enthalpy collision/streaming and native boundaries.
Its `dynamics::ParameterFromCell` supplies per-control latent heat `f * L` to
`TotalEnthalpyAdvectionDiffusionTRTdynamics` with explicit native `MAGIC = 0.25`;
a narrow parameter-routing coupling
invokes the original `TotalEnthalpyPhaseChangeCoupling` with the same per-cell
latent value. The original solver owns phase/temperature evolution. Retain
upstream source links from [the Stefan reference](freezing-reference.md).

Invoke the pinned `BouzidiAdeDirichletPostProcessor` and `BouzidiPostProcessor`
at the lower and upper half links, respectively. These native calls own both
boundary replacements; the driver records their actual population energy
changes. Require distinct boundary controls. Full-way ghost reflection stores
undeclared boundary energy on a nonuniform source and fails the independent
insulated-wall gate. Earlier BGK temporal refinements also overheat original
controls and remain refused without clipping. Neither diagnostic is promoted.

No flow evolution, expansion, pressure, deformation, fracture or lifetime is
inferred. Native phase/temperature, initial state, original source phase/velocity
and per-step population-boundary energy exchange must remain closed authoritative
artifacts with units and independent source/execution identities.

## Acceptance and delivery order

1. Validate explicit thermal inputs and independently reconstruct conservative
   source-to-initial-enthalpy mapping. Refuse overshoots, moving states, missing
   temperatures/provenance and weakened gates.
2. Verify a thin native driver against the uniform-phase Stefan reference and
   nonuniform original retained distributions. Independently reconstruct original
   phase/enthalpy relations, mass and native population-exchange energy at every
   retained time; conservation gates remain at most `1e-10`.
3. Assess equal-physical-time spatial and temporal refinement independently, at
   at-most-`0.02` normalized errors. Any subcell refinement must explicitly copy
   the complete parent phase into congruent subcontrols and conserve parent mass
   and initial enthalpy; it cannot re-evaluate or smooth the original wetting
   solution. A numerical replay cannot qualify native execution.
4. Route the independent immutable cooling approval through the common worker,
   transactionally retain distinct-inode originals before acknowledgment, and
   qualify exact packaged native execution, CLI/MCP parity, results, exports,
   restart/idempotency, cancellation/death and root/reservation release.

Use existing Rust/Serde/Schemars contracts, Rust integration tests, Pytest native
reconstruction tests and operation-only Nix packages. Verify each slice with
`cargo test --locked`, `cargo clippy --locked --all-targets -- -D warnings`,
`cargo build --locked`, focused `uv run --locked --project python pytest`, and the complete
`python3 -B scripts/check_cpu.py` gate before its local commit. Package/campaign
execution uses the existing guarded Canix lease and immutable source capture.
This document defines acceptance; it records no executed cooling capability.

## Source-bound initialization preparation

`harbor-cad results prepare-retained-cooling examples/retained-cooling.json` and
results/all MCP `results_prepare_retained_cooling` share the Rust preparation
contract. Replace the example source ID with a succeeded registered wetting job
and select an exact retained time. The request carries explicit original thermal
quantities and provenance; it accepts no caller-supplied phase, velocity or
source temperature. The result retains complete source/receipt/execution
identities and independently checks conserved water mass and the sum of sensible
and latent initial energy. Its `executed` field is false: preparation supplies
inputs for a separately approved native cooling solve.

## Independent native reference packages

`retained-cooling-native-cpu`, `retained-cooling-reference-cpu` and
`runtime-retained-cooling-reference-cpu` use the same pinned OpenLB and isolated
Python 3.13. The operation-only sandbox binds the original CSV read-only. Its
strict envelope binds original byte count/checksum, the explicit native recipe
and a conservation tolerance no greater than `1e-10`. The adapter preserves the
unmodified native receipt, every original thermal control and every per-step
boundary exchange before publishing independent original-field checks. No
import of native libraries occurs in the MCP interpreter.

Parent subdivision factors `1/2/3/4` copy complete original phase into congruent
controls without smoothing or changing retained mass. Qualify complete retained
histories, including partially frozen states: a fully frozen final phase alone
does not establish convergence. The coarse original grid remains unqualified
at the observed `0.02316` intermediate enthalpy error. Fine uniform diagnostic
cases have maximum normalized Stefan temperature errors `0.00970` and `0.00818`.
Full original-history fine spatial and temporal qualification, exact packaged
execution and independent worker approvals are separate gates.

## Shared-worker execution contract

`results plan-retained-cooling REQUEST` and MCP `retained_cooling_plan` resolve
one succeeded registered native wetting job and independently approve a v18
CPU cooling/bundle DAG. A `CoolingExecutionRequest` contains `schema_version: 1`,
the complete existing `initialization` request, `spatial_refinement: 1|2|3|4`,
`integration_substeps: 1|2|4`, bounded `base_steps` and complete ordered
`observation_base_steps` including zero and the final step. Native steps are
base steps multiplied by `q² * integration_substeps`; original-spacing thermal
physical times remain identical across separately approved refinements. Every
approval includes complete wetting CSV history, original/verified receipts,
source request, execution binding and independent authorization identities.

Ordinary `job submit PLAN --approve DIGEST --idempotency-key KEY` executes
through the same authority/admission/systemd worker. Submission transactionally
copies distinct, checksummed original inodes before acknowledgment. The native
sandbox sees only its operation closure and read-only original/request files.
Complete original subcontrols and per-step boundary energy are reconstructed
again in Rust before acceptance, and originals remain available after failure.
Unverifiable unpublished staging artifacts remain preserved during recovery.

`results sample-retained-cooling REQUEST` and MCP
`results_sample_retained_cooling` return exact original native subcontrols for
one `job_id`, destination `region`, approved `physical_time_s` and 1–64 distinct
`points: [[i,j], ...]`. The response retains original parent identities, SI
positions, water fractions, Float64 enthalpy/temperature/liquid fractions,
complete history summaries and source/approval provenance. Every exported
cooling CSV retains its physical time, units and original-control association.
Queries refuse interpolation, extrapolation, changed registered bytes and
unsupported regions. Execution/conservation evidence does not establish
convergence or physical applicability. Exact-worker lifecycle qualification is
required separately from the completed `c8-30` isolated package campaign.

The worker also publishes lossless VTK XML `.vts` views of every closed native
CSV. StructuredGrid points preserve all original Float64 coordinates and fields,
child/parent IDs and x-fastest topology. PointData represents original nodal
controls; CellData is empty and carries no invented interpolation. FieldData
retains the approved physical time, explicit extrusion and subcontrol volume.
The z coordinate is the original source plane at the start of that extrusion.
Historical qualification reconstructs each published view from its authoritative
CSV and requires the entire declared view history. Earlier archived v18 jobs
without these views retain their original CSV evidence.
