# GPU backend decision

## Decision (2026-10-05)

Prioritize **HIP/ROCm for OpenLB on AMD**, with **CUDA best-effort**. Keep the
independent **ParaView EGL** and **FFmpeg VAAPI** stages. Vulkan is a useful
cross-vendor compute API, but is not an implemented OpenLB backend at our pin.
This decision follows the user's revised priority and upstream source inspection;
it is not a claim that HIP has beaten Vulkan in an equal-accuracy benchmark.
Exact source hashes, retained host observations and qualification status are in
[`evidence/gpu-backends.json`](evidence/gpu-backends.json).

## Evidence

| Question                                 | Finding and source                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Can the current solver use AMD GPUs?     | OpenLB `145cd54810b468f4b6fd3ed86b10644264841578` contains `GPU_HIP`, collision/streaming, communication and device memory implementations. See [`src/core/platform/platform.h`](https://gitlab.com/openlb/release/-/blob/145cd54810b468f4b6fd3ed86b10644264841578/src/core/platform/platform.h), [`src/core/platform/gpu/hip`](https://gitlab.com/openlb/release/-/tree/145cd54810b468f4b6fd3ed86b10644264841578/src/core/platform/gpu/hip), [`rules.mk`](https://gitlab.com/openlb/release/-/blob/145cd54810b468f4b6fd3ed86b10644264841578/rules.mk) and [`config/gpu_only_amd.mk`](https://gitlab.com/openlb/release/-/blob/145cd54810b468f4b6fd3ed86b10644264841578/config/gpu_only_amd.mk). |
| How mature is this backend?              | The [OpenLB 1.9 release announcement](https://www.openlb.net/news/openlb-release-1-9-available-for-download/) calls AMD support preliminary, with upstream tests on MI300A and RX 7800 XT. Those tests do not qualify our Float64 forced-channel driver, card or sandbox.                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Does the source support Vulkan?          | `Platform` lists CPU_SISD, CPU_SIMD, GPU_CUDA and GPU_HIP. Inspection of the pinned source and build rules found no Vulkan platform or build path. Adding one would require a new numerical implementation and its qualification, rather than a compiler switch.                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Does Vulkan run this science on any GPU? | Khronos documents [`shaderFloat64`](https://docs.vulkan.org/refpages/latest/refpages/source/VkPhysicalDeviceFeatures.html) as a queried feature. A Vulkan-capable card does not by itself guarantee the Float64 shaders required here. Feature support, numerical behavior, device identity and actual kernels still need testing.                                                                                                                                                                                                                                                                                                                                                               |
| Is Vulkan almost as fast as ROCm?        | No same-model, same-card, same-precision Vulkan/HIP comparison was established for this OpenLB pipeline. Performance depends on implementation, memory layout, synchronization, transfers and FP64 throughput. Keep this an unmeasured hypothesis; do not use LLM-inference comparisons as CFD evidence.                                                                                                                                                                                                                                                                                                                                                                                         |
| Can scientific filters reuse ROCm?       | Viskores source snapshot [`6fddd7edee9aea5f0388672b7f817960dc3429fb`](https://github.com/Viskores/viskores/blob/6fddd7edee9aea5f0388672b7f817960dc3429fb/CMake/ViskoresDeviceAdapters.cmake) supports HIP through **Kokkos**, with a separately selected HIP toolchain. This is research evidence, not a new dependency pin or qualification of the bundled ParaView filter path.                                                                                                                                                                                                                                                                                                                |
| Does AMD list the card family?           | AMD's [ROCm 7.2.1 Radeon Linux matrix](https://rocm.docs.amd.com/projects/radeon-ryzen/en/latest/docs/compatibility/compatibilityrad/native_linux/native_linux_compatibility.html) lists RX 7900 XTX/XT/GRE. Its OS list is Ubuntu/RHEL, not NixOS; hardware listing and upstream NixOS tests do not replace testing our pinned Nixpkgs ROCm stack.                                                                                                                                                                                                                                                                                                                                              |

The upstream AMD example has an inconsistent comment/architecture (`RX 7800 XT`
but `gfx942`) and uses Float32. Our package must set an explicit architecture and
Float64; it must not copy those defaults or upstream `HIP_ARCH=native` behavior.

## Host observations

Read-only PCI/KFD inspection on 2026-10-05 established:

- `0000:03:00.0`: AMD Navi 31, PCI device `1002:744c`, bound to `amdgpu`,
  25,753,026,560 bytes reported total VRAM. The PCI name covers multiple SKUs;
  it does not uniquely identify the retail model.
- KFD node 1: `domain=0`, `location_id=768`, `gfx_target_version=110000`,
  `drm_render_minor=128`. These correlate to PCI `0000:03:00.0`, `gfx1100`,
  and `/dev/dri/renderD128`. `/dev/kfd` exists.
- `0000:7d:00.0`: integrated AMD GPU bound to `vfio-pci`; it is not an available
  AMD compute/render candidate in this host configuration.
- `hipcc`, `rocminfo`, `vulkaninfo`, `amd-smi` and `rocm-smi` were absent from
  the inspected PATH. Package-local tools must establish actual runtime support.

KFD is a shared device. Device visibility variables alone are not isolation.
Production admission needs verified PCI/UUID/KFD/render-node correlation and a
tested selected-device mount policy before HIP worker execution is enabled.

## Native qualification

The isolated `openlb-hip` package now compiles and links for `gfx1100` against
ROCm 7.2.3. It requires explicit PCI/UUID selection, rejects architecture
spoofing, and verifies HIP block assignment and kernel completion. The clean
HIP inventory identifies the card as **AMD Radeon RX 7900 XTX**; its UUID uses
the documented adapter encoding of the 16 bytes returned by `hipDeviceGetUuid`.

The Float64 periodic forced channel passed the 0.05 analytical-velocity gate at
resolutions 8/16, with relative L2 errors `0.011124428266122175` and
`0.0027813873418746048`. At every retained time, CPU/HIP velocity differences
were below `1.05e-13` relative L2 (acceptance `1e-10`) and material IDs matched
exactly. Pressure differences are retained diagnostics, not an accuracy claim.
The trusted numerical probe used a bounded service and the production shared
physical-card reservation helper. Full details are in
[`qualification.md`](qualification.md#actual-openlb-hip-reference).

Two regressions were repaired without changing scientific inputs or tolerances:
ROCm 22's binary offload output during dependency scanning, and per-step host
uploads that reset the GPU solution. Host-only dependency scanning preserves
GPU compilation/linking; resident stepping preserves the native solver state
between output observations.

The native result is a formulation/precision/card-specific qualification.
Selected-node sandbox compatibility and raw KFD VM acquisition on the running
kernel have since passed the scoped probes in
[`evidence/hip-sandbox.json`](evidence/hip-sandbox.json). Authority-bound worker
support uses `harbor-cad-native-hip-single-kfd-v1`, requires exact device and
compiled/runtime receipts, and rejects another exposed KFD GPU. The packaged
durable CLI/MCP B1 chain and HIP forced-death/cancellation recovery have now
passed on this card, including shared admission, unchanged retained fields,
independent EGL/VAAPI receipts and verified reservation/runtime-root release.
Exact evidence is in [`evidence/native-hip-b1.json`](evidence/native-hip-b1.json).
Multi-GPU exclusion, measured VRAM peaks, GPU filters, convergence and physical
validation still need their own gates.

## Implementation and acceptance

1. **Passed:** build the existing OpenLB HIP backend against the locked Nixpkgs
   ROCm stack, targeting `gfx1100` explicitly and retaining Float64/precision patches.
2. **Implemented/tested on the observed card:** require explicit HIP PCI + UUID,
   reject stale UUIDs, wrong backend, ambiguous stages and CPU fallback, and
   record runtime/compiler/device identities and kernel completion. The
   architecture-match guard is implemented; another architecture/card was not tested.
3. **Passed for the scoped fixture:** verify the synthetic periodic forced
   channel against the independent analytical reference and retained CPU fields.
4. **Passed for the observed single-KFD fixture:** authoritative admission,
   selected-node containment and durable CLI/MCP B1, with retained-field hashes,
   worker restart, cancellation and runtime/reservation release. Multi-GPU KFD
   isolation and physical-validation statuses remain separate.
5. Advance HIP/Kokkos numerical filters only against an exact compatible
   ParaView/VTK/Viskores set. EGL rendering and VAAPI media retain their own
   selected devices and receipts.

Version-1 approvals retain their serialization. B1 planning now accepts explicit
`hip` or `cuda` compute selections; `rocm` is not a backend alias and `vulkan`
cannot silently select another solver. Existing historical execution bindings
continue to name their original packages and policy.
