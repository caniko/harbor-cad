---
tags: [skill-creator]
max_turns: 6
timeout_seconds: 180
allowed_tools: [Read, Glob, Grep, Skill, Bash]
---

For a geometry screening decision, use these four positive lower hard limits: wall >= 1.6 mm, seal_land >= 2.5 mm, edge_distance >= 3.0 mm and internal_clearance >= 10.0 mm. The objective weights are wall 0.40, seal_land 0.30, edge_distance 0.20 and internal_clearance 0.10.

For a lower limit L > 0, define the margin as m = (x - L) / L and the weighted margin as s = w * m. Let M = min(s) be the worst weighted margin, so the reported objective is O = -M. All four hard limits must hold. manufacturing_ok is a hard boolean gate only: false is a failure and it is not an objective term. A missing or non-finite selected value is unknown, not a pass. There are no zero limits in this table.

| candidate | wall | seal_land | edge_distance | internal_clearance | manufacturing_ok |
| --- | --- | --- | --- | --- | --- |
| P | 2.00 | 3.00 | 4.20 | 14.0 | true |
| Q | 1.76 | 3.00 | 3.60 | 14.0 | true |
| R | 2.20 | 2.75 | 3.30 | 11.0 | true |
| S | 2.40 | 3.50 | 4.80 | 16.0 | false |
| T | 2.20 | missing | 4.50 | 15.0 | true |
| U | 2.20 | 3.20 | 4.20 | overflow to +inf | true |

Calculate M and O for every scorable feasible candidate, identify the winner and
ties, and state the disposition of S, T and U. Confirm distinct selected constraints
and finite positive weights. Check the supplied proposed box/ledger and finite
evaluation, wall-time and artifact ceilings. No actual child-plan resource estimate
or body-size cap is supplied; neither the box nor its ceilings are approved.

The fixture supplies immutable `inputs/screen.json` in the local
`DesignScreenRequest` schema. Missing and non-finite values use JSON null, never
zero; the table above remains the governing scalar input. Read-only scalar CLI
calculation/schema inspection is allowed. Write only `outputs/report.json` and
`outputs/response.md`; keep the numeric report generated from its calculation,
without separately hand-calculated annotations. Do not overwrite existing outputs
or change inputs/product files. Do not execute native CAD, solver, worker or
campaign jobs or approve a plan. The fixture checker compares both files to the
read-only native calculation and renderer, in addition to semantic/trace grading.
