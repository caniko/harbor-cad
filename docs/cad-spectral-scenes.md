# Source-bound CAD spectral materials

## Direct-only native triangle reference

`cad-spectral-direct-reference-cpu` and
`runtime-cad-spectral-direct-reference-cpu` provide the independent native
candidate for `opaque_lambertian_direct_only` transport. Its strict descriptor
binds the complete prepared scene, original region STL records, explicit
collimated spectral irradiance, source-amplitude history, three seeds and a
bounded per-original-facet sample budget. The native entrypoint requires the
operation-only CPU sandbox and read-only original source mount.

Each original facet becomes a separate native `Mesh` without vertex-normal
smoothing, welding, reordering or additional placement. The original Float32 mm
vertices are converted once to Float64 metres, then explicitly measured against
the renderer's Float32 vertex storage. This additional conversion must satisfy
the approved metre rounding budget and a fixed `1e-6` native area-error screen;
the original closed-box `1e-10` area/volume gates remain unchanged. Scientific
power and energy use the original facet areas. Every original native surface
point, normal, collimated direction, source PDF, four-knot irradiance packet and
native material BSDF response is retained in closed CSV shards.

Independent reconstruction checks complete facet/sample/knot coverage, original
Float32 conversions, point association, closed-prism visibility, cosine response
and optical energy closure. It reduces exact piecewise-linear spectral products
with compensated Float64 sums. Results distinguish incident power, absorbed
heating, outgoing reflected power and ageing-weighted dose. Missing ageing
inputs leave that channel absent with an explicit status; missing optics refuse
execution. The direct-only model excludes incoming interreflection, atmosphere
and thermal feedback. It does not claim total irradiance for general reflecting
scenes or material lifetime.

`scripts/verify_cad_spectral_cpu.py --runtime RUNTIME --output NEW_DIRECTORY`
checks manufactured original facets in ten cases: three normal orientations,
oblique incidence, translated geometry, full and partial occlusion, black/white
optics and absent ageing data. Complete-scene projected-area power is checked
independently under the unchanged `0.02` numerical gate. Its explicit development
bridge mode is package-unqualified. Native original-point verification and
sampling convergence are separate statuses; registered-source worker execution
requires its own approved plan and qualification.

The Mesh construction/vertex-buffer/face APIs and native visibility behavior
follow Mitsuba 3.9.1 commit
`478e193a183c21723f4a8251afc3ad29a8da4c5e`:
[Mesh tests](https://github.com/mitsuba-renderer/mitsuba3/blob/478e193a183c21723f4a8251afc3ad29a8da4c5e/src/render/tests/test_mesh.py),
[Mesh storage](https://github.com/mitsuba-renderer/mitsuba3/blob/478e193a183c21723f4a8251afc3ad29a8da4c5e/src/render/mesh.cpp),
and [emitter sampling/visibility](https://github.com/mitsuba-renderer/mitsuba3/blob/478e193a183c21723f4a8251afc3ad29a8da4c5e/src/render/scene.cpp).

### Independently bound transport descriptor

`CadSpectralTransportRequest` binds the complete material-scene request and the
explicit illumination, history, seed/sample and geometry-rounding inputs.
`CadSpectralTransportSpec` resolves that request against the unchanged registered
CAD source into its complete prepared scene. The resolver verifies the scene's
content identity, unique whole-region assignments, known optics, complete STL
records, source approvals and separated disjoint boxes; it rejects source-byte
substitution. `examples/cad-spectral-transport.json` supplies a manufactured
request with missing ageing deliberately retained. The descriptor maps directly
to the qualified native triangle entrypoint. This source-binding foundation
resolves into an independent version-17 CPU optical/bundle plan. That plan
retains distinct-inode STL, BREP and geometric-context copies plus the original
CAD approvals in the durable submission transaction. Subsequent source changes
do not change acknowledged inputs, and interrupted unpublished copies are
removed only while the original authorized source remains verifiable. Worker
dispatch uses the operation-only CPU closure and read-only retained source tree.

`harbor-cad cad plan-spectral-transport REQUEST.json` and the simulation-profile
MCP `cad_plan_spectral_transport` return the same immutable plan and approval
digest. Submit it through `job submit` or MCP `job_submit` against an authority
that approves `runtime-cad-spectral-direct-worker`. The descriptor's request
file is never consulted after acknowledgment. The generic native runtime's
`spectral` slot selects the direct-only adapter in this dedicated runtime; its
v17 sandbox/receipt contract remains distinct from the v13 sensor reference.

Rust independently reconstructs every original facet/sample/spectral packet,
including native Float32 metre conversion, original STL topology and area,
analytical full-scene visibility, direction/normal/cosine, optical response,
incident/absorbed/outgoing-reflected power and prescribed dose. Registered
originals retain units and native facet-packet association, with no invented
physical timestamp for a seed. `qualify --job ID` rechecks the original copies,
approvals, packet metadata and numerical reductions. Manufactured receipt
fixtures cannot establish native execution or sandbox qualification. Exact
registered-source worker and lifecycle qualification remain separate gates.

The complete approved v17 plan must leave 4 KiB inside the 64 KiB wire allowance
for its request/approval envelope. Native three-seed facet receipts are artifacts:
their reader uses the existing 256 KiB registered-JSON bound, independently of
wire messages. Oversized files and symlinks refuse before numerical ingestion.

### Reading retained optical results

`harbor-cad results cad-optical REQUEST.json` and results-profile MCP
`results_cad_optical` read a complete original region at one declared native
seed. For example:

```json
{
  "schema_version": 1,
  "job_id": "QUALIFIED_OPTICAL_JOB_UUID",
  "seed": 17,
  "region_name": "solid"
}
```

The view requires independently recorded execution and verified original
packets. It retains facet order, original triangle and packet hashes, material
and illumination provenance, units, complete prescribed history and the
distinction between irradiance, power, dose and energy. Missing ageing leaves
its channel absent. A seed has no physical timestamp; the view neither averages
seeds nor introduces spatial or temporal interpolation. Queued, failed,
unbound, modified or foreign-recipe jobs cannot produce verified optical views.

`cad prepare-spectral-scene REQUEST.json` and MCP
`cad_prepare_spectral_scene` (`cad`, `results`, `all`) verify a registered
source and return a bounded preparation report. The request supplies ordered
explicit wavelength quantities, a geometry tolerance, named optical materials
and one provenance-backed assignment for **every** imported named region.
The CLI and MCP use the same Rust worker; preparation opens no CAD document
and changes no source artifacts or job state.

The initial supported geometry is the existing top-level closed planar box
scope. The report binds each region's original BREP/import evidence,
science/execution/authorization identities and registered binary STL checksum.
Whole-region material assignments cover every facet in its original order;
face numbers and display colours supply no spectral material identity.

## Precision and geometry gates

The pinned FreeCAD [Mesh writer](https://github.com/FreeCAD/FreeCAD/blob/4fd3bf320d9566a27e60069fc8387448aaa3a094/src/Mod/Mesh/App/MeshPyImp.cpp)
maps `STL` to binary STL (`BSTL`). The reader requires its exact complete
50-byte facet records, finite original Float32 positions/normals and zero
attribute bytes. It preserves coordinates and winding, converting millimetres
to Float64 metres once. The placement is already present in world coordinates;
the retained original FreeCAD placement is provenance and is not applied again.

Verification checks nondegenerate outward normals, unique facets, closed
oriented two-facet edges, complete six-plane coverage, world bounds, surface
areas and signed volume. No welding, gap healing or rounding repair is used.
World bounds satisfy the explicit tolerance, which cannot weaken the original
importer tolerance. Area and volume correspondence retain `1e-10` relative
gates. Inputs whose original STL precision cannot satisfy those gates refuse;
the source bytes remain available unchanged. This geometric assessment is
independent of spectral numerical/convergence and physical validation.

## Optical inputs and unknowns

The initial optical formulation is explicitly `opaque_lambertian`, with
`piecewise_linear` reflectance and absorptivity at **every original knot**.
Both arrays must be supplied and lie in `[0, 1]`, closing their sum to one
within `1e-12`. There is no transmission, implicit complement, RGB-to-spectrum
conversion, wavelength extrapolation or guessed reflectance. Each optical
response and dimensionless ageing action carries its own provenance and
synthetic classification through `PhysicalInput`.

Missing optical responses produce `missing_optical_inputs`; missing calibrated
ageing action leaves ageing readiness missing while retaining optical data.
Known inputs produce `prepared_not_executed`. Neither status asserts an
irradiance/dose solve, material lifetime, GPU execution or physical validation.
The scene identity changes with material, association, original geometry or
evidence changes. Triangle arrays stay in their registered source artifacts; the report
contains their identities and aggregate geometric assessments.

[`examples/cad-spectral-scene.json`](../examples/cad-spectral-scene.json) is
an explicit manufactured-material example with missing ageing data. Replace
its source job and tags only with a verified registered CAD job. Arbitrary
assemblies, curved solids, per-face mixtures and transmitting materials remain
outside this geometric/material preparation scope. Native material-tagged
surface transport requires an independently approved solver slice.
