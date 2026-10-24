# Completion tasks

See [plan.md](plan.md) for decisions, the phase order and boundaries. Each task
counts as done only with implementation, focused tests, MCP coverage through the
shared worker and source-bound evidence. Native evidence is tracked separately.
Do not weaken scientific gates, bypass hooks or leases, or change hosts outside
the operator-assisted cache scope and the user-authorized Phase 4 boot step.

## Workstreams — source level (done at `f738aed`)

- [x] W1 material/process library, estimate-only cost and screens; `materials list|show`, `cost estimate`, MCP `materials_list`/`cost_estimate`.
- [x] W2 spreadsheet-alias CAD variants (v2) and the synthetic `enclosure_fixture` seed. Only a FreeCAD 1.1.3 development smoke, as reported by the coordinator. The packaged gate needs FreeCAD ≥1.1.4.
- [x] W3 bounded design loop: `design init|validate|plan|submit|propose|status|export` and five MCP tools.
- [x] W4 `results correlate` / MCP `results_correlate`.
- [x] W5 harbor-rs PR #20 merged (`29073b5`, `lib.mkCachePin`, nix-cache-pin `0.3.0` = `a69647e`); consumer patch prepared.
- [x] W7 wetting HIP implemented (unqualified); typed CPU-only refusal for freezing/retained cooling; render/compute routing documented.
- [x] W8 chained templates, manufacturing providers, calibration and enclosure example envelopes.
- [x] W9 general-solid mesh and steady enclosure thermal; `verify_enclosure_thermal_cpu.py` 4/4 with the pure-Python reference solver.
- [x] W10 documentation close-out of W1–W9 (`7034bdf`).
- [x] W11 ETSI EN 300 019-1-4 classes, film ranges, finishes and `sourced` metrics; the pilot envelope uses the class 4.1 hot/cold points.
- [x] W11b conservative window solar bound, compatible `MATERIAL__PROCESS__FINISH` triples, sourced cavity film, component margin from the hotter of board and internal air.
- [x] W12 MCP `cad_plan_solid_mesh`/`enclosure_thermal_plan` (69 → 71 tools); render/media single-node DRM policies (native run pending).
- [x] W13a KFD multi-GPU exclusion receipt gate and `scripts/verify_kfd_exclusion.py` (mocked-ioctl tests only; not run).
- [x] W13b Canix `5df6b230d` opt-in `igpu-amdgpu` boot entry (local, not activated; default boot proven identical by evaluation).
- [x] W14 PCB power sweep (1 W/15 W superposition), `max_supported_pcb_power_w`, cost vs supported-power Pareto, cited 85 °C and FR-4 defaults.
- [x] W15 linearized exterior radiation and orthotropic board lower bounds; reference-solver ablation (window none → 10.42 W, opaque lid 2.04 → 11.10 W).
- [x] W16 multi-start pattern search with edge-following (40-seed sweep: worst 27.7 % → 11.7 %, median 5.3 % → 0 %, exact 6 → 34/40).
- [x] W17 cavity radiation lower bound with a strict postcondition; it fails on the analogue, so the pilot leaves it off.
- [x] Source gate at `f738aed` (ambient Cargo 1.100, not pinned 1.94): treefmt clean, Clippy `-D warnings`, 284 Rust / 300 Python tests, Ruff; both enclosure examples validate; MCP matrix 71 tools.
- [x] W18 documentation refresh: spec §10, the 2026-10-24 ledger slice, this plan and the handoff.

## Phase 0 — cache security (operator-assisted)

- History: v1 `operator-unified.sh` was refused at root-plan line 8 (`operator-Eke6ZHo4`, sudo exports `XDG_CONFIG_DIRS`); host unchanged.
- [ ] Operator runs `operator-unified-v2.sh` (SHA-256 `acb15b70…0e9e044`) in a quiet window (`pgrep -c -x nix-daemon` = 1). The first v2 attempt `operator-by6AgK4k` was refused at the quiet-window guard; host unchanged.
- [ ] Verify checksums, failed logs, generation/config hashes, the daemon drain/identity and effective policy: retired trust absent, `require-sigs=true`.
- [ ] Triage the running-system inventory and the `lantian:` provenance (also signed by `cache.nixos.org-1`, or a local build; read-only NAR hash comparison). Any path signed only by `lantian` stops the work.
- [ ] Record the pre-authorized disposition only if every check passes. Confirm the Nix-token rotation.

## Phase 3 — after disposition

- [ ] Apply the cache-pin patch; run `nix flake lock --option substitute false` (user approval).
- [ ] Run `nix eval .#cachePinMeta --json`; the policy check needs caches `["https://cache.nixos.org"]`.
- [ ] Run `canix repo update cache-pin nixpkgs` to move to the newest fully cached revision, and record the report.
- [ ] Run `canix cache binary build` for `default`, `mcp`, `python-test-environment`, `runtime-{cpu,hip}`, `openlb-hip`, `filter-hip`, `wetting-native-{cpu,hip}`, `runtime-wetting-worker-hip`, the CAD/mesh/FEM/thermal runtimes (CalculiX) and `gui`; GC-root each qualified output.
- [ ] Run `scripts/check_cpu.py` in the pinned devShell.
- [ ] Run the native gates `verify_native_hip`, `verify_openlb_hip`, `verify_filter_hip`, `verify_hip_sandbox`, `verify_wetting_hip`, `verify_cad_variant_worker`, `verify_study_worker`, `verify_egl_fields`/`verify_vaapi` (DRM policies) and `verify_enclosure_thermal_cpu.py --ccx`.
- [ ] Run the enclosure demo loop (`examples/design-enclosure-demo.json`, inputs labelled synthetic), then the real loop (`examples/design-enclosure.json`) and export the winner with the copper-plane board requirement.

## Phase 4 — iGPU activation (user authorizes the host step)

- [ ] `canix host rebuild boot atlas` from a clean deployable Canix trunk containing `5df6b230d`; reboot into `igpu-amdgpu` (fallback: the default entry). Check Immich ROCm ML.
- [ ] Run `verify_kfd_exclusion.py` and install the receipt via `authority install`; run iGPU `verify_egl_fields`/`verify_vaapi`; rerun `verify_native_hip`.
- [ ] Run B1 concurrently: iGPU render/media with dGPU OpenLB-HIP.

## Phase 5 — closure

- [ ] Do the independent A0–F closure review. Preserve the coarse enthalpy/Stefan failures, missing physical inputs and the Paperclip audit.
- [ ] Refresh the handoff, the ledger and `tasks/`. Push only with user authorization.

## Open product inputs and limits

- [ ] Replace the cited defaults with product data: the limiting part's temperature limit and the real board stack-up. The in-plane 8.1 W/(m·K) bound requires at least one continuous copper plane, which the operator confirmed for the pilot PCB. The 0.15 board emissivity, used only by the cavity bound, is also a board requirement.
- [ ] ETSI classes are design severities, not site weather. A real site needs its own cited class. Gap/interface conduction stays uncredited.
- [ ] pc-uv glazing has no sourced solar transmittance/absorptance split, so it uses the conservative cavity-deposit bound. `pigment-white-polymer` optics are missing.
- [ ] Supported-power figures so far come from the pure-Python reference analogue, not CalculiX.
- [ ] Proposer residual: worst 11.7 % gap over the 40-seed sweep, with a surrogate as the upgrade path. The `min_wall_m` one-point-per-face screen and the ray-based exterior-face selection are unqualified.
- [ ] Remaining spec work: hybrid FEM/fill-in estimates, format/topology/ghost round trips, GPU spectral transport, logout/reboot and solver resume, hosted CI. CUDA is unqualified. Physical/product validation is unqualified for every recipe.

## Superseded role packets

The 16 role packets of revision 1 are superseded and were never dispatched. Their
completed items and evidence references are preserved at `e271596:tasks/todo.md`
and in the ledger. Examples: the read-only execution preflight at `8f77e2c`, the
61-tool matrix at `862af55`, build provenance at `1eaaf52`, retained cooling
`c8-30`/`c8-34`, the portable checks at `06ce6ec`, and factorization readiness at
`96563f4`.
