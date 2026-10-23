# Harbor-CAD completion: Atlas workstreams

Planning revision: 2, 2026-10-23. Source baseline: trunk `e271596`.
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
    of scope. Workstreams W1–W9 replace the 16-role, two-host plan.
  - Cache pinning uses the harbor-rs `lib.mkCachePin` helper, and pushing is
    authorized for it.
  - After containment, move nixpkgs **now** to the newest fully cached revision,
    then re-qualify the native stages.
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
| W5 | harbor-rs `lib.mkCachePin` (PR #20 merged); consumer patch prepared, not applied | this plan, Phase B |
| W7 | Wetting HIP path implemented, unqualified; typed CPU-only refusals; render/compute routing | [gpu-backends](../docs/gpu-backends.md) |
| W8 | Chained templates, W1 providers, calibration envelopes | [design-loop](../docs/design-loop.md) |
| W9 | General-solid Gmsh mesh; steady enclosure conduction (CalculiX deck) | [enclosure-thermal](../docs/enclosure-thermal.md) |

W6 (CLI/MCP integration) was folded into W1–W4 and W8–W9: each wired its own
CLI commands and, except W9's two planning operations (CLI and design loop only),
its MCP tools (61 → 69). All of these are source/CPU only. They
have no Nix build, no native qualification and no A0–F closure. See the
2026-10-23 ledger slice for the gate counts and limits.

## Phase 0 — cache security (blocks everything below)

`/etc/nix/nix.conf` still trusts the compromised `lantian` cache. The operator runs
`/data/scratch/tmp/opencode/harbor-cad-cache-security-20261020/operator-unified.sh`
(SHA-256 `4f5114e6e429d156df8d69e04d748400a1b114de4843e4080b92929e7d13e1d8`). It
matches runbook §1–3 except for an ERR trap and audit roots that are reduced to
the running system, because the retained harbor-cad outputs were garbage-collected.
The agent then verifies the returned evidence, and the Canix security owner gives
a disposition. Nix-token rotation is unverified.

## Phase B — after disposition, in order

1. Apply `/data/scratch/tmp/claude-1000/harbor-cad-cache-pin-29073b5.patch`, then
   run `nix flake lock --option substitute false`. This needs user approval.
2. Run `nix eval .#cachePinMeta --json`.
3. Run `canix repo update cache-pin nixpkgs` and record its report.
4. Run `canix cache binary build` for the CLI, MCP, test environment, openlb-hip,
   wetting-native-{cpu,hip}, runtime-wetting-worker-hip, filter-hip, the
   FEM/CAD-mesh runtimes and gui.
5. Rerun `scripts/check_cpu.py` in the pinned devShell.
6. Run the native gates: `verify_native_hip`, `verify_openlb_hip`,
   `verify_filter_hip`, `verify_hip_sandbox`, `verify_cad_variant_worker`,
   `verify_study_worker`, `verify_wetting_hip` and
   `verify_enclosure_thermal_cpu.py --ccx`.
7. Run the enclosure demo design loop with labelled synthetic inputs.
8. Have an independent reviewer close A0–F.

Keep evaluation, build, activation and campaign leases separate. Recheck live
RAM/disk/VRAM, bound jobs/cores/time, and queue rather than changing scientific
accuracy or overriding reserves. A failed, cancelled or skipped gate is not a pass.

## Completion gates

1. Phase 0 disposition is recorded, and Phase B packages and native gates pass at
   exact recorded identities, or each one has an explicit unsupported/unqualified
   boundary.
2. Every framework keeps meaningful MCP coverage through the shared Rust worker,
   tracked by the executable matrix (69 tools, 28 frameworks at `e271596`).
3. An independent closure review reconciles A0–F. It preserves the
   numerical/physical distinctions, failed campaigns, missing physical inputs and
   the Paperclip audit.

## Superseded

Revision 1 (2026-10-19) planned 16 logical roles: 13 on Nomad and 3 on Atlas,
dispatched through a native campaign transport. The transport never qualified and
no owner was ever registered. That text and its observations (Nomad
route/profile, the 8,291/304 session enumerations, compaction receipts) are
preserved in Git history at `e271596:tasks/plan.md`. The evidence those
observations cite stays under `docs/evidence/`.
