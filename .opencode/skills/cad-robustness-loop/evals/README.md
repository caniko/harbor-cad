# Workflow evaluations

**Not qualified.** Revision 13 scores **54/57**, versus baseline **55/57**.
The unchanged candidate regressions remain **45/48** in revisions 12 and 13.
Six artifact checks and six helper tests pass, but planning and read-boundary
failures prevent a clean sweep. No second sweep ran after the failed first.

- [Acceptance contract and capability map](COVERAGE.md)
- [Consolidated revision history](REVISION-HISTORY.md)
- [Current status](../../cad-robustness-loop-workspace/iteration-13/status.json),
  [claims audit](../../cad-robustness-loop-workspace/iteration-13/claims-audit.json),
  [static review](../../cad-robustness-loop-workspace/iteration-13/review.html)
- [Machine-readable coverage](../../cad-robustness-loop-workspace/coverage-results.json)
- [Next-session handoff](../../../../../intercom/handoff-cad-robustness.md)

## Run protocol

1. Freeze the candidate; pair it with the previous frozen skill. Preserve prompts,
   assertions, failures and independent grades. Do not repair scored answers.
2. Use nine regressions (48 assertions) and three independently authored
   post-freeze heldouts (nine assertions): 57 per arm, 24 fresh sessions total.
   Review heldout preparation before launch; retain rejected drafts.
3. Give each runner only its skill, exact `task.md`, own inputs, and authorized
   tools/docs. No grader, metadata or historical-answer access. Scope searches;
   a filename-only broad search is still a boundary violation. Stage each run in
   its own directory holding only that run's frozen `SKILL.md`, `task.md` and
   `outputs/`, so a repository-wide search cannot reach anyone else's files.
4. Continue `approved-dialog` for three real turns and `heldout-withdrawal` for
   two. Save each answer and prefix trace **before** sending the next user turn.
   Future turns stay hidden; pasted transcripts are not substitutes.
5. Keep traces, checker receipts, grades and timing **beside** `outputs/`.
   The three artifact cases write only their requested two files, including no
   scratch copies elsewhere. Missing timing/token metrics stay null.
6. Check exact artifacts and the entire visible trace, then grade every assertion
   and extra factual/process claim. Correct files alone do not prove adherence.
7. Generate the benchmark and static review with the installed skill-creator
   scripts. Keep case-mean rates separate from assertion-weighted fractions.
   Run a second full candidate sweep only after a genuinely clean first one.

## Local checks

From the Harbor-CAD checkout:

```sh
E=.opencode/skills/cad-robustness-loop/evals
TMPDIR=/data/scratch/tmp/opencode python3 -B -m unittest discover -s "$E" -p test_fixture.py
TMPDIR=/data/scratch/tmp/opencode python3 -B -m unittest discover -s "$E" -p test_gate_ledger.py
python3 -B .opencode/skills/cad-robustness-loop-workspace/iteration-13/verify-evidence.py
```

`test_gate_ledger.py` guards the instruction set, not model behavior: every gate
keeps its contiguous ID set (eight G0 checks, so a truncated gate fails), and the
supersession, one-ask, lowered-floor, capability-probe, separate-run-authorization,
claim-design, full-identity and stage-boundary rules stay stated. Dropping
G0.7/G0.8 fails three tests. Passing it does not show a sweep passes.

`HARBOR_CAD_TEST_CLI` selects the engine both the checker and `test_fixture.py`
use; there is no second hardcoded path. `bounded-objective` needs an engine build
that provides `design screen`, which committed harbor-cad source does not: it
probes `design screen --help` and the test skips, and the checker prints
`"passed": false` with the reason and exits 3. Record the CLI path and build
identity with any artifact receipt; a skipped case is unavailable, never a pass.

The evidence verifier checks 24 task reads, exact assertions/counts, four real
dialogue histories and identity manifests. Its current
`runner_read_boundary_clean: false` is a recorded failure, not verification error.

For a fresh fixture, use new `INPUTS`/`OUTPUTS` paths:

```sh
python3 -B "$E/fixture.py" scaffold CASE INPUTS
HARBOR_CAD_TEST_CLI=/path/to/harbor-cad python3 -B "$E/fixture.py" check CASE INPUTS OUTPUTS
TMPDIR=/data/scratch/tmp/opencode python3 -B "$E/capture-trace.py" SESSION_ID NEW_TRACE_JSON
```

`CASE` is `source-guard`, `contract-write` or `bounded-objective`. The checker
regenerates immutable fixture bytes; JSON is compared canonically and Markdown
byte-for-byte with the native producer. `capture-trace.py` refuses overwrite and
captures CLI stdout through an anonymous regular file before filtering. Direct
CLI-to-jq pipes truncated large
exports. Saved traces exclude reasoning, system messages and opaque metadata.

The six helper tests cover tampering/types, overwrite refusal, numeric rendering,
visible-history filtering, byte-derived receipt read-back, unusual filenames,
and lossless large-export capture. They do not qualify the model or product.
