# Workflow coverage contract

This contract covers the **agent workflow**, not arbitrary CAD correctness or
product certification. Mapping a responsibility is not evidence that it passes.
Current acceptance is **not closed**; see the [runbook](README.md),
[revision history](REVISION-HISTORY.md) and
[coverage manifest](../../cad-robustness-loop-workspace/coverage-results.json).

## Completion rule

1. Map every responsibility below to explicit assertions and retained evidence.
2. Run all nine regressions against the revised and previous frozen skills.
3. Add three independent post-freeze heldouts. Tuning against them requires a
   new revision and fresh heldouts; validate preparation before any runner launch.
4. Require **two consecutive clean full candidate sweeps**, with no mapped or
   safety-critical claim/process failures. Preserve failures; do not weaken
   assertions, lower thresholds or sample unchanged runs until one passes.
5. Verify actual artifacts, complete visible traces, genuine multi-turn order,
   routing and scoped native references. Missing/provider-failed runs are not passes.

Each sweep has finite turn/time/resource limits and immutable small inputs.
Name an unavailable gate and its prerequisite instead of broadening authority.

## Capability map

| ID | Responsibility | Required evidence |
|---|---|---|
| C1 | Preserve dirty work/HEAD; retained assembly wins over old spec | Repository baseline and scoped hashes; source-guard conflict/sentinel |
| C2 | Source hashes, LFS refusal, format-compatible audit, static/parametric scope, overwrite refusal | source-guard checker **and trace**; native archive boundaries |
| C3 | Revision-bound keepouts; simplified seed is not the product | source-guard, heldout-source; pinned seed regression |
| C4 | Separate approved/open/physical-input/qualification states | contract-write JSON; heldout-source and heldout-withdrawal |
| C5 | One unresolved question; retain/correct/stop; recovery target vs capability; conditional cold modes | Actual approved-dialog and withdrawal turns; missing-mission; no reconfirmation |
| C6 | Native recompute, valid/non-overlapping solids, keepouts, seal support, coupled grid | Pinned seed/native receipts; complete proposed G0 checks |
| C7 | Diagnose before redesign; preserve metric meaning, units, datum and sampling limits | failed-stock assertions; sampled-stock regression |
| C8 | Matching mating parts; both-side seating/support/stops; unknown compression/tolerance | failed-stock; native gasket projection/support, not sealing proof |
| C9 | Negated weighted weakest normalized margin; ties, hard gates, unknown/nonfinite/overflow; unique finite selection | bounded-objective; shared Rust scorer regressions |
| C10 | 32-total evaluation cap; finite resources; fresh identity/preflight; backend/budget refusal | approval-stop, heldout-approval, bounded-objective; rejection of 33 evaluations |
| C11 | Equal material requirements; unknown exposure/ageing; body vs installed clearances | materials-lifecycle; contract-write; approved-dialog |
| C12 | Separate schema/native/reference/worker/product/lab gates; exact receipts; affected-only rechecks | changed-contract, heldout-approval; actual hashes and historical receipt preservation |
| C13 | Brief handoff; runnable regression for changed logic | source-guard summary; fixture checker/helper tests |
| X1 | Actual tool behavior and authorized I/O throughout | Full traces plus immutable artifacts; scratch/browser and hidden-material access audits |
| X2 | Real multi-turn state, not a pasted transcript | Same-session continuations, prefix captures and full histories |
| X3 | Appropriate discovery, not forced skill loading | Eight historical routing traces: four positive/four negative |

## Evidence boundaries

- Revision 13: six exact fixture checks and six helper tests pass; 24 task reads,
  four dialogue histories and source/product/heldout hashes verify. Its broad
  source-runner grep crossed the read boundary, so it is **not clean isolated
  behavioral evidence**, even though only filenames appeared in that result.
- Native library/selected integration evidence: **103 passed, zero ignored** in
  [iteration 11](../../cad-robustness-loop-workspace/iteration-11/native-evidence.json).
  Revisions 12/13 reuse it at unchanged source/binary identities; it is not fresh
  execution or pinned Nix/package/native-worker qualification.
- Earlier separate probes record 16 seed/native-stdlib checks, one focused scorer
  check, four direct CalculiX references at two refinements each, and CLI rejection
  of 33 evaluations. Their identities/scopes remain in the coverage manifest and
  historical receipts; none proves complete product equivalence.
- Routing tests cover selection, not answer quality. Scores across changed
  rubrics/heldouts are not an identical test; case means are not assertion fractions.

## External gates remain open

No workflow score establishes packaged-worker isolation/capability, measured
power/material/contact laws, machining tolerances, installed fit, aged-battery or
assembled thermal behavior, IP66/IK10, environmental durability or lifetime.
Those need separately authorized, exact-source worker/product/lab evidence.
Unknown, failed, omitted and inapplicable results cannot establish release.
