# Planar wetting reference

Original source-bound phase/velocity retention with explicit conservative
extrusion is described in [retained wetting distribution](retained-wetting.md).

The standalone CPU reference uses OpenLB 1.9.0 at
`145cd54810b468f4b6fd3ed86b10644264841578`, with its two D2Q9 lattices,
well-balanced Cahn–Hilliard coupling and interpolated wetting walls from
[`contactAngle2d.cpp`](https://gitlab.com/openlb/release/-/blob/145cd54810b468f4b6fd3ed86b10644264841578/examples/multiComponent/contactAngle2d/contactAngle2d.cpp).
The upstream three-point estimator has an incorrect coordinate assignment in
its vertical interpolation. Harbor retains complete native phase/velocity and
material fields instead, then independently fits the entire `phi=0.5` contour.

## Scientific contract

This reference is explicitly synthetic and two-dimensional, with equal liquid
and vapor density/kinematic viscosity, constant properties, periodic x and
impermeable planar y walls, a prescribed uniform contact angle and no gravity.
SI diameter, diffuse-interface thickness, density, kinematic viscosity and
surface tension are explicit with material and boundary provenance. The fixed
Navier–Stokes relaxation time is 1; the phase relaxation time is explicit.
`initial_center_above_wall_m=0` explicitly initializes a wall-centered half-circle.
The native source patch changes only the upstream initializer's center from
`y=1` to the Bouzidi wall at `y=0.5`. The original initializer placed the center
`dx/2` above the wall, changing the initial physical geometry and phase area
across refinements; the failed native-4 campaign retains those original bytes.
Rust owns the strict `WettingReferenceSpec` schema and matching SI/bounds checks;
Python parity tests cover the same native descriptor.

For `dx = diameter / resolution`, the pinned converter gives
`dt = (tau - 0.5) dx² / (3 nu)`. Surface tension in lattice units is
`sigma dt² / (rho dx³)`. The wall lies at `y=dx/2`. The physical interface
thickness is held fixed across refinements. Native sampling is in SI world
coordinates; raw lattice velocity remains labelled as lattice velocity.

The supported bounds are n24–96, 100–800000 steps, at most 32 observations,
60–120° contact angle, at least three cells across the interface and lattice
surface tension no greater than 0.02. Descriptors require distinct initial and
final observations. Water–air property ratios and additional capability fields
reject before solving.

## Verification and isolation

`wetting-native-7` passes six solves, twelve rejection cases and both 90°/100°
refinement sequences at the explicit extended settling duration. Maximum phase
mass error is `0.000797298811`, with all late-angle changes at most `0.2°`.
The 90° errors are `0.592696494 → 0.036307450 → 0.033501707°`; the 100° errors are
`1.032076235 → 0.285973641 → 0.208032116°` at n24/36/48. Exact identities and
original report hashes are retained in [native evidence](evidence/wetting-native-cpu.json).

The first packaged worker attempt fails before solving: its preopened
`wetting.log` conflicts with the adapter's empty-directory guard. Revision
`8ee49c2` admits only this empty, regular, single-link worker log and retains
rejection of stale scientific output and aliased logs. Build 35 repeats the full
native gate as `wetting-native-8`, with the same six solves/twelve rejections and
both decreasing-error sequences. `wetting-worker-2` then exposes the C++ driver's
independent directory guard, which also rejects the preopened worker log before
solving. The original `process.log` records this earliest causal error.

Repair `7fb436c` uses the actual native filesystem guard in a hardware-independent
C++ test: standalone and worker layouts pass, while stale scientific output,
nonempty worker logs, aliased logs, directories and symlinks reject. The native
driver accepts only distinct regular `process.log` plus an empty regular
single-link `wetting.log`. Native subprocess failure now retains and reports its
process log before attempting to read a nonexistent receipt. Build 40 repeats
the exact native six-solve gate, exercising standalone directories for 90° and
the worker directory layout for 100°, before the matching packaged worker gate.

`runtime-wetting-reference-cpu` supplies the exact adapter and operation closure.
The bridge requires `harbor-cad-wetting-cpu-v1`: closure-only read-only store,
no GPU nodes/sysfs/host home/session bus/worker socket/network access and a
read-only descriptor. The native library owns collision, streaming and wetting
coupling; the bridge only verifies inputs, launch identity and retained outputs.

Version-9 `case plan-wetting-reference INPUT.json` and simulation-profile MCP
`case_plan_wetting_reference` return a digest-bound wetting→bundle plan. Explicit
`job submit --approve DIGEST` uses the persistent worker and
`runtime-wetting-worker`, the same immutable adapter/closure as the standalone
reference, authority-backed shared RAM/disk admission and an owned systemd tree.
Old plans reject injected wetting capabilities, including null fields.

HIP (unqualified): `backend: "hip"` plus an explicit `selection`
(`role: compute`, PCI and UUID) plans the same recipe on that card with
`runtime-wetting-worker-hip` and sandbox `harbor-cad-wetting-hip-single-kfd-v1`.
See [GPU backends](gpu-backends.md#recipe-backends) for scope and pending gates.

Before registering success, Rust re-parses every bounded native CSV, verifies
the exact step/physical-time mapping and recorded field hashes, independently
reconstructs complete Cartesian/material coverage, phase area and the whole
contour, and rechecks the unchanged mass/contact-angle gates. Historical
`qualify --job JOB_ID` binds registered original CSV bytes. One job's numerical
checks do not establish settling, refinement or physical validation.
Each CSV manifest carries its approved physical time, native-lattice-point
association and explicit per-column units. Lattice velocities remain
dimensionless original values; their SI conversion uses the retained spacing and
physical step. Material IDs and phase fractions remain dimensionless.

Independent checks require complete unique Cartesian coordinates, the two
semantic wall planes, a finite bounded phase field and a single isolated
near-circular symmetric droplet. A centered least-squares contour fit produces
`theta = acos((y_wall - y_center)/radius)`; the wall/interface region is excluded
from the fit, and the maximum relative radial residual must be at most 0.05.
Phase area is `sum(1-phi) dx²` over bulk cells, per unit out-of-plane depth.
The maximum relative phase-mass drift must be at most `1e-3`; the absolute
angle error must be at most `5°`. Inputs can tighten these gates.

`scripts/verify_wetting_cpu.py` exercises n24/36/48 at equal physical duration
for 90° and 100° references. Each solve retains its original field bytes and
uses 200000/450000/800000 steps, respectively (0.133333 s at the declared SI inputs).
The shorter 0.005333 s and 0.016 s campaigns retained an unsettled 100° droplet;
that failed settling gate does not establish convergence. Each reference
must pass mass and angle gates; late angle change must be at most 0.2°, and
angle error must decrease across mesh refinements. Unsupported inputs must
reject with empty scientific output directories.
Campaign 5 at 0.026667 s, with the corrected fixed initial geometry, passed its
mass/angle/late-change checks and 100° refinement; its 90° coarse error was
non-monotonic while the last-quarter angle drift remained 0.16975°. Those
failed original bytes remain retained. Campaign 6 at 0.033333 s failed its late
angle-change gate. A bounded n24 diagnostic at 200000 steps approaches 89.4073°
with a last-interval change of 0.002318°; horizontal and two-direction contour
fits both confirm the drift, while independent analytical controls bound the
coarse estimator error to about 0.1°. The current campaign explicitly extends
the step ceiling to 800000 and the adapter deadline to 1200 s, with 1300 s
worker jobs, while preserving cell/snapshot/memory and every scientific gate.
Phase relaxation time is an explicit lattice numerical parameter; the reference
qualifies static angle/mass, not physical phase mobility or wetting kinetics.

Exact worker lifecycle and the changed launch runtime require qualification.
The recorded native refinement campaign supplies part of
the A1 feasibility foundation. Inlet spray momentum/drop sizes, 3D wetting,
seal/pore ingress, vapor flux, evaporation, transient Stefan verification and
physical validation retain their separate acceptance gates.
