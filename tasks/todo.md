# Completion task packets

See [plan.md](plan.md) for host/admission policy and shared ownership. These are
16 logical assignments, **not registered or running agent sessions**. All sessions,
remote physical worktree paths and effective policies remain unassigned pending
native transport/campaign qualification. No framework is exempt from MCP coverage.

For every role: read the applicable spec/ledger section, relevant files and one
existing verified pattern only. Work in one small independently usable slice;
leave unrelated changes alone. Acceptance is implementation + focused tests +
framework MCP tests + source-bound handoff, with native evidence separate.
No pushes, host changes, upstream changes or weakened scientific gates.

## Dispatch prerequisites — coordinator / Atlas

- [x] Verify direct `can@dnomad:1337` destination and existing OpenCode service.
- [x] Inspect installed native command surface; record missing campaign/transport.
- [x] Check current baseline and preserve the newer registry-Fleetix change.
- [ ] Obtain qualified native campaign/transport frontend through its external owner.
- [x] Transfer/resolve Harbor-CAD through native workspace handling; verify bytes (`303bff2`, canonical Nomad adoption; staged source retained).
- [x] Supply exact locked Python source-check environment and matching CLI; verify on Nomad without remote compilation (`06ce6ec`, 15 scoped real MCP/protocol checks passed; Atlas cold suite 223 passed).
- [ ] Observe existing sessions, effective policies and physical context on both hosts.
- [ ] Declare resource reserves/capacity from fresh observations before admission.
- [ ] Register exact 16 owners/worktrees once and persist native requests before delivery.

## 01 — Shared worker coordinator — Atlas

**First slice:** Reconcile strict framework capability/qualification contracts with
the current worker without changing historical approvals.
**Files:** `src/{contracts,worker,main,lib,admission,execution,lifecycle,storage}.rs`,
shared runtime/resource/sandbox modules and `scripts/native_worker_campaign.py`.
- [ ] Keep one dispatch/lifecycle implementation and explicit independent approvals.
- [ ] Integrate domain contract deltas in small compatible batches; preserve idempotency.
- [x] Read-only CLI/MCP execution preflight reuses submission's pre-staging checks; no approval, retention, reservation or execution (`8f77e2c`, source gate and clean-runtime `c8-35/preflight`).
- [x] Verify `cargo test --locked --jobs 2 --test worker --test contracts --test lifecycle` (included in the complete 222-Rust/224-Python source gate).
**Consumes:** Each domain's narrow integration handoff; no initial dependency.

## 02 — MCP platform and conformance — Nomad

**First slice:** Enumerate actual tools/profiles and implement the executable
framework-to-tool matrix, catching campaign/profile mismatches before native work.
**Files:** `python/harbor_cad_mcp/`, central protocol/conformance tests.
- [ ] Compose domain modules without native imports or a second worker/scheduler.
- [ ] Test real SDK discovery, schema/error parity and allowed/denied profiles.
- [x] Check all static shared campaign tool/profile pairs against real SDK discovery, including finite profile loops (25 sites / 26 pairs; Atlas source checks, not a Nomad dispatch).
- [x] Reconcile all 61 registered tools / 139 allowed profile pairs against the executable 28-framework matrix at `862af55`; real-client discovery, schema, denied profiles and ten typed worker-refusal probes pass without native programs on `PATH`. At that revision PaStiX4CalculiX/PaRSEC were missing and three optional integrations deferred; this is not native or full framework qualification.
- [x] Verify `python -m pytest -q python/tests/test_protocol.py python/tests/test_native_campaign_api.py python/tests/test_retained_cooling_protocol.py` using the locked check environment (included in the 225-case Python suite).
**Consumes:** Domain-owned MCP tool contracts; current registrations suffice initially.

## 03 — Build and dependency infrastructure — Atlas

**First slice:** Reconcile registry Fleetix 0.6.0 with Nix GPU contracts and refresh
only affected exact-source packaging/evidence, not previously qualified runtimes.
**Files:** Locks, `Cargo.toml`, `pyproject.toml`, `flake.nix`, `nix/`, dependency
manifest and generated CI. Domain package changes flow through this owner.
- [ ] Preserve isolated Python/native ABIs, immutable pins and narrow unfree policy.
- [ ] Expose meaningful package/framework readiness through the shared MCP worker.
- [x] Exact-plan package/source/policy preflight through all four MCP profiles; matching lightweight CLI/MCP privately published. Broader framework-specific capability inventory remains separate.
- [x] Expose compiled lock/source provenance through existing `doctor` on CLI/all MCP profiles, without native imports/evaluation or readiness promotion (`1eaaf52`; 222 Rust / 226 Python source and exact Nix package/check gates; clean all-profile inspection passed).
- [x] Verify `python scripts/check_cpu.py`; realize current CLI/MCP/cooling worker production outputs through guarded Canix (`303bff2`, source gate 3 and `c8-34`; no broader package qualification inferred).
**Consumes:** Domain package/ABI deltas; no initial dependency.

## 04 — FreeCAD — Nomad

**First slice:** Audit current controlled-copy/region/import coverage against the
remaining CAD applicability requirements and add one missing verified vertical slice.
**Files:** `src/cad*.rs` excluding mesh/optical transport, `adapters/freecad_bridge.py`,
`adapters/{cad_variant,import_policy}.py`, corresponding tests and framework MCP module.
- [ ] Preserve original documents, world placement and semantic region identities.
- [ ] Keep opening documents inside the patched importer sandbox and approved DAG.
- [ ] Verify `python/tests/test_cad_regions.py`, `test_cad_variant.py`, `test_import_policy.py`; own native CAD campaign receipts.
**Consumes:** Shared contract integration only when needed; existing fixtures work now.

## 05 — Gmsh mesh — Nomad

**First slice:** Cover a missing imported-CAD physical-group/correspondence rejection
and expose mesh quality/correspondence through scoped MCP results.
**Files:** `src/cad_mesh.rs`, `adapters/cad_mesh.py`, mesh tests, framework MCP module.
- [ ] Bind mesh to approved original geometry, units, transforms and semantic groups.
- [ ] Verify topology/Jacobians and refuse ambiguous or unsupported correspondence.
- [ ] Verify `python/tests/test_cad_mesh*.py`; own native mesh evidence.
**Consumes:** Role 4 CAD receipts; request shared FEM helper changes from role 10.

## 06 — OpenLB flow / HIP — Nomad

**First slice:** Reconcile allocated-flow resource/observation coverage and identify
one supported bounded improvement without silently changing the science.
**Files:** `adapters/openlb.cpp`, HIP identity driver, flow/backend tests and MCP module.
- [ ] Keep Float64, explicit formulation/Mach bounds and HIP device/CPU parity evidence.
- [ ] Preserve separate compute/EGL/media routing and single-KFD qualification scope.
- [ ] Verify flow/HIP identity and refusal tests; own queued native HIP measurements.
**Consumes:** Role 1 resource/dispatch integration and role 3 packages, not initial analysis.

## 07 — OpenLB wetting / retention — Nomad

**First slice:** Verify retained-source contracts and meaningful wetting/retention
MCP inspection against existing original-field fixtures.
**Files:** `src/wetting*.rs`, `adapters/openlb_wetting.cpp`, wetting/retention adapters/tests/MCP module.
- [ ] Preserve original phase/history, conservative extrusion, units and explicit applicability.
- [ ] Refuse moving/unverifiable sources rather than inventing cooling initial state.
- [ ] Verify `python/tests/test_wetting_reference.py`, `test_wetting_native_guard.py`, `test_retention_campaign.py`; own wetting native evidence.
**Consumes:** Existing qualified sources; coordinate shared staging with role 1.

## 08 — Freezing / retained cooling — Nomad

**First slice:** Reconcile the completed exact V18 worker lifecycle gate and
deliver framework-scoped cooling inspection/MCP coverage against its originals.
**Files:** `src/{freezing*,retained_cooling,cooling*}.rs`, cooling/freezing adapters,
`scripts/verify_retained_cooling_worker.py`, focused tests and MCP module.
- [ ] Preserve stationary equal-property scope, original water fraction and all fixed gates.
- [x] Before-ack retention, source loss, restart/idempotency, cancellation/death and export/release evidence pass `c8-34` with original CSV/VTK queries. Completed by Atlas coordinator while owner registration remains blocked; not a Nomad dispatch receipt.
- [ ] Verify `python/tests/test_retained_cooling*.py`, `test_freezing*.py`; run the exact newly captured worker campaign on Atlas.
**Consumes:** Role 7 closed originals, role 3 matching packages; no baseline source replacement.

## 09 — Thermal / moisture / snow — Nomad

**First slice:** Assemble the missing combined cold/wet moisture-risk report from
verified thermal histories, preserving missing/inapplicable branches.
**Files:** Thermal/transfer/moisture/snow modules, `adapters/thermal_history.py`,
corresponding tests and domain MCP module; not shared `fem_reference.py`.
- [ ] Preserve explicit material domains, physical times and conservative coupling.
- [ ] Never infer convection from velocity or snow deposition/flow from prescribed coverage.
- [ ] Verify `python/tests/test_thermal*.py`, `test_snow*.py`; own focused native receipts.
**Consumes:** Existing thermal originals; shared helper requests go to role 10.

## 10 — Mechanics / contact / factorization — Nomad

**First slice:** Audit compatible CalculiX/PaStiX/PaRSEC contracts and implement
honest factorization/resource readiness and contact MCP inspection.
**Files:** FEM/imported/contact/thermal-contact modules and adapters, including sole
ownership of `adapters/fem_reference.py`; mechanical tests and MCP module.
- [ ] Keep CPU references independent; CUDA remains best-effort/unqualified without evidence.
- [ ] Bound fill-in/workspace estimates; preserve missing gasket/material data.
- [ ] Verify `python/tests/test_fem*.py`, `test_contact_reference.py`, `test_thermal_contact.py`; own native FEM/contact evidence.
**Consumes:** Role 9 temperature-transfer contract and role 3 packages when integrating.

Coordinator continuation, not a Nomad owner turn: source-only candidate inspection
now exposes pinned modified PaStiX/PaRSEC compatibility/build/ILP64 blockers and
unknown resource estimates through `doctor`/`backend_list`. CPU planning parity
and strict no-fallback checks pass in the 228-case Python source gate. Matching
packaged verification remains pending; source hashes and terminal empty service
trees are reconciled after intermittent harness launch refusals. This does not complete the
factorization/resource or independent domain qualification assignments above.

## 11 — Atmospheric radiation — Nomad

**First slice:** Add framework-scoped libRadtran source/input/output inspection and
verify conformance against retained angular originals without rerunning old campaigns.
**Files:** Atmosphere fields/source/transfer modules and adapter, focused tests/MCP module.
- [ ] Preserve direct/diffuse angular/spectral units, immutable source and boundary gates.
- [ ] Expose explicit synthetic scope and separate execution/refinement/physical statuses.
- [ ] Verify `python/tests/test_atmosphere*.py`; own native atmosphere evidence.
**Consumes:** Existing `c8-26` receipts; role 12 consumes versioned source handoffs.

## 12 — Optical transport / exposure — Nomad

**First slice:** Expose meaningful Mitsuba/Dr.Jit scene/source/transport inspection
and independent power/dose queries for the supported original-facet workflows.
**Files:** Spectral/CAD-optical/atmospheric-transport modules and adapters/tests/MCP module.
- [ ] Preserve original facet order, fixed native precision screens and unknown ageing.
- [ ] Keep unsupported curved/mixed/transmitting/interreflection and GPU paths explicit.
- [ ] Verify `python/tests/test_spectral*.py`, `test_cad_spectral*.py`, `test_atmospheric_spectral*.py`; own native optics evidence.
**Consumes:** Roles 4/11 exact sources and role 3 compatible packages; existing fixtures suffice initially.

## 13 — GPU numerical analysis — Nomad

**First slice:** Extend exact VTK field/association/round-trip coverage and expose
the qualified Viskores/Kokkos filter allowlist with its real backend restrictions.
**Files:** `src/{fields,filters}.rs`, `adapters/filters/`, numeric-format/filter tests/MCP module.
- [ ] Own shared array/association contracts; coordinate—not duplicate—with role 14.
- [ ] Preserve precision/topology/IDs/ghosts/units and independent numerical GPU evidence.
- [ ] Verify `cargo test --locked --jobs 2 --test retained_fields`; own queued native filter evidence.
**Consumes:** Domain original formats and existing qualified filter fixtures.

## 14 — Scientific visualization — Nomad

**First slice:** Verify ParaView/EGL retained-field render recipes and truthful
format support/limitations through scoped MCP, independent of numerical filtering.
**Files:** `src/{frames,presentation}.rs`, `adapters/paraview_bridge.py`, render tests/MCP module.
- [ ] Keep fixed scales, units, physical-time labels and selected EGL identity.
- [ ] Treat Catalyst/Conduit/VTKHDF as optional; no placeholders masquerading as support.
- [ ] Verify `python/tests/test_render_contract.py`; own queued native render evidence.
**Consumes:** Role 13 association contracts and role 3 exact packages.

## 15 — Hardware media — Nomad

**First slice:** Cover FFmpeg/VAAPI frame/timestamp/selected-device inspection and
the approved registered-frame video interface through real MCP clients.
**Files:** `adapters/video_bridge.py`, video-specific tests and MCP module;
shared frame contract changes go through role 14.
- [ ] Keep encoding independent of compute/render and scientific originals lossless.
- [ ] Verify actual decode, labels, timestamps and bounded staging—not codec listings.
- [ ] Verify `python/tests/test_video_contract.py`; own queued native media receipts.
**Consumes:** Role 14 registered frames; existing manufactured sequences suffice initially.

## 16 — Independent qualification / product closure — Atlas

**First slice:** Map every remaining A0–F requirement to domain-owned evidence,
missing physical inputs or explicit unsupported capability; review stale claims.
**Files:** Acceptance/qualification/product docs and independent cross-domain checks.
- [ ] Review exact source/package/hardware evidence without promoting synthetic/CPU scope.
- [ ] Reconcile framework MCP coverage, recovery/resource measurements and coupled recipes.
- [ ] Verify current full source gate and independent cross-domain checks; preserve all failed campaigns and the Paperclip audit.
**Consumes:** Every role's versioned evidence; audit existing ledger immediately.

## Integration checkpoints

- [ ] Initial dispatch: native command/transport/workspace/policy/resource/context prerequisites pass.
- [ ] After each small batch: affected tests, framework MCP matrix and source identity pass.
- [ ] Before a native campaign: matching newly committed capture/packages and independent approval pass.
- [ ] Closure: A0–F reconciliation and fresh complete idle/no-pending/no-leak observations pass.
