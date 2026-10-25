# Harbor-CAD completion: Atlas workstreams

Planning revision: 3, 2026-10-24. Source baseline: trunk `f738aed`.
The [implementation specification](../docs/implementation-spec.md) remains
authoritative; the [ledger](../docs/acceptance-ledger.md) records scoped evidence,
not a transferable qualification for changed packages or hosts.

## Goal and decisions

- Complete the remaining A0–F implementation and evidence without weakening
  applicability, numerical gates, source preservation, approvals or isolation.
- User decisions of 2026-10-23:
  - Cache containment is **operator-assisted**. The agent prepares and reviews
    the evidence, and the operator runs the privileged steps.
  - The first product is an **outdoor electronics enclosure**, piloted at
    **10–100 units**.
  - Agents run **in-session on Atlas only**. Nomad and campaign transport are out
    of scope. Workstreams W1–W18 replace the 16-role, two-host plan.
  - Cache pinning uses the harbor-rs `lib.mkCachePin` helper, and pushing is
    authorized for it.
  - After containment, move nixpkgs **now** to the newest fully cached revision,
    then re-qualify the native stages.
- User decisions of 2026-10-24:
  - PCB power is a **swept** design quantity (1–15 W). The component limit and
    board conductivity are cited conservative defaults.
  - The pilot PCB **will have at least one continuous copper plane**, so the
    in-plane 8.1 W/(m·K) bound applies. The exported design report states this
    board requirement.
  - Disposition "released for build and native qualification" is
    **pre-authorized only if** every containment check and the provenance
    triage pass. Anything suspicious stops Phase 3 and goes back to the user.
  - The coordinator issues `nix flake lock --option substitute false` after
    containment, and the user approves that prompt.
  - iGPU activation (Phase 4) is in scope. The user authorizes the host step at
    the time.
- HIP/ROCm is the AMD OpenLB priority; CUDA stays best-effort. Vulkan requires
  an actual supported implementation, not a new compatibility claim.
- Do not change unrelated host configuration, upstream projects (beyond the
  authorized cache-pin work) or the independent Paperclip audit.

Tasks and checks are in [todo.md](todo.md).

## Workstreams (source level, done)

| ID | Delivered on trunk | Docs |
|---|---|---|
| W1 | Sourced material/process library; estimate-only cost, manufacturability, service margins | [materials-and-cost](../docs/materials-and-cost.md) |
| W2 | Spreadsheet-alias CAD variants; synthetic `enclosure_fixture` seed | [cad-variants](../docs/cad-variants.md) |
| W3 | Bounded approval-bound design loop over ordinary jobs; 5 MCP tools | [design-loop](../docs/design-loop.md) |
| W4 | Measured CSV vs thermal-history correlation | [correlation](../docs/correlation.md) |
| W5 | harbor-rs `lib.mkCachePin` (PR #20 merged); consumer wiring landed as harbor-cad PR #5 (`8d1cf6c`) | Phase 3 |
| W7 | Wetting HIP path implemented, unqualified; typed CPU-only refusals; render/compute routing | [gpu-backends](../docs/gpu-backends.md) |
| W8 | Chained templates, W1 providers, calibration envelopes | [design-loop](../docs/design-loop.md) |
| W9 | General-solid Gmsh mesh; steady enclosure conduction (CalculiX deck) | [enclosure-thermal](../docs/enclosure-thermal.md) |
| W10 | Docs close-out of W1–W9 (`7034bdf`) | spec §10, ledger 2026-10-23 |
| W11 | ETSI EN 300 019-1-4 classes, film ranges, finishes, `sourced` metrics (`7d2f4f2`, `8f7942e`) | [design-conditions](../docs/design-conditions.md) |
| W11b | Window solar bound, compatible triples, sourced cavity film, hotter-of-board/air margin (`be77e98`, `357ffc8`) | [enclosure-thermal](../docs/enclosure-thermal.md) |
| W12 | MCP `cad_plan_solid_mesh`, `enclosure_thermal_plan` (69 → 71); render/media single-node DRM policies (`cb97aec`, `549a3de`) | [gpu-backends](../docs/gpu-backends.md) |
| W13a | KFD multi-GPU exclusion receipt gate, `verify_kfd_exclusion.py` (`9e085e9`), not run | [host-authority](../docs/host-authority.md) |
| W13b | Canix `5df6b230d`: opt-in `igpu-amdgpu` boot entry, local and not activated | Phase 4 |
| W14 | PCB power sweep by superposition, `max_supported_pcb_power_w`, cost vs power Pareto (`278e06b`..`cb60eee`) | [design-loop](../docs/design-loop.md) |
| W15 | Linearized exterior radiation and orthotropic board lower bounds (`e277489`, `c8a66b5`) | [enclosure-thermal](../docs/enclosure-thermal.md) |
| W16 | Multi-start pattern search with constraint edge-following (`9b7187d`, `204c11d`) | [design-loop](../docs/design-loop.md) |
| W17 | Cavity radiation lower bound behind a strict postcondition; off in the pilot (`29639fc`, `f738aed`) | [enclosure-thermal](../docs/enclosure-thermal.md) |
| W18 | This docs refresh: spec §10, ledger 2026-10-24, plan, handoff | — |

W6 (CLI/MCP integration) was folded into the other workstreams. All of these are
source/CPU only. They have no Nix build, no native qualification and no A0–F
closure. See the 2026-10-23 and 2026-10-24 ledger slices for the gate counts
(latest at `f738aed`: 284 Rust and 300 Python tests) and limits.

## Phase 0 — cache security (done)

A build-host cache-security gate (retired third-party binary cache) was
contained and dispositioned by the operator on 2026-10-25; details are kept in
the operator's private incident evidence.

## Phase 3 — after disposition, in order (revision 2's Phase B)

1. Done: harbor-cad PR #5 (`8d1cf6c`) wires `lib.mkCachePin` with the lock.
2. Next: bump nixpkgs via the cache-pin search. Run
   `canix repo update cache-pin nixpkgs` and keep its report. Drop a custom
   derived target from `from`; never weaken `caches`.
3. Run `canix cache binary build` with bounded jobs for the CLI, MCP and test
   environment, the CPU/HIP runtimes, `openlb-hip` and `filter-hip`, wetting, and
   the CAD/mesh/FEM/thermal runtimes (including CalculiX) plus `gui`. GC-root each
   qualified output under the evidence directory.
4. Run the gates and keep failed attempts:
   - `scripts/check_cpu.py` in the pinned 1.94 devShell;
   - the native HIP gates `verify_native_hip`, `verify_openlb_hip`,
     `verify_filter_hip`, `verify_hip_sandbox` and `verify_wetting_hip`;
   - `verify_cad_variant_worker` and `verify_study_worker`;
   - the render/media gates under the DRM policies, `verify_egl_fields` and
     `verify_vaapi`;
   - the first real CalculiX run, `verify_enclosure_thermal_cpu.py --ccx`.
5. Run the synthetic demo loop. Then run the real enclosure loop: the ETSI 4.1
   hot and cold points, cited finishes and defaults, swept power, routes
   CNC 6061, 5052 sheet and PA12, and at most 32 evaluations. Export the winner
   with its cost range, supported power, margins, evidence labels and the
   copper-plane board requirement.

## Phase 4 — iGPU activation (user authorizes the host step)

1. Run `canix host rebuild boot atlas` from a clean, deployable Canix trunk that
   contains `5df6b230d`, coordinated with the other Canix workers. This also
   delivers `d770d18a4` permanently. Then reboot into the `igpu-amdgpu` entry.
   The fallback is the default entry. Check whether Immich's ROCm ML picks up the
   iGPU.
2. Run `verify_kfd_exclusion.py` and install its receipt through
   `authority install`. Run the iGPU `verify_egl_fields`/`verify_vaapi` and rerun
   `verify_native_hip`.
3. Do a B1-style concurrent run: render/media on the iGPU while OpenLB-HIP runs
   on the dGPU.

## Phase 5 — closure

A fresh independent agent reconciles A0–F. Refresh the handoff, the ledger and
`tasks/`. Pushing harbor-cad needs the user's authorization.

Keep evaluation, build, activation and campaign leases separate. Recheck live
RAM/disk/VRAM, bound jobs/cores/time, and queue rather than changing scientific
accuracy or overriding reserves. A failed, cancelled or skipped gate is not a pass.

## Completion gates

1. The Phase 0 disposition is recorded. Phase 3 packages and native gates pass at
   exact recorded identities, or each one has an explicit unsupported/unqualified
   boundary.
2. Phase 4: the exclusion receipt is bound to the running kernel, iGPU EGL/VAAPI
   pass, dGPU HIP re-verifies, and the default boot entry is unchanged. If any
   of these is not met, the concurrent route stays unqualified.
3. Every framework keeps meaningful MCP coverage through the shared Rust worker,
   tracked by the executable matrix (71 tools, 28 frameworks at `f738aed`).
4. An independent closure review reconciles A0–F. It preserves the
   numerical/physical distinctions, failed campaigns, missing physical inputs and
   the Paperclip audit.

## Superseded

Revision 2 (2026-10-23, at `e271596`) is preserved at `7034bdf:tasks/plan.md`;
its Phase B is Phase 3 here. Revision 1 (2026-10-19) planned 16 logical roles: 13
on Nomad and 3 on Atlas, dispatched through a native campaign transport. The
transport never qualified and no owner was ever registered. That text and its
observations are preserved at `e271596:tasks/plan.md`, and the evidence they cite
stays under `docs/evidence/`.
