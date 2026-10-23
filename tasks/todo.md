# Completion tasks

See [plan.md](plan.md) for decisions, the phase order and boundaries. Each task
counts as done only with implementation, focused tests, MCP coverage through the
shared worker and source-bound evidence. Native evidence is tracked separately.
Do not weaken scientific gates, bypass hooks or leases, or change hosts outside
the operator-assisted cache scope.

## Workstreams — source level (done at `e271596`)

- [x] W1 material/process library, estimate-only cost and screens; `materials list|show`, `cost estimate`, MCP `materials_list`/`cost_estimate`.
- [x] W2 spreadsheet-alias CAD variants (v2) and the synthetic `enclosure_fixture` seed. Only a FreeCAD 1.1.3 development smoke, as reported by the coordinator. The packaged gate needs FreeCAD ≥1.1.4.
- [x] W3 bounded design loop: `design init|validate|plan|submit|propose|status|export` and five MCP tools.
- [x] W4 `results correlate` / MCP `results_correlate`.
- [x] W5 harbor-rs PR #20 merged (`29073b5`, `lib.mkCachePin`, nix-cache-pin `0.3.0` = `a69647e`); consumer patch prepared.
- [x] W7 wetting HIP implemented (unqualified); typed CPU-only refusal for freezing/retained cooling; render/compute routing documented.
- [x] W8 chained templates, manufacturing providers, calibration and enclosure example envelopes.
- [x] W9 general-solid mesh and steady enclosure thermal; `verify_enclosure_thermal_cpu.py` 4/4 with the pure-Python reference solver.
- [x] Source gate (ambient Cargo 1.100, not pinned 1.94): treefmt clean, Clippy `-D warnings`, 262 Rust / 263 Python tests, Ruff; MCP matrix 69 tools.
- [x] Documentation close-out: spec §10, the 2026-10-23 ledger slice, and this plan.

## Phase 0 — cache security (operator-assisted)

- [ ] Operator runs `operator-unified.sh` (SHA-256 `4f5114e6…7d13e1d8`) and returns the evidence directory.
- [ ] Verify checksums, failed logs, generation/config hashes, the daemon drain/identity and effective policy: retired trust absent, `require-sigs=true`.
- [ ] Triage the running-system inventory. The retained harbor-cad outputs were garbage-collected, so do not rebuild them to fill the gap.
- [ ] Get the Canix security owner's disposition. Confirm the Nix-token rotation.

## Phase B — after disposition

- [ ] Apply the cache-pin patch; run `nix flake lock --option substitute false` (user approval).
- [ ] Run `nix eval .#cachePinMeta --json`; the policy check needs caches `["https://cache.nixos.org"]`.
- [ ] Run `canix repo update cache-pin nixpkgs` to move to the newest fully cached revision, and record the report.
- [ ] Run `canix cache binary build` for `default`, `mcp`, `python-test-environment`, `openlb-hip`, `wetting-native-{cpu,hip}`, `runtime-wetting-worker-hip`, `filter-hip`, the FEM/CAD-mesh runtimes and `gui`.
- [ ] Run `scripts/check_cpu.py` in the pinned devShell.
- [ ] Run the native gates `verify_native_hip`, `verify_openlb_hip`, `verify_filter_hip`, `verify_hip_sandbox`, `verify_cad_variant_worker`, `verify_study_worker`, `verify_wetting_hip` and `verify_enclosure_thermal_cpu.py --ccx`.
- [ ] Run the enclosure demo loop (`examples/design-enclosure-demo.json`) with inputs labelled synthetic.
- [ ] Do the independent A0–F closure review. Preserve the coarse enthalpy/Stefan failures, missing physical inputs and the Paperclip audit.

## Open product inputs and limits

- [ ] Real PCB power, site ambient, exterior/internal film coefficients, solar irradiance, gap/internal coefficients and component limits are all missing.
- [ ] Source the pc-uv and PA12 solar absorptance.
- [ ] Add a surrogate proposer if the pattern search's 0–28 % excess over a 40-seed sweep matters. Improve the one-point-per-face `min_wall_m` screen. Qualify the ray-based exterior-face selection.
- [ ] Remaining spec work: hybrid FEM/fill-in estimates, multi-GPU KFD exclusion (needed for iGPU render with dGPU HIP compute), format/topology/ghost round trips, GPU spectral transport, logout/reboot and solver resume, hosted CI. CUDA is unqualified. Physical/product validation is unqualified for every recipe.

## Superseded role packets

The 16 role packets of revision 1 are superseded and were never dispatched. Their
completed items and evidence references are preserved at `e271596:tasks/todo.md`
and in the ledger. Examples: the read-only execution preflight at `8f77e2c`, the
61-tool matrix at `862af55`, build provenance at `1eaaf52`, retained cooling
`c8-30`/`c8-34`, the portable checks at `06ce6ec`, and factorization readiness at
`96563f4`.
