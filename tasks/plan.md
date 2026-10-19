# Harbor-CAD completion: Atlas coordinator, Nomad owners

Planning revision: 1, 2026-10-19. Source baseline:
`27e853b2c857c92e6307396829d804cc3944fde6`, `implementation/local-worker`.
The [implementation specification](../docs/implementation-spec.md) remains
authoritative; the [ledger](../docs/acceptance-ledger.md) records scoped evidence,
not a transferable qualification for changed packages or hosts.

## Goal and decisions

- Complete the remaining A0–F implementation and evidence without weakening
  applicability, numerical gates, source preservation, approvals or isolation.
- Place 13 of the 16 logical owner roles on **Nomad through `can@dnomad`**.
  Keep worker integration, build authority and independent acceptance on Atlas.
  A role's development host is not its solver's qualified execution host.
- Use existing registered sessions when found; do not replace their models,
  histories or permissions. No worker sessions have been created by this plan.
- Keep commits local on `implementation/local-worker`; push nothing. Do not
  change host configuration, upstream projects or the independent Paperclip audit.
- HIP/ROCm is the AMD OpenLB priority; CUDA stays best-effort. Vulkan requires
  an actual supported implementation, not a new compatibility claim.

Tasks, owners, first bounded slices and checks are in [todo.md](todo.md).
All roles have useful work against existing contracts. A missing producer blocks
the consuming integration task, not the owner's whole assignment.

## Admission prerequisites and current observations

| Prerequisite | Observed on 2026-10-19/20 | Release condition |
|---|---|---|
| Atlas shell and source | Shell restored; checkout initially clean at the baseline | Recheck scoped diffs/HEAD before each integration |
| Direct route | `canix host ssh ... dnomad -- hostname -s` returned `nomad`, as `can` on port 1337 | Reverify destination on each transport |
| Handoff profile | `canix host profile status atlas-adjacent nomad` reports `ON` via `dnomad` | Preserve the existing profile; no activation requested |
| Nomad runtime | `opencode v2.0.22+047c74e`; existing service active on loopback port 4096; host-local credentials authenticated read-only API discovery | Native transport plus complete session-policy/context observation remain required; no worker admitted |
| Campaign frontend | Installed `canix fleet --help` on both hosts has no `campaign`; Atlas `host opencode` is also absent | Qualified Canix producer/consumer exposes `import`, `plan`, `status`, `run`, `deliver`, `checkpoint`, `stop` and native OpenCode transport |
| Project transfer | Complete `303bff2` bundle verified and adopted through native workspace preview/apply at `/data/can/canix/projects/owned/harbor-cad`; verified incremental bundle fast-forwards cleanly to `06ce6ec` | Complete for that source; do not transfer/adopt again or infer worker admission |
| Nomad test environment | Exact prebuilt locked `python-test-environment` and source-compatible CLI transferred/verified; 15 real MCP/protocol checks pass without `.venv`, installers or remote compilation | Complete scoped source-check environment; native solver and agent admission remain separate |
| Capacity | Nomad OpenCode service has an 8 GiB hard ceiling; observed usage about 1.8 GiB | Fresh backend/cgroup and existing-session observations, explicit aggregate admission policy and diagnostic reserve |

These are observations, not dispatch receipts. The 13 roles are **not** a grant
for 13 simultaneous turns. Unknown context usage or capacity blocks admission.
Do not create a campaign-local Python/JavaScript scheduler, change services,
read credentials onto Atlas, bypass leases or claim that service readiness is
campaign qualification. Missing native commands are an external release gate;
repairing their producer is outside this repository's authorized scope.

Atlas continuation completed exact retained-cooling worker campaign `c8-34` at
`303bff2`; [source-bound evidence](../docs/evidence/retained-cooling-worker-20261019.json)
records CLI/MCP/VTK/lifecycle gates and post-server-restart release checks.
This is coordinator work while dispatch is blocked, not a Nomad owner/session
receipt. All 16 session identities remain unassigned.

[Portable source-check evidence](../docs/evidence/portable-python-checks-20261019.json)
records the cold Atlas suite (223 passed) and scoped Nomad protocol checks (15
passed), including the preserved three-test portability failure. Read-only Nomad
enumeration covers all 304 sessions, with no Harbor-CAD candidate or active session
at that observation. Atlas continuation reused the first 40 historical pages and
read 44 more, reaching the end cursor: 8,291 sessions at
`2026-10-19T21:54:44.767064+00:00`. Only this coordinator and its old idle review
child matched Harbor-CAD; five sessions were active. The retained summary is
`/home/can/.local/state/harbor-cad-opencode-observation-2/summary.json`, SHA-256
`1a57a34056eacc40490a5f5e44c0c169d2d08bc1dabb5d68460de35b7240b00a`.
This completed historical enumeration is not a frozen current inventory, an
effective owner-policy/context review or a capacity/admission grant.

At `2026-10-19T22:17:52.332154+00:00` the coordinator's own canonical location and
merged agent response were observed read-only. Its latest completed input/cache
total exceeded 180k tokens, so native compaction was requested with persisted ID
`msg_harborcad_compact_20261020_1`. The native context subsequently contains that
compaction and four active messages; the latest completed input/cache total is
94,922 tokens. The private post-compaction receipt SHA-256 is
`a21d6cd1ff4d4c0968d983604113c14f6bb5e867b52990ad35aabc0b85b2fa14`.
No framework owner has been registered, admitted or prompted by these checks.

## Ownership and integration

- Role 1 owns shared Rust contracts, protocol/dispatch, CLI composition, admission,
  lifecycle and `scripts/native_worker_campaign.py`. Domain owners propose small
  integration patches rather than racing edits in shared files.
- Role 2 owns `python/harbor_cad_mcp/server.py`, client/composition, profile schemas
  and the executable framework/tool/profile/error matrix. Domain owners own their
  focused framework MCP modules and real-client tests; no native import at discovery.
- Role 3 alone owns locks, dependency manifests, `flake.nix`, package composition,
  Nix integration and generated CI. Domain owners provide source/ABI/package deltas.
- Role 10 owns shared `adapters/fem_reference.py`; roles 5 and 9 request helper
  changes through that owner. Role 13 owns shared numerical VTK/association
  contracts with role 14 as the presentation consumer, not a second writer.
- Role 16 owns the acceptance ledger and final evidence reconciliation. Each
  domain produces and tests its own source-bound evidence; role 16 reviews it
  independently instead of executing every domain campaign.

Resolve isolated Harbor-CAD worktrees through the workspace workflow after remote
preflight. Do not create Canix worktrees, reset foreign branches, copy live SQLite
state, or clone into guessed workspace roots. Register exact worktree, baseline,
session, title, model, permissions, context usage and host before any admission.

Integrate small independently usable batches continuously. Refresh HEAD and
retest affected consumers; no foundation-wide waiting wave. Compilation and
qualified native campaigns stay on Atlas. Nomad realization is permitted only
through the explicit guarded Atlas offload interface; never issue compiler/Nix
build commands over SSH or create a reverse builder path.

## Framework-wide MCP contract

One distribution and shared Rust worker, with scoped interfaces/profiles—not
separate schedulers or arbitrary native-code execution. This table assigns
coverage; it does not claim that the new interfaces already exist.

| Framework / runtime | Owner | Meaningful MCP coverage to verify |
|---|---|---|
| Fleetix | 1 | Exact route/source contracts, backend identity, refusal and approved worker routing |
| SQLite, systemd, Bubblewrap | 1 | Durable jobs/events, owned service/isolation receipts, cancellation/recovery and bounded inspection |
| Official MCP SDK | 2 | Real-client discovery, schema, profiles, typed errors and job/resource behavior |
| Nix, Harbor-Rs, Harbor-Py | 3 | Package/source/ABI/policy inspection and allowed workflow readiness; no arbitrary build/eval tool |
| FreeCAD | 4 | Sandboxed inspect/regions/export and independently approved controlled copies |
| Gmsh | 5 | Source-bound mesh planning, physical groups, correspondence and quality results |
| OpenLB + HIP/ROCm | 6, 7, 8 | Flow, wetting/retention and freezing/cooling planning, approved execution, original-field inspection |
| CalculiX | 9, 10 | Thermal/moisture/snow and mechanical/contact plans and verified native results |
| PaStiX4CalculiX, PaRSEC | 10 | Exact compatibility/factorization/resource inspection and supported approved FEM workflows; unsupported execution refuses |
| libRadtran | 11 | Explicit atmospheric inputs, approved native source generation and original angular/spectral queries |
| Mitsuba, Dr.Jit | 12 | Optical scene/material/source planning, bounded transport and irradiance/power/dose queries |
| VTK, Viskores, Kokkos | 13 | Original-array/association contracts, qualified numerical filters, device/precision evidence |
| ParaView + bundled VTK + EGL | 14 | Retained-field rendering, camera/scales/units/time recipes and independent render identity |
| FFmpeg + VAAPI | 15 | Registered-frame video plans, selected media identity and decoded-frame/timestamp inspection |
| Catalyst, Conduit, VTKHDF | 14 | Optional: inventory and explicit unqualified/deferred status; add execution only after measured justification |

The executable matrix must enumerate every framework actually used, including
embedded/build frameworks, map it to real registered tools and profiles, and test
allowed and denied calls. Build-only frameworks expose inspection through the
worker, not shell access. Do not fabricate successful optional integrations.

## Restart-safe handoffs and resource queue

The native campaign journal retains one coordinator lock, exact persisted
request IDs/bodies, active/queued inputs, owner observations and transfer digests.
Delivery is not execution. Reconcile inbox and transcript after ambiguous replies;
never invent another ID to bypass uncertainty. Keep Atlas mirrors parked when
Nomad owns a session. Preserve unrelated existing campaigns and their schedulers.

Each handoff carries source/target revisions, schema versions, dependency pins,
units/coordinates, original/native/derived field associations, artifact checksums,
qualification scope, fixtures, exact commands/results and limitations. A remote
path is usable evidence only after bounded byte exchange and digest acknowledgement.
Native originals may remain on Atlas; send compact receipts rather than copying
live worker databases or entire caches. Preserve unverifiable source orphans.

Roles own bounded turns targeting 120–160k context. Checkpoint before 180k,
request native compaction at 180k observed input/cache tokens, and verify it before
continuation. Reconcile or renew context before 240k; never admit work projected
to reach 280k, retaining margin below the user's 300k ceiling and model limits.

Run source analysis/edits and non-compiling checks concurrently within observed
budgets. Role 3 queues guarded builds; role 1 queues native services and shared-card
reservations. Keep evaluation, build, activation and campaign leases separate.
Recheck live RAM/disk/VRAM, reserve diagnostic headroom, bound threads/tasks/time,
and queue rather than changing scientific accuracy or overriding reserves.

## Completion gates

1. Dispatch prerequisites pass and all 16 exact owners are registered once.
2. Each domain delivers tested implementation, meaningful MCP coverage and
   source/package-bound evidence or an explicit unsupported/unqualified boundary.
3. Role 16 reconciles A0–F independently, preserving numerical/physical distinctions,
   failed campaigns, missing physical inputs and the independent Paperclip audit.
4. Fresh error-free observations show settled handoffs, no active/pending owner
   turns and no leaked owned processes/reservations. Idle or delivered is not done.
