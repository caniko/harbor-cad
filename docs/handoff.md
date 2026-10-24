# Harbor-CAD continuation handoff

Snapshot: **2026-10-24**. This is the public handoff: host-security details live
only in the operator's private incident evidence and are deliberately left out
of this repository.

## Start here

Harbor-CAD is **not complete**. Workstreams W1–W18 are done **at source level
only**: the product design loop, materials and cost, correlation, CAD variants,
GPU backends, enclosure thermal, design conditions, power sweep, conservative
bounds, the proposer, DRM policies and the KFD exclusion gate. No Nix build and
no native qualification has happened since `96563f4` (2026-10-20).

Build and native work is **held behind a host cache-security gate** on the build
host. The operator runs the containment step and the agent verifies its
evidence (private). Phase 3 below starts only after that gate's disposition.

Skip rerunning unchanged source gates just to write documentation. Run nothing
privileged and push nothing without the operator's authorization.

## Revisions

| Item | State |
| --- | --- |
| Harbor-CAD | `caniko/harbor-cad`, branch `trunk`; also the harbor superproject's `components/cad` submodule |
| Trunk | `7741f2c`: PR #1 (the pilot design loop, 39 commits + `28ed12c`, which fixes the pinned 1.94 Clippy lints) and PR #2 (the harbor superproject sync, Python workspace moved to `python/`) |
| Last Nix-packaged source | `96563f4` (2026-10-20); nothing since has been built with Nix |
| harbor-rs | `29073b5` provides `lib.mkCachePin` (nix-cache-pin `0.3.0` = `a69647e`), CI green |
| Harbor-CAD pins | nixpkgs `73e728dd…` (URL rev); toolchain `rust-toolchain.toml` 1.94.0 |

Hosted CI runs Clippy from the pinned 1.94 toolchain. Local gates on a newer
nightly once missed 9 lints, so land every change through a PR with CI, or
gate inside the pinned devShell.

## Decisions

- **Objective:** completion against `docs/implementation-spec.md`, with
  framework-wide MCP coverage and independent A0–F acceptance.
- **Product:** an outdoor electronics enclosure, piloted at **10–100 units**.
- **Execution:** Atlas is the only build and GPU host, with in-session agents.
- **nixpkgs:** pinned through `rust.mkCachePin`, so it only moves to revisions
  whose required upstream binaries are pre-built on `cache.nixos.org`. Bump it to
  the newest fully cached revision after the cache-security gate, then
  re-qualify the native stages.
- **PCB power:** swept (1–15 W). The component limit (85 °C industrial grade) and
  board conductivity are cited conservative defaults.
- **Board requirement:** the pilot PCB **has at least one continuous copper
  plane**, so the in-plane 8.1 W/(m·K) bound applies. State this requirement in
  the exported design report.
- **iGPU (Phase 4):** in scope. The operator authorizes the host step when it
  comes up.

Preserve configured Git identities and signing; never use `--no-verify`.

## What is done (source/CPU only)

All of this is on trunk; the docs are in `docs/`.

- **Design loop** (`design-loop.md`):
  - Approval-bound envelopes and chained templates, with idempotent restart.
  - The proposer is multi-start pattern search with edge-following. Over a
    40-seed sweep, the worst gap moved 27.7 % → 11.7 %, the median 5.3 % → 0 %,
    and exact hits 6 → 34/40.
  - External candidates are accepted over MCP.
- **Realism data:**
  - 13 materials and 6 pilot processes (`materials-and-cost.md`).
  - ETSI EN 300 019-1-4 classes and a finishes table (`design-conditions.md`).
  - All values are cited; missing data stays missing.
- **Enclosure thermal** (`enclosure-thermal.md`):
  - General-solid Gmsh meshing; steady conduction with film convection and an
    internal air node.
  - Power sweep by superposition, with a third verification solve within 0.5 K.
  - Conservative bounds: window solar deposited into the cavity, linearized
    exterior radiation, and an orthotropic lower-bound board.
  - A cavity radiation bound sits behind a strict postcondition and is left off
    in the pilot.
- **Correlation and calibration:** `correlation.md`.
- **GPU** (`gpu-backends.md`, `host-authority.md`):
  - Wetting has a HIP path, implemented but unqualified.
  - Freezing and retained cooling refuse GPU with a typed CPU-only reason.
  - Render and media run under single-node DRM policies.
  - HIP compute with a second KFD GPU requires a kernel-bound exclusion receipt.
- **MCP:** 71 tools across 28 frameworks.

Reference-solver ablation for the aluminium/white candidate. These are
**pure-Python reference solver results on a simplified analogue, not CalculiX**:

| Credit | With window | Opaque lid |
| --- | --- | --- |
| none | none (air 373.6 K at 1 W) | 2.04 W |
| linearized radiation | 10.11 W | 10.77 W |
| orthotropic board | none | 2.08 W |
| both (pilot) | 10.42 W | 11.10 W |

Latest source gate on `7741f2c` (`scripts/check_cpu.py`, ambient toolchain):
288 Rust and 300 Python tests passed, with formatting, Ruff and Clippy clean.
Hosted CI is green for PR #1. No Nix build, no native qualification, no A0–F
promotion.

## Phase 3: after the cache-security disposition

Use bounded jobs. A failed, cancelled or skipped gate is not a pass. Keep failed
attempts.

```text
1. Cache-pin wiring (harbor-cad PR with CI):
   - add nix-cache-pin (flake = false, a69647e) and harbor-rs >= 29073b5
   - set cachePinMeta = rust.mkCachePin { … }, with lockOnly = false
   - expose legacyPackages.<system>.cachePinTargets
   - extend the policy check: caches == ["https://cache.nixos.org"]
   then: nix flake lock --option substitute false; nix eval .#cachePinMeta --json
2. canix repo update cache-pin nixpkgs                 # newest fully cached revision; keep the report
3. canix cache binary build .#<pkg> for default, mcp, python-test-environment,
   runtime-{cpu,hip}, openlb-hip, filter-hip, runtime-filter-hip,
   wetting-native-{cpu,hip}, runtime-wetting-worker-hip, runtime-cad-mesh-worker,
   runtime-fem-worker, runtime-fem-imported-worker, runtime-thermal-worker
   (CalculiX), visualization, media, gui; retain GC roots for qualified outputs
4. scripts/check_cpu.py in the pinned devShell; verify_native_hip, verify_openlb_hip,
   verify_filter_hip, verify_hip_sandbox, verify_wetting_hip,
   verify_cad_variant_worker, verify_study_worker, verify_egl_fields, verify_vaapi;
   verify_enclosure_thermal_cpu.py --ccx PATH --ccx-version V (first CalculiX run)
5. Demo loop (examples/design-enclosure-demo.json, labelled SYNTHETIC), then the
   real loop (examples/design-enclosure.json: ETSI 4.1, cited finishes and
   defaults, swept power, CNC 6061 / 5052 sheet / PA12, at most 32 evaluations).
   Export the winner with cost range, supported power, margins, evidence labels
   and the copper-plane requirement.
```

## Phase 4: iGPU (the operator authorizes the host step)

A Canix opt-in `igpu-amdgpu` boot entry exists (not activated). The default boot
is unchanged.

```text
1. From a clean, deployable Canix trunk: canix host rebuild boot atlas, then
   reboot into the igpu-amdgpu entry (fallback: the default entry).
2. scripts/verify_kfd_exclusion.py --selected-pci 0000:03:00.0 --foreign-pci 0000:7d:00.0 …
   then install the receipt with `harbor-cad authority install`.
   Run iGPU verify_egl_fields / verify_vaapi, and rerun verify_native_hip.
3. B1-style concurrent run: render/media on the iGPU, OpenLB-HIP on the dGPU.
```

## Phase 5: closure

A fresh independent agent reconciles A0–F. Refresh this handoff, the ledger and
`tasks/`.

## Limits to preserve

- **Board:** the 8.1 W/(m·K) in-plane bound needs a continuous copper plane.
  Without one, use the isotropic 0.26 W/(m·K), and the supported-power figures
  no longer hold.
- **Defaults and classes:**
  - The 85 °C limit and the board conductivities are cited defaults, not product
    data.
  - ETSI classes are design severities, not site weather.
  - pc-uv uses the conservative cavity-deposit bound.
  - PA12 cannot pass the 10 K cold margin (7 K).
- **Solver:** supported-power figures come from the reference-solver analogue.
  **CalculiX has never run** for this recipe.
- **Proposer and geometry:** the proposer's residual worst gap is 11.7 %. The
  `min_wall_m` probe samples one point per face. Ray-based exterior-face
  selection is unqualified.
- **Unqualified:** CUDA; physical and product validation for every recipe.
- **Preserved failures:** coarse enthalpy `0.0232` and uniform Stefan `0.0375`,
  both against the `0.02` gate.
- **GPU policy:** HIP/ROCm first and CUDA best-effort, with no invented Vulkan
  support and no silent CPU or other-GPU fallback. Modified PaStiX/PaRSEC remains
  inspection-only.

Remaining spec work: hybrid FEM/fill-in estimates, format/topology/ghost round
trips, GPU spectral transport, logout/reboot and solver checkpoint/resume,
physical and product validation.

Authoritative entry points: `docs/implementation-spec.md` (§10),
`docs/acceptance-ledger.md`, `tasks/plan.md`, `tasks/todo.md`.
