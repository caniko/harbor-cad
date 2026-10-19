# Modified PaStiX / PaRSEC readiness

`backend list` and every MCP profile's `backend_list`/`doctor` expose **source
candidate inspection only** for `pastix4calculix` and `parsec`. Neither is
packaged, integrated or executable by this worker. Resource estimates are `null`,
not zero; build, runtime, numerical and physical qualification remain separate.
The existing CPU CalculiX references use SPOOLES and their LP64 closure is not a
substitute for the candidate ILP64 stack.

## Source-backed compatibility findings

These are inspected candidates, **not selected runtime dependencies**:

| Component | Immutable source / observation | Consequence |
|---|---|---|
| PaStiX4CalculiX | [`d6b493bc431ecf860b3578a2ee6a25f392eb44be`](https://github.com/Dhondtguido/PaStiX4CalculiX/tree/d6b493bc431ecf860b3578a2ee6a25f392eb44be), CMake version 6.0.1 | Retain this modified stack; a generic current PaStiX package is not a verified replacement. |
| Patched PaRSEC | `pastix-6.0.1` resolves to `b580d208094e6cb4106e41d5eeec84f304a090b8` in the [repository required by PaStiX](https://bitbucket.org/mfaverge/parsec/) | Tag/source availability does not establish CalculiX compatibility. |
| CalculiX 2.23 | [Official installation instructions](https://www.dhondt.de/ccx_2.23.README.INSTALL), lines 77–126 in the retained snapshot | Modified PaStiX requires an eight-byte-integer stack and the separate `Makefile_i8`/`-DPASTIX` build. The current LP64 BLAS/SPOOLES build cannot be relabelled. |

The pinned PaStiX [CMake source](https://github.com/Dhondtguido/PaStiX4CalculiX/blob/d6b493bc431ecf860b3578a2ee6a25f392eb44be/CMakeLists.txt#L48-L51)
unconditionally adds `-march=native` and site-specific CUDA include paths. Its
[README](https://github.com/Dhondtguido/PaStiX4CalculiX/blob/d6b493bc431ecf860b3578a2ee6a25f392eb44be/README.md#L62-L93)
requires sequential BLAS/LAPACKE, matching ordering-library integer types and
patched PaRSEC with development headers. Packaging must repair those exact
reproducibility assumptions locally, pin the full compatible graph and verify
integer/BLAS/compiler/MPI/thread ABIs before execution is offered.

The candidate PaRSEC [CMake source](https://api.bitbucket.org/2.0/repositories/mfaverge/parsec/src/b580d208094e6cb4106e41d5eeec84f304a090b8/CMakeLists.txt)
reports version 2.0.0, defaults CUDA and home-configuration lookup on, and labels
its OpenCL option unsupported. Development-header, home-config, thread/MPI and
device policies must be explicit in a future isolated package. Neither inspected
candidate establishes a Vulkan or HIP factorization path. The official CalculiX
instructions identify its GPU path as NVIDIA/CUDA-only; CUDA remains best-effort
under the current priority, not an excuse to replace the independent CPU reference
or claim ROCm support.

## Admission boundary

No new solver stage, backend selector, source patch, Nix dependency, installer or
host change is introduced here. Requests to the existing FEM planner with
`backend=cuda|hip|vulkan` or injected `factorization=pastix|parsec` refuse through
the same strict Rust contract on CLI and MCP. Planning the explicit CPU reference
still works and never dispatches implicitly.

An executable hybrid workflow needs its own compatible closure, supported matrix
and factorization, symbolic fill-in/workspace/staging bounds, explicit integer and
precision contracts, independently approved device/resource binding, native
sandbox/JIT checks, CPU agreement and measured equal-accuracy evidence. Upstream
speedups are not Harbor-CAD measurements or a universal memory requirement.

Retained upstream file hashes and scoped test/package outcomes are recorded in
the [dependency manifest](dependency-manifest.json) and acceptance ledger. This
inspection does not qualify any existing or future native job.
