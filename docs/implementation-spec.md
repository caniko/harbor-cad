# harbor-cad — implementation specification

Target: `https://github.com/caniko/harbor-cad`. Implement the requirements below; this package contains no built software or measured host results. Use `references.md` to verify upstream APIs, security fixes and compatibility, then pin immutable dependencies.

## 1. Product and architecture

Deliver a local-first **Rust CLI + persistent Rust worker**, **Python MCP**, and **Nix flake** taking FreeCAD designs through environmental simulation, GPU analysis/rendering, hardware video encoding, and portable scientific export. First qualify x86_64 Linux/NixOS. Preserve source CAD and existing repository work.

```text
CLI / Python MCP → Rust worker → immutable execution DAG → native adapters
                                 ↑ Fleetix + runtime device resolution
FreeCAD → geometry/meshes → solvers → scientific fields → GPU filters → EGL → encoder
```

Rust owns contracts, validation, scheduling, resources, lifecycle and artifacts. Python owns narrow FreeCAD/ParaView/Mitsuba bridges and MCP. Native solvers own numerical algorithms; implement thin OpenLB C++ drivers. Use versioned descriptors and files, not cross-language embedding or duplicate schedulers.

| Stage                                          | Selected stack                               | Execution                                                                  |
| ---------------------------------------------- | -------------------------------------------- | -------------------------------------------------------------------------- |
| CAD / FEM mesh                                 | Security-patched FreeCAD; Gmsh               | CPU; optional GPU viewport                                                 |
| Airflow / local water / qualified phase change | Public OpenLB; model-specific drivers        | HIP/ROCm first on AMD; CUDA best-effort; each backend separately qualified |
| Thermal / mechanical / contact                 | CalculiX + compatible PaStiX4CalculiX/PaRSEC | Hybrid CUDA; independent CPU references                                    |
| Spectral exposure                              | libRadtran → matched Mitsuba/Dr.Jit          | CPU atmosphere; qualified CUDA/OptiX transport                             |
| Scientific filters                             | Compatible ParaView/VTK/Viskores             | Qualified GPU-filter allowlist                                             |
| Rendering / encoding                           | ParaView EGL → FFmpeg                        | Verified render and media devices                                          |
| Later optimization                             | Catalyst/Conduit; qualified VTKHDF           | Optional after baseline delivery                                           |

Do not add another mandatory physics framework, cloud service, network database or web dashboard. Public source availability and an exact model/backend benchmark—not announcements—determine support. [R04–R10]

The GPU priority follows the researched decision in `gpu-backends.md`. Vulkan
remains a candidate for workloads with an actual supported implementation; API
portability alone does not qualify an OpenLB solver or establish performance parity.

## 2. Nix and Harbor integration

Reuse `caniko/harbor-rs` toolchain/Crane/Cargo/cache helpers, `caniko/harbor-py` uv/pyproject helpers, and `caniko/fleetix`. Inspect pinned signatures and tests; do not reproduce their infrastructure or modify those repositories without authorization. [R01–R03]

**Fleetix:** consume the merged functionality of PR #3; verify exports and fixtures even after a squash merge. Align Cargo/Nix source identity and contract digest; test drift. Expected interfaces are `lib.gpu.routes`, `lib.gpu.forHost`, and `fleetix::gpu::pci_selector`; verify at the resolved pin. Verify that the selected dependency contains the required contracts.

**Packages:** expose lightweight `default` CLI, `worker`, `mcp`; separate native packages, `runtime-cpu`, optional `runtime-cuda`, qualified `runtime-hip`, headless `visualization`, and optional `gui`. Provide `doctor`, `qualify`, deterministic non-GPU checks, and opt-in NixOS/Home Manager modules. Tool discovery must neither import native applications nor require their closures.

**Isolation:** FreeCAD, ParaView, Mitsuba and MCP use individually compatible interpreters/runtimes. Launch exact packaged executables with controlled environments; no global Python, loader or Qt-path injection. Test imports and execution outside development shells.

**Locks:** commit `flake.lock`, `Cargo.lock`, `uv.lock`; record native source/patch hashes, compilers and Python/CUDA/MPI/BLAS/integer ABIs. Pin ParaView/VTK/Viskores and Mitsuba/Dr.Jit as compatible sets; retain CalculiX's compatible modified PaStiX stack. Declare GPU architectures; no build-time hardware discovery, undeclared downloads, unqualified fast-math, or hidden `-march=native`.

**Audit-sensitive defaults:** explicitly set effective `allowUnfree = false` plus narrowly approved GPU dependencies. Do not inherit Harbor profiles that obstruct Unix sockets, namespaces, JIT memory or devices. Do not substitute a RADV/Vulkan helper for CUDA/EGL/media routing or a system-service helper for a user service. Verify effective settings at the chosen pins. [R02–R03]

Separate immutable package caches, writable JIT caches and scientific artifacts; retain active runtime closures. Use `ci`, `prototype`, `production`, `research` policies over one implementation. Research results never become validated production outputs automatically. Test upgrades before promotion, including security fixes.

The flake does not control host kernel drivers. Verify loaded runtime libraries and exclude driver stubs. No automatic driver installation, permissions changes, service activation, user lingering, trusted-cache changes or uploads. Record licenses, notices and scoped non-libre dependencies; do not describe the CUDA runtime as fully libre. [R11–R12]

## 3. Fleetix and GPU execution

Route **operations**, not programs:

| Fleetix role | Work                                                |
| ------------ | --------------------------------------------------- |
| Compute      | OpenLB, PaStiX, Mitsuba, Viskores numerical filters |
| Render       | ParaView EGL; optional FreeCAD viewport             |
| Media        | FFmpeg encoding                                     |

Compute policy selects a backend, not a physical GPU or scheduler. Resolve PCI identity and backend UUIDs against actual hardware; ordinals are diagnostic only. A Mesa PCI selector is not a CUDA ID. Reject ambiguous/stale bindings; absent routes stay disabled unless an explicit recorded override applies. Qualify partitions separately. Split filter/render stages when they use different devices unless independent selection is proven. [R01]

Reserve RAM/VRAM and concurrency by physical device, sharing one budget across roles. Default to exclusive heavy compute, with headroom for external/display workloads. Recheck before launch; queue rather than silently changing mesh, precision or physics. Limit native thread counts to avoid nested oversubscription. Device reservations are admission policy, not universal hard VRAM quotas.

Each stage declares `required`, `preferred`, or `cpu_only`. Heavy supported workflows require actual GPU execution. Small CPU references and measured hybrid FEM dispatch are legitimate when selected before plan approval; never silently downgrade a required stage.

Record requested/observed device, runtime, precision, context/backend and execution evidence. Qualify actual kernels, numerical filters, EGL identity, and encode/decode—not just build flags or codec listings. Reject software graphics in required mode. Measure equal-accuracy end-to-end time, transfers, output bytes, peak RAM/VRAM and cold/warm JIT. Qualify multi-GPU separately; promise neither cross-vendor memory pooling nor zero-copy.

## 4. Contracts, CAD and applicability

Rust owns versioned schemas; generate JSON Schema and test Python parity.

| Contract                               | Required contents                                                                                      |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `CaseSpec`, `PhysicsApplicability`     | Geometry/material provenance, units, boundaries, formulation, validated ranges, exclusions, tolerances |
| `ExecutionPlan`, `TransferSpec`        | Immutable DAG, dependencies, resource estimates, quantity mappings and conservation/error policy       |
| `HostExecutionProfile`, `GpuSelection` | Allowed roots/devices/packages, budgets, routes, overrides, effective enforcement                      |
| `ObservationPlan`                      | Metrics/probes, retained fields/times, checkpoints, previews, budgets/backpressure                     |
| `ArtifactManifest`, `ValidationReport` | Hashes, formats, units/times, field association, provenance, evidence and limitations                  |
| `WorkerRequest`                        | Protocol version, request ID, typed operation, idempotency key                                         |

Adapters implement `capabilities`, `prepare`, `run`, `inspect`; checkpoint/resume only when verified. Keep reusable case parameters runtime-configurable where supported. Capabilities are keyed by **source + formulation + dimensionality + backend + precision + refinement + runtime/hardware**, not generic solver names.

Track source availability, build, runtime, numerical verification and physical validation independently. Process exit, convergence and validated engineering conclusions are different statuses.

Normalize explicit units to SI while retaining originals, assembly transforms and spectral units. Unknown material data, humidity, operating histories and acceptance limits stay unknown; synthetic fixtures must be labelled. Separate science, execution and presentation identities. Relevant geometry/material/numerical changes invalidate results; changing a camera does not.

Preserve originals read-only; edit copies through allowlisted parameters. Validate named regions using geometry and assembly context after every recompute. Reject ambiguous tags; do not trust face numbering or heal intended gaps away.

```text
FEM:      FreeCAD STEP/BREP → Gmsh physical groups → CalculiX
Fluid:    FreeCAD controlled STL/CSG/VTI → OpenLB lattice geometry
Spectral: FreeCAD material-tagged triangles → Mitsuba
```

Use whole-device flow/thermal models and local gap/wetting submodels. Estimate actual allocated distributions, halos, fields, factorization fill-in and staging—not just fluid-cell counts. Reject resolved-ingress claims for unresolved paths. Never alter density/viscosity ratios or numerical accuracy silently to obtain stability or fit VRAM.

## 5. Engineering recipes and boundaries

| Recipe                    | Implement and verify                                                                      | Never infer automatically                            |
| ------------------------- | ----------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| Cold soak → restart       | Transient thermal history, power, interfaces; component temperatures and heater energy    | Electronic boot reliability                          |
| Thermal expansion / seals | Preload, contact/gasket data, temperature transfer; displacement and residual compression | Sealing with missing cold-material data              |
| Wind / local water        | Wall-mounted airflow; qualified wetting/retention; mass balance and refinement            | Whole-device waterproofness from splash images       |
| Snow                      | Prescribed coverage, insulation and blocked openings                                      | Snow deposition or adhesion                          |
| Retained water → freeze   | Conservative state transfer; phase fractions, temperature, energy balance                 | Expansion pressure, fracture or freeze–thaw lifetime |
| Solar / UV                | Angular direct/diffuse inputs, surface irradiance and time-integrated dose                | Material lifetime without calibration                |

Start with one-way transfers. Record source/destination regions, units, orientation, interpolation and conservation losses. **Velocity alone does not supply a convection coefficient:** use documented measurements/correlations or validated thermal-flow data with heat flux and wall/reference temperatures; reject ill-conditioned conversions.

Every combined cold/wet report requires a moisture-risk entry: supported screening, justified inapplicability, or missing inputs. Dew-point screening is not condensate-mass or moisture-transport simulation.

Radiation tests cover planar irradiance, orientation, occlusion, reflection and spectral-unit conversion. Preserve UV optical properties, sensor-area semantics and atmospheric angular information. Integrate dose with justified temporal sampling; distinguish absorbed heating from ageing-weighted exposure. Numeric benchmarks and CPU/GPU agreement do not replace prototype validation. [R05–R07]

## 6. Worker and security

One Rust worker serves a local Unix socket, durable SQLite state and event history. Submit immutable approved plans with idempotency keys; disconnect must not kill or duplicate jobs. Use tracked transient systemd user services/cgroups on qualified NixOS hosts; persist unit/invocation identities, durable exit records and restart reconciliation. Do not trust saved PIDs or attach every job's lifetime to the worker.

Cancel only owned complete process trees; use bounded termination/escalation. Checkpoint only through verified adapters. Distinguish worker restart, logout, machine reboot and solver resume; label weaker foreground-mode guarantees. Enforce effective CPU/RAM/process controls and disk budgets; monitor VRAM with conservative admission. Do not kill unrelated applications or alter GPU clocks. [R12]

Open documents only in a **security-patched FreeCAD import sandbox**, before inspection. Check the selected source against upstream security advisories; disabling macros is not a substitute for import isolation. [R04]

Use separate tested profiles for worker, importer, solver, JIT and visualization. Grant minimal mounts, selected devices and scratch/cache paths; deny unnecessary network, home, credentials and session access. Allow required namespace/JIT operations only where needed. Native payloads must not inherit the worker socket or host D-Bus. GPU visibility variables are not isolation. Fail production isolation qualification rather than broadening privileges silently. [R12]

Use typed allowlisted operations, argument arrays and traversal/symlink-safe paths. Treat imported metadata as data; no arbitrary shell, Python evaluation, macros, dependency installation or host changes through MCP.

## 7. Observations, analysis and exports

Specify metrics/probes, full-field snapshots, restart checkpoints and presentation cadence independently. Numerical time steps do not determine output frequency. Use qualified on-device reductions; budget all native/derived copies. Scientific-output congestion must backpressure, checkpoint or fail; only explicitly labelled previews may drop frames.

Retain authoritative native scientific data and verified restart state. Use VTK XML/time collections as baseline; qualify VTKHDF per dataset/time/partition and actual reader/writer. Export Parquet/CSV metrics and JSON provenance; Zarr is optional, not another mandatory copy. Round-trip precision, topology, IDs, ghost cells, units, coordinates and node/cell association. Use lossless numeric compression. [R09]

Write committed artifacts atomically with checksums; isolate incomplete output. Prefer closed shards initially over unqualified concurrent HDF5 access. Deduplicate immutable data safely; never hardlink mutable run files or silently discard sole source/checkpoint artifacts.

Qualify Viskores filters numerically and on the compute device; EGL rendering and media encoding require independent evidence. Use bounded frame pipes/staging, fixed comparison scales, units and physical-time labels. Record interpolation, dropped previews, deformation exaggeration, codec/pixel format and frame/timestamp correctness. Scientific arrays remain independent of lossy video. [R08, R10]

Offline bundles include relative paths, selected numerical data, case/material/plan provenance, exact package/source/runtime identities, device receipts, validation and render recipes. Record omitted private geometry; no uploads. Re-render without solving **only for retained fields/times**.

Add Catalyst/Conduit only after the baseline works. Start with bounded observations; verify buffer ownership, synchronization, transfers and downstream device support. Keep it only when end-to-end measurements justify it. [R09]

## 8. CLI and Python MCP

Provide stable JSON output and typed errors for these proposed command groups:

```text
harbor-cad doctor | backend list
harbor-cad cad inspect|regions|export|variant
harbor-cad case init|validate|plan
harbor-cad job submit|status|logs|cancel
harbor-cad results describe|sample|compare
harbor-cad render | video | artifact list|export
harbor-cad-mcp --profile cad|simulation|results|all
```

Use one Python MCP distribution and the official SDK; verify and test the API at the selected immutable pin. Profiles call the same Rust worker. Long operations return durable job IDs independently of protocol task-extension support. Default to stdio, stderr logs, bounded structured results and resources; arrays stay in artifacts. Test with a real MCP client. [R13]

## 9. Delivery gates and acceptance

| Gate   | Required result                                                                                                                              |
| ------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
| A0     | Locked clean-runtime packages; patched importer; effective sandbox/unfree policy; Fleetix/schema parity; worker/MCP contracts                |
| A1     | Small airflow, thermal-boundary, wetting-feasibility and FEM references; identify unsupported combinations without blocking B1 unnecessarily |
| **B1** | **Synthetic FreeCAD → real GPU OpenLB → fields → EGL image → hardware video → bundle, through CLI and MCP**                                  |
| B2     | One verified GPU filter; bounded observations; format round trips; measured resource peaks                                                   |
| C      | Cold start, expansion/contact, justified thermal coupling, hybrid FEM and moisture assessment                                                |
| D      | Local water, prescribed snow and qualified freezing with applicability, conservation and refinement                                          |
| E      | Spectral surface irradiance/dose, analytical references and GPU evidence                                                                     |
| F      | Optional Catalyst; bounded studies; retention/recovery; measured equal-accuracy optimization                                                 |

Finish vertical slices, not placeholder adapters. One owner controls schemas/locks; parallelize adapters after contracts stabilize.

CI covers formatting/lints, builds, schemas, CPU references, applicability rejection, units, source immutability, paths and export round trips. Lifecycle tests cover idempotency, disconnect, worker crash, cancellation, partial writes and disk exhaustion. Hardware qualification independently tests mixed/ambiguous routes, actual execution, JIT/sandbox compatibility and rejected software fallback. Never run untrusted PR code on trusted GPU hosts carrying secrets/private models.

Unavailable hardware is **unqualified**, not passed or a reason to stop unblocked work. Handoffs report changed files, immutable pins, exact commands/outcomes, artifact paths, device evidence, unsupported combinations and missing physical inputs. B1 completes integration, not environmental certification; engineering completion requires each recipe's evidence gates.
