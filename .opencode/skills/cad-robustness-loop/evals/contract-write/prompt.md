---
tags: [skill-creator]
max_turns: 20
timeout_seconds: 300
allowed_tools: [Read, Glob, Grep, Skill, Write, Edit, Bash]
---

Implement these approved decisions in a NEW outputs/requirements.json, preserving
all fixture inputs and the old receipt. Use the provided local JSON shape; this
is a synthetic project contract, not a harbor-cad core API schema. Do not alter
existing stock floors, hardware, unknown physical inputs or open decisions.

Approved: 95-minute outage at -18 C, initially warm-running panel, aged pack,
six 60-second video/audio calls, camera off between calls, all heating included,
no solar credit and required building network/access-control backup. Report
continuous-call runtime separately; battery-only cold start is excluded. Main
body cap: width175 x height355 x depth88 mm. Compare finished aluminum and
stainless under identical requirements; no grade/finish, lifespan, battery
retirement, mass/cost cap, installed clearance or measured power/capacity approved.

For mission use keys: ambient_k, duration_s, initial_state ("warm_running"),
aged_pack, heating_included, solar_credit, calls ({count, duration_s_each}),
camera_between_calls ("off"), building_backup_required, continuous_call_runtime
("separate"), battery_only_cold_start_included. body_limit_mm uses width/height/depth.
material_routes uses ["finished_aluminum", "stainless"]. Preserve other keys.
Use JSON integers for durations, call counts and body dimensions, JSON booleans
for flags, and a numeric Kelvin ambient value.

Write only that file and outputs/change-record.json. The latter has exactly
source_requirements_sha256, historical_receipt_sha256, requirements_sha256
(hash the actual output bytes), executed:false, qualification:"unqualified" and
approval_required:true. Do not overwrite existing outputs or update old hashes
to falsely claim new tests. Do not run CAD, solvers, workers or approve a plan.

Input files: fixture
