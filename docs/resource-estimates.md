# Admission estimate validation

Resource declarations are part of the immutable approved plan. Both generated
and hand-built plans must meet operation-specific conservative minima before
submission and launch. Estimates can be increased explicitly; an insufficient
estimate rejects the plan and preserves its scientific parameters.

The current estimator applies to the supported fixed adapters:

| Operation              | Conservative allowance                                                                                            |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Analytical channel     | 16 MiB RAM plus 160 bytes/sample; at least 1 MiB output or 128 bytes/sample                                       |
| CAD fixture/inspection | 1 GiB RAM and a 16 MiB native-output baseline                                                                     |
| Periodic OpenLB        | Padded whole-cuboid allocation `(nx+4)(ny+6)(nz+4)`; 2048 bytes/cell plus 64 MiB RAM; same allowance in GPU VRAM  |
| Native fields          | 96 bytes/allocated cell **for each retained time**, plus 32 MiB shared native overhead                            |
| EGL renderer           | At least 1 GiB RAM or 256 bytes/cell plus 32 bytes/pixel; 128 MiB VRAM plus 64 bytes/cell and 32 bytes/pixel      |
| PNG sequence           | Each approved frame reserves 8 bytes/pixel plus 1 MiB                                                             |
| VAAPI encoder          | At least 512 MiB RAM or 16 bytes/pixel; 128 MiB VRAM plus 16 bytes/pixel; 128 MiB output plus 4 bytes/pixel/frame |
| Bundle indexing        | 16 MiB RAM; streaming copies are already included in native staging admission                                     |

The lattice dimensions are computed from SI geometry and refinement using
checked allocation arithmetic. The padded allocation includes non-fluid and
halo cells rather than counting only fluid cells. The fixed driver retains
Float64 distributions and fields. Frame/disk allowances scale with **all**
approved times and full pixel dimensions. Native disk admission reserves two
copies of the complete allowance for work/verified staging. Overflow rejects
before allocation, submission or execution.

These are intentionally conservative admission estimates, not measured peaks
or a universal CAD-complexity bound. Imported assembly/tessellation complexity
can exceed an estimate and must still fail under effective RAM/output/time
limits. Future FEM fill-in, particle, JIT or independently imported field
topologies require their own estimators. VRAM estimates alone do not enforce
capacity/headroom or establish GPU qualification; authoritative aggregate
same-user admission and measured per-device accounting remain separate work.

Approval schemas and their serialization stay at version 1. Historical exports
keep the original record/digest; `current_plan_check` records any stricter
estimate rejection. An idempotent worker retry preserves an existing job's
binding instead of silently raising its budgets or replacing its approval.
