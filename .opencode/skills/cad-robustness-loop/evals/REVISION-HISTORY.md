# Revision history — consolidated

**No qualified revision.** Latest: 13, candidate **54/57**, baseline **55/57**.
The unchanged candidate regressions score **45/48 in both 12 and 13**; different
heldouts explain the full-score change. Two clean full sweeps remain absent.

Run links below retain original answers, independent grades, corrections,
snapshots, traces and failures. No raw evidence was removed by consolidation.

| Run | Candidate | Baseline | Scope / outcome |
|---|---:|---:|---|
| [1](../../cad-robustness-loop-workspace/iteration-1/) | 9/9 | 6/9 | Three-case smoke test; no-skill baseline, not full qualification |
| [2](../../cad-robustness-loop-workspace/iteration-2/) | 12/12 | 9/12 | Strengthened three-case rubric; retained untested boundaries |
| [3](../../cad-robustness-loop-workspace/iteration-3/) | — | — | Incomplete/failed: unsupported budgets and cold/service assumptions |
| [4](../../cad-robustness-loop-workspace/iteration-4/) | — | — | Incomplete/failed: arithmetic, corner ledger and invented inputs |
| [5](../../cad-robustness-loop-workspace/iteration-5/) | 49/53 | 51/53 | Full sweep failed dialogue, parser use, independent work and installed survey |
| [6](../../cad-robustness-loop-workspace/iteration-6/) | 53/54 | 48/54 | Failed numeric reporting |
| [7](../../cad-robustness-loop-workspace/iteration-7/) | 52/54 | — | Same-skill verification failed diagnosis/check completeness |
| [8](../../cad-robustness-loop-workspace/iteration-8/) | 50/54 | — | Same-skill repeat failed numeric/parser/geometry checks; not discarded |
| [9](../../cad-robustness-loop-workspace/iteration-9/) | 5/6 | 5/6 | Numeric pilot failed; full suite not run |
| [10](../../cad-robustness-loop-workspace/iteration-10/) | 51/55 | 52/55 | Native render/audit tools; receipt hash transcription and state/gating failures |
| [11](../../cad-robustness-loop-workspace/iteration-11/) | 54/55 | 53/55 | Byte-derived receipt fix; life/maintenance question still missing |
| [12](../../cad-robustness-loop-workspace/iteration-12/) | 53/57 | 55/57 | Two additive assertions; omitted checks and unsupported energy consequences |
| [13](../../cad-robustness-loop-workspace/iteration-13/) | 54/57 | 55/57 | Three failures moved; extra claims and read-boundary violation remain |

`—` means no comparable completed arm/score, not zero or pass. Final scores
include documented review corrections; independent originals remain beside them.
The rubric grew from 9 to 12, 53, 54, 55 and 57 assertions. Native-produced numeric
artifacts replaced free-form reporting in run 10; run 12 added metric-meaning and
authorized-I/O assertions without rescoring history. Fresh heldouts also differ.

## Durable findings

- Shared native scoring/rendering and archive inventory improved exact artifacts,
  not free-form planning reliability. Correct files do not excuse false claims.
- Run 11 added tested byte-to-JSON receipt generation and read-back; retain
  historical receipts instead of refreshing their hashes to claim new tests.
- Capture fixes retain V2 user text and complete visible history, not hidden
  reasoning. Run 12 added the regular stdout fd to avoid large CLI-to-pipe truncation.
- Word limits versus more prose did not close semantic failures. Run 13 still
  omits G0 checks, invents affected retesting and asks an extra confirmation.
- Run 13's source runner searched grader/history filenames. Its mapped score and
  artifact checks remain, but it is not clean isolated behavioral evidence.
- Run-10 preparation-invalid attempts and run-13 unrun rejected heldout drafts
  remain separately retained; no scored answer was repaired for a luckier pass.

## Forensic entrypoints

- [Coverage manifest](../../cad-robustness-loop-workspace/coverage-results.json)
- [Latest claims audit](../../cad-robustness-loop-workspace/iteration-13/claims-audit.json)
- [Exact regression comparison](../../cad-robustness-loop-workspace/iteration-13/regression-comparison.json)
- [Latest evidence verification](../../cad-robustness-loop-workspace/iteration-13/evidence-verification.json)
- [Original narrative archive](../../cad-robustness-loop-workspace/documentation-archive/revision-docs-before-consolidation-2026-11-06.tar.gz)
  and [archive hashes](../../cad-robustness-loop-workspace/documentation-archive/manifest.json)

The live runbook and acceptance contract replace the append-only narratives.
Frozen snapshots and per-run documentation remain unchanged as evidence.
