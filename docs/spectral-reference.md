# Explicit UV irradiance and dose reference

`case validate-spectral-reference examples/spectral-reference.json` and
simulation/all MCP `spectral_reference_validate` use the same Rust contract.
They return independent analytical preparation, `executed: false`, and an
identity binding the complete original request. Preparation does not qualify
native transport, a solver package, GPU execution or a physical exposure.

## Source, surface and units

The bounded synthetic fixture uses either a fixed collimated spectral
irradiance with an explicit unit propagation direction, or an isotropic
spectral radiance. Surface normal and dimensions are explicit. Spectra have
2–64 strictly increasing samples over 200–2500 nm and cover every optical
weight; no extrapolation or visible-RGB conversion is authorized. Inputs retain
their units and provenance while normalization uses wavelengths in metres,
spectral irradiance in W/(m² m) or radiance in W/(m² sr m).

Directional incidence uses `max(0,-dot(propagation,surface_normal))`.
Hemispherical isotropic incidence uses `pi`. A procedural full-cover occluder
is supported only for the directional reference. A directional scalar cannot
authorize a sky distribution, and no atmospheric angular distribution is
collapsed or invented. libRadtran, atmospheric histories and imported surfaces
remain separate native integration work.

Sensor power is irradiance times the explicitly prescribed area. Irradiance
and radiant exposure remain independent of sensor area. Optical absorptivity
and a distinct dimensionless ageing action spectrum are explicit, bounded
piecewise-linear arrays with provenance. Incident exposure, absorbed heating
and ageing-weighted exposure are reported separately; they imply neither
temperature nor material lifetime.

The spectral integral of a piecewise-linear source times a piecewise-linear
weight is integrated exactly on each shared interval. For normalized coordinate
`x` over an interval of width `d`, source `a0 + da*x` and weight `b0 + db*x`,
the integral is `d*(a0*b0 + (a0*db+b0*da)/2 + da*db/3)`. Endpoint-product
trapezoids would incorrectly treat the quadratic product as linear.

Time dependence is an explicitly prescribed piecewise-linear nonnegative source
amplitude, with fixed geometry, direction and optics. Its complete history starts
at zero and ends within one year. Trapezoidal integration is exact for this
declared amplitude model, not evidence that a measured solar history is sampled
adequately. The example integrates 240 W/m² incident, 132 W/m² absorbed and
170 W/m² ageing-weighted UV irradiance; its one-hour ramp has integrated
amplitude 3600 s and absorbed exposure 475200 J/m².

## Native compatibility boundary

Official release metadata pins Mitsuba **3.9.1** to Dr.Jit **1.5.0**; the
Python-3.13 manylinux wheels are immutable by SHA-256:

| Package       | Wheel SHA-256                                                      |
| ------------- | ------------------------------------------------------------------ |
| Mitsuba 3.9.1 | `8959e8de33427cf4d9b515a52d741dca7624e7c17094c5849f123a96504ca123` |
| Dr.Jit 1.5.0  | `33a4b146cc56a02ea0dd9c43034277c59ae3c5dd3490678c2cbc8ef69a1e8c93` |

Mitsuba tag v3.9.1 resolves to commit
`478e193a183c21723f4a8251afc3ad29a8da4c5e`. Source-backed contracts:

- [`irradiancemeter`](https://github.com/mitsuba-renderer/mitsuba3/blob/478e193a183c21723f4a8251afc3ad29a8da4c5e/src/sensors/irradiancemeter.cpp): incident power per area, inherited shape/normal, one-pixel sensor and cosine-hemisphere sampling.
- [`specfilm`](https://github.com/mitsuba-renderer/mitsuba3/blob/478e193a183c21723f4a8251afc3ad29a8da4c5e/src/films/specfilm.cpp): native unprocessed weighted spectral channels, response ranges outside visible wavelengths, alphabetically named channels and explicit Float32 OpenEXR.
- [`directional`](https://github.com/mitsuba-renderer/mitsuba3/blob/478e193a183c21723f4a8251afc3ad29a8da4c5e/src/emitters/directional.cpp): spectral irradiance normal to propagation, explicit direction and native visibility/next-event weight.
- [`irregular`](https://github.com/mitsuba-renderer/mitsuba3/blob/478e193a183c21723f4a8251afc3ad29a8da4c5e/src/spectra/irregular.cpp): piecewise-linear spectral interpolation and explicit wavelength range.
- [`uniform`](https://github.com/mitsuba-renderer/mitsuba3/blob/478e193a183c21723f4a8251afc3ad29a8da4c5e/src/spectra/uniform.cpp): exact zero reflectance or optical response over the original declared band, without constructing a zero-mass irregular sampling distribution.
- [`sensor`](https://github.com/mitsuba-renderer/mitsuba3/blob/478e193a183c21723f4a8251afc3ad29a8da4c5e/src/render/sensor.cpp): spectral-film response controls wavelength sampling, preventing the default visible CIE range from truncating UV.

Ordinary sensor-based path tracing does not sample a delta-directional light at
the meter's origin. Directional next-event measurement and visibility must be
qualified explicitly; a zero path-traced result is not a physical zero exposure.
Hemispherical radiance uses the native meter/film path. Source weights remain
spectral power quantities and are not converted through CIE photometry.

The CPU contract deliberately records `scalar_spectral` and `Float32`, matching
the upstream wheel's native types. Three distinct ordered seeds and bounded
power-of-two sample counts are required; the declared reference tolerance cannot
exceed 0.02. Native statistical/refinement evidence is separate from analytical
preparation. CUDA transport and CPU/GPU agreement remain hardware-dependent,
best-effort work; the pinned stack does not provide a Vulkan/HIP transport route.

`nix/spectral.nix` exposes `spectral-drjit`, `spectral-mitsuba` and
`spectral-environment-cpu` in an isolated Python-3.13 environment. Exact upstream
wheel digests, bundled ELF dependencies and BSD notices have been inspected;
Nix resolves external C++/atomic/zlib libraries through the immutable package
closure and asserts the selected native CPU variant after fixup. The source
packaging passes syntax/format checks; guarded evaluation, realization and
closed-sandbox execution remain required before package qualification.

## Current verification

Rust tests check exact optical-product quadrature, nm/metre spectral-density
conversion, prescribed-history dose, sensor-area scaling, cosine orientation,
opposing normals, directional occlusion, isotropic angular integration and
rejection of missing provenance, invalid optical weights, unsupported precision/
backends, extrapolation and weakened acceptance. CLI/MCP schema and typed-error
parity use the same worker. Native packaging and execution are not qualified by
these tests.

The standalone `spectral-reference-cpu` adapter now preserves native directional
emitter weights/cosines/positions as CSV and native hemispherical irradiance
channels as Float32 EXR. The independent Python verifier uses exact Simpson
quadrature for the piecewise-quadratic optical products, separate from the Rust
coefficient reduction. Dose reduction uses Float64; it does not change native
transport precision. The operation-specific reference sandbox grants only its
immutable closure, a read-only request and writable output, with the shared
eight CPU isolation canaries.

`scripts/verify_spectral_cpu.py --executable CLI --runtime RUNTIME --output NEW`
checks normal/inclined/back-facing/occluded incidence, area semantics and
hemispherical samples at 4096/16384/65536 with three independent seeds. Every
original directional sample is reconstructed and every EXR is reopened in a
separate process. Analytical accuracy, numerical sampling refinement and
physical qualification stay separate. The explicitly marked development modes
retain diagnostic results without qualifying immutable packages or sandboxes.

OpenEXR reorders channels by name. The writer/reader check binds each scientific
channel name to its original Float32 value and requires exact equality, retaining
separate absorbed and ageing channels. It rejects swapped values, missing or
duplicated names, another pixel shape and lower-precision storage. CLI refusal
checks parse the structured `invalid_input` envelope on stdout and retain its
original bytes; stderr contains diagnostic text only.

The bounded extracted-wheel campaign `spectral-native-diagnostic-8` passes all
11 cases, 33 seed observations and 11 native/CLI pre-output rejections. Maximum
channel relative error is `0.0031445940276881856` against the unchanged `0.02`
gate. Isotropic sampling-error RMS decreases from `0.0009773395532564606` through
`0.00037565236390244103` to `0.0002414243944856635` at 4096/16384/65536 samples.
The development ABI includes explicitly hashed C++/atomic/zlib libraries and
their SONAME links. Its LLVM initialization warnings remain in the originals;
execution uses the requested `scalar_spectral` variant. These observations
establish native API/numerical diagnostics, with production package, sandbox,
worker, GPU and physical qualification still separate. Exact report and failed
attempt identities are retained in [continuation evidence](evidence/cpu-native-diagnostics-20261007.json).

The complete CPU gate passes 123 Python tests, all Rust tests, strict Clippy,
locked builds and formatter/linter checks. Native source/API, syntax and
immutable digest checks pass. Guarded build 45 refused evaluation at 12 GiB
combined memory/swap headroom, below the required 24 GiB; no realization ran.
The native spectral ABI diagnostic timed out before execution behind Atlas's
host lease held by `chaosbox-full-snapshot-sync` (PID 3337737). Its original
command, exit record and refusal log remain retained. No native spectral
measurement has been produced by that attempt.

## Bounded Lambertian reflection reference

`case validate-spectral-reflection-reference examples/spectral-reflection-reference.json`
and MCP `spectral_reflection_reference_validate` prepare a separate strict
`isotropic_lambertian_disk` contract. It wraps an explicit isotropic UV source
and a downward-facing black rectangular irradiance meter at the origin. The
centred, upward-facing disk is at `z=-height`; radius, height, constant UV
Lambertian reflectance, geometry and optical provenance are prescribed. Extra
reflection fields, including null values, are rejected by the older incident
contract.

For a point receiver, the disk's projected solid-angle fraction is
`F=R²/(R²+h²)`. Under isotropic incident spectral radiance `L_lambda`, the
Lambertian disk's outgoing radiance is `rho*L_lambda`. The part of the meter's
hemisphere outside the disk still sees the original environment, so total
spectral irradiance is `pi*L_lambda*(1-F+rho*F)`. Discarding the uncovered sky
would give the wrong finite-disk reference. The source, absorbed and ageing
integrals retain their distinct optical weights and prescribed-history dose.

The full sensor footprint is finite. If `a` is its half-diagonal, disks of
radii `R-a` and `R+a` bound the projected solid angle at every sensor point.
This gives a conservative footprint-relative-error bound. The black sensor
also blocks incoming disk illumination: its solid angle is at most
`area/h²`, giving a separate relative-error bound
`rho*F_upper*area/(pi*h²*(1-F+rho*F))`. Their sum must stay below the explicit
model-error limit, at most `1e-5` and at most one eighth of the numerical gate.
An unresolved sensor/geometry combination is rejected rather than hiding this
model error inside Monte Carlo noise.

The standalone native adapter constructs the actual disk and a black meter;
native path transport, its spectral response channels and separate EXR reader
remain the measured solver path. The exact CPU campaign adds two nonzero
reflectances and a black-disk case with independently resolved sensor dimensions,
plus native/CLI pre-output rejection tests. Separate numerical sampling and
physical qualification remain required.

The native reflection follows pinned upstream
[`diffuse`](https://github.com/mitsuba-renderer/mitsuba3/blob/478e193a183c21723f4a8251afc3ad29a8da4c5e/src/bsdfs/diffuse.cpp)
(one-sided Lambertian UV spectrum),
[`disk`](https://github.com/mitsuba-renderer/mitsuba3/blob/478e193a183c21723f4a8251afc3ad29a8da4c5e/src/shapes/disk.cpp)
(unit disk at the origin transformed by the prescribed radius and height) and
the spectral irradiance-meter/film APIs above. It makes no atmospheric, imported
material or physical-lifetime claim.

## Immutable directional worker slice

`case plan-spectral-reference examples/spectral-reference.json` and MCP
`spectral_reference_plan` produce an independent version-13 plan containing
`spectral → bundle`. Approve its exact digest and use ordinary `job submit` /
MCP `job_submit`; these share the durable Rust worker, returning a job ID.
Native submission requires systemd and the canonical shared admission authority.
The immutable `runtime-spectral-worker` selects only the isolated CPU adapter
and its operation-specific closure. The binding retains those store objects and
the original runner; restart/cancellation and resource controls use the existing
worker lifecycle.

Each of three seeds retains every native surface sample, position, cosine and UV
emitter knot. Rust independently reopens the closed CSV, checks byte identity,
ordered coverage, the explicit physical sensor frame/rectangle, native Float32
roundoff, visibility and the unchanged analytical tolerance. Compensated Float64
reductions and exact optical-product integrals reconstruct incident, absorbed,
and ageing channels plus prescribed dose. Registered originals include complete
units and `native_surface_sample` association; seeds are never physical times.
Historical `qualify --job ID` rechecks the original registered bytes and receipt,
and reports execution, numerical evidence and physical validation separately.

The version-13 worker is explicitly directional. Hemispherical/reflected EXRs
retain the standalone native campaign and require separate original-reader
worker integration. Manufactured numerical fixtures exercise corruption,
metadata/approval drift and non-promotion; they supply no native-execution
qualification. Exact packaged worker execution remains pending normal build and
runtime availability. Build 46 refused before evaluation at 9 GiB combined
headroom against the unchanged 24 GiB start gate.

`scripts/verify_spectral_worker.py` requires the complete exact-package standalone
spectral report and matching native adapter/closure. It checks CLI/MCP approvals,
immutable jobs, original CSV reconstruction, exact standalone/worker original
parity, exports, source edits, worker restart, mutation rejection, effective
controls/aggregate peaks, forced owned-tree death, cancellation and final shared
reservation/runtime-root release. The campaign itself remains unexecuted until
the normal packaging and Atlas lease prerequisites are available. Refused attempts
are recorded in [the prerequisite evidence](evidence/qualification-refusals-20261007.json).

The campaign calls the actual shared `WorkerCampaign.command/submit/wait/start`
API and uses the packaged MCP server's environment-selected socket. An ABI-free
API/signature check covers both spectral and freezing entrypoints; native
scientific qualification still requires running the complete exact-package gate.
