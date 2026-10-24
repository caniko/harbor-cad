# Acceptance and execution ledger

Baseline: `d9aff76f71f7bd6b06ec54d1fad277271d1a0516`, branch
`implementation/local-worker`. This ledger records scoped evidence and remaining
acceptance work against [the implementation specification](implementation-spec.md).
Recorded qualification does not qualify a changed runtime or another host.

## Baseline verification — 2026-10-06

- `python3 scripts/check_cpu.py`: passed before this slice and after the
  source-bound presentation implementation; Rust tests, Clippy `-D warnings`,
  locked build, formatting, Ruff and 35 Python cases.
- `treefmt -C <repository> --config-file <repository>/treefmt.toml`: passed.
- Compatible pinned Simit `init ci --check --diff`: passed. Hosted CI remains
  unexecuted because work is local.
- `flake.lock` SHA-256:
  `c82b720484076521ba2a59ca4bf891873b69971a0c9766bd7f58c181d403a807`.
- `Cargo.lock` SHA-256:
  `086eb4d8227bbdaf2e376c1eba42ec43cba8bdb09d5805b8a17a2a8fbce68ae6`.
- `python/uv.lock` SHA-256:
  `b1d44d4f31b32796c5f759964ff934aa9933f6df769f6888ed17982ab7fe0f25`.

## Requirement map

Post-P1b local verification also passes the complete CPU gate with 39 Python
cases, strict version-3 frame approvals, copied-frame/time/dimension checks,
source-loss orphan preservation and concurrent admission bootstrap. Bundled
SQLite is upgraded to 3.53.2 through rusqlite 0.40.2 to include the upstream
WAL-reset fix; updated `Cargo.lock` SHA-256 is
`cebcbe7034f99fd0bbd82cbb7cda0df6630e01dff47603abaf8fc3ca010148f8`.
Generated CI drift check passes. Packaged candidate evaluation reached the
600-second wait limit while another Canix operation owned the host guard; no
lease bypass or resource override was used.

Subsequent guarded evaluation and one-job/two-core realization completed for
`6ec1289df74593d682c71690f71b9f53136a863f`. The packaged P1 qualifier passed
CLI/MCP retained-field rendering and independent registered-frame VAAPI video,
including direct source/frame mutation before/after acknowledgment and
worker restart/retry with unchanged service invocation. Cancellation preserved
science, all five new service trees closed, and runtime roots/reservations
released. See [scoped P1 evidence](evidence/standalone-presentation.json).
Version-4 source-bound HIP gradient contracts/CLI/MCP pass the CPU gate; their
separate native build and hardware qualification remain in progress.

Job-scoped `qualify --job` and results-profile MCP `qualification_report` now
inspect checksummed historical receipts and immutable execution/device/source
identities. They distinguish recorded execution from reported numerical checks
and leave current-runtime qualification/physical validation unpromoted.

An independent locked Gmsh/CalculiX CPU candidate now implements synthetic
steady conduction/Fourier flux and free-expansion reference decks, semantic
planar face predicates, positive-Jacobian/volume correspondence and complete
native field parsing. Its source/manual APIs are inspected and parser/selector
rejection tests pass. The exact native references and version-5 CLI/MCP worker
integration now pass their separate gates; see
[FEM reference scope](fem-references.md) and [exact evidence](evidence/fem-cpu.json).
Static solver parameters retain no invented physical-time mapping.

The independent version-6 prescribed transient thermal path is implemented:
CLI/MCP planning binds explicit SI histories/material domains/provenance,
native solver substeps and independent physical output times. Its CPU-only
worker policy mounts an operation-specific closure and requires conservative
authority-backed admission. The complete CPU gate now passes with 66 Python
cases, including legacy/schema/gate rejection and imported geometry/origin
contracts. `thermal-native-5` passed eight native solves, nine pre-output
rejections and separate spatial/time assessments at the recorded exact package;
the first worker attempt rejected a CPython cache entry before solving. The
bytecode-repaired native package passed `thermal-native-6`; a one-CPU timeout
in `thermal-worker-2` led to a fresh explicit two-CPU `thermal-worker-3`, which
passed CLI/MCP, raw-field/energy checks, exports, restart/idempotency, forced
complete-tree death, cancellation and final release. Exact evidence is in
[thermal evidence](evidence/thermal-cpu.json).

The updated complete CPU gate passes 103 Rust tests and 44 Python cases,
Clippy with warnings denied, locked build, Treefmt and Ruff. New systemd jobs
verify effective kernel controls before native launch and retain aggregate
process-tree RAM peaks/CPU usage through orderly completion; abrupt kills have
explicitly absent final peak records. Packaged effective-control and B2 native
qualification are still pending.

The guarded filter build from `06b018a8aaae754353acbd3d11a32bb3cf2e4877`
successfully compiled the pinned Kokkos, VTK/Viskores and native HIP adapter.
This establishes a build, not native numerical/device qualification. The
subsequent scalar-label/direction guards and latest runner must be realized
before their exact-package qualification.

CAD-facing `cad inspect|regions|export` and MCP `cad_submit`/`cad_regions` now
use the same approved worker jobs. Region views are bounded/checksummed and bind
the succeeded native CAD stage, original science/execution and placement units.
The CAD profile can submit its own inspection DAG and read durable status/logs;
foreign plans and receipts reject. Ordinal face identities, controlled variants
and imported-CAD/Gmsh correspondence remain pending.

Exact revision `358fd57f7c0a9933aafe64db63af69de7b7c91ce` passes the guarded
native B2 analytical/CPU-HIP gate and the separate packaged CLI/MCP worker gate.
Both scalar/vector linear gradients, quadratic interior gradients, boundary
CPU parity, retained velocity/pressure and 13 negative cases pass `1e-10`.
Worker source mutations, restart/retry with unchanged invocation, selected
single-KFD binding, effective controls/aggregate peaks, forced death and
cancellation pass. Original source hashes are unchanged; reservations and
runtime roots end at zero. See [exact scoped evidence](evidence/numerical-filter-hip.json).

| Specification                                            | Implementation / verification owner                                                         | Evidence / remaining acceptance                                                                                                                                                                                                                                                                                                                   |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| §1 local CLI, worker, MCP, file/native boundaries        | `src/main.rs`, `src/worker.rs`, `python/harbor_cad_mcp`, `adapters/`                        | CLI/MCP B1 recorded; remaining recipes below                                                                                                                                                                                                                                                                                                      |
| §1 selected native stacks; no extra mandatory framework  | `nix/native.nix`, `nix/openlb.nix`, `nix/fem.nix`, `nix/spectral.nix`, `nix/atmosphere.nix` | FreeCAD/OpenLB/ParaView/FFmpeg, isolated Gmsh/CalculiX, CPU Mitsuba and libRadtran pass recorded scoped native/worker gates; registered atmospheric transport passes `c8-26`                                                                                                                                                                      |
| §2 Harbor and Fleetix merged PR #3                       | `flake.nix`, `Cargo.toml`, `build.rs`                                                       | pinned signatures/contracts; source and digest drift tests                                                                                                                                                                                                                                                                                        |
| §2 packages, modules, clean-runtime discovery            | `flake.nix`, `nix/modules.nix`, `tests/worker.rs`                                           | lightweight CLI/MCP and native packages built; remaining backend packages pending                                                                                                                                                                                                                                                                 |
| §2 independent interpreters and immutable ABIs           | `nix/native.nix`, `docs/dependency-manifest.json`                                           | importer/MCP/EGL, isolated Gmsh/CalculiX and Mitsuba 3.9.1/Dr.Jit 1.5.0 Python 3.13 CPU native/worker execution verified at recorded identities                                                                                                                                                                                                   |
| §2 locks, native patches, GPU architecture               | lock files, `nix/patches/`, `nix/openlb.nix`                                                | immutable pins, `gfx1100`, isolated CPU FEM, compatible HIP filter and CPU spectral stack verified; CUDA remains best-effort/unqualified                                                                                                                                                                                                          |
| §2 unfree, driver boundary, caches and closure retention | `flake.nix`, `src/retention.rs`                                                             | scoped policy, no host activation; active roots/recovery qualified                                                                                                                                                                                                                                                                                |
| §3 compute/render/media identities                       | `src/devices.rs`, `src/authority.rs`                                                        | exact HIP PCI/UUID and initialized EGL/VAAPI identities recorded                                                                                                                                                                                                                                                                                  |
| §3 shared admission, headroom and required execution     | `src/admission.rs`, `src/resources.rs`, `src/estimates.rs`                                  | RAM/disk/card contention and death retention qualified; FEM fill-in estimates pending                                                                                                                                                                                                                                                             |
| §3 measurements, JIT and partitions                      | `scripts/verify_openlb_hip.py`, native receipts                                             | scoped kernel evidence and fixed-case equal-accuracy CPU timings pass `c8-4`; whole-card VRAM peaks and multi-GPU qualification pending                                                                                                                                                                                                           |
| §4 schemas and Python parity                             | `src/contracts.rs`, `python/tests/test_protocol.py`                                         | strict v1–v18 plans and original approval compatibility tested; source-bound spectral/cooling preparation, independent execution approvals and result-query schemas/CLI/MCP implemented                                                                                                                                                           |
| §4 prepare/run/inspect/capabilities, verified resume     | fixed adapters and receipts                                                                 | native execution available; explicit capability keys and supported resume pending                                                                                                                                                                                                                                                                 |
| §4 units, applicability, identities, unknowns            | `src/science.rs`, `src/contracts.rs`, `src/materials.rs`, `src/recipes.rs`                  | SI/identity/rejection, thermal property domains and missing-input preservation tested; scoped synthetic CPU contact and optical native/worker execution verified                                                                                                                                                                                  |
| §4 controlled CAD, tags, meshes, variants                | `adapters/freecad_bridge.py`, `src/storage.rs`, `src/cad_spectral.rs`                       | source snapshot/import isolation and imported box correspondence qualified at recorded identities; immutable v16 parameter copies/reimports and whole-region spectral triangle preparation qualified in `c8-5`; exact direct-only native triangle reference passes `c8-11`; registered optical worker passes `c8-29`                              |
| §4 allocated solver resources, unresolved paths          | `src/estimates.rs`, `adapters/openlb_channel.cpp`                                           | channel allocation/Mach/model gates; other formulations and local-gap resolution gates pending                                                                                                                                                                                                                                                    |
| §5 cold start and expansion/contact                      | recipe contracts and native FEM adapters                                                    | synthetic transient prescribed histories/heater energy, free expansion and planar contact pass scoped native/CLI/MCP gates; refined source-bound v11 thermal/contact worker passes `c8-2`; hybrid CUDA FEM and product material validation unqualified                                                                                            |
| §5 airflow/wetting/snow/freezing                         | OpenLB recipe drivers and snow thermal adapter                                              | synthetic single-phase flow, planar wetting, prescribed snow insulation/opening geometry and fixed-volume Stefan freezing pass recorded gates; exact retained-distribution cooling package passes analytic/conservation/complete-history gates in `c8-30`, shared-worker lifecycle pending; blocked-opening flow unsupported                      |
| §5 solar/UV, angular inputs and dose                     | atmospheric/spectral adapters                                                               | CPU spectral orientation/occlusion/reflection/unit/temporal gates, libRadtran/registered atmospheric worker `c8-26` and original-CAD direct optics worker `c8-29` pass scoped gates; GPU transport remains unqualified                                                                                                                            |
| §5 typed one-way transfers and convection                | `src/transfers.rs`, `src/thermal_transfer.rs`, `src/thermal_contact.rs`                     | typed conservative maps, native C3D8 capacitance projection and v11 thermal→projection→contact worker pass scoped original-field and lifecycle gates; signed retained-wetting extrusion preserves original distributions; velocity-to-convection inference rejected                                                                               |
| §5 moisture-risk entry                                   | `src/moisture_results.rs`, thermal source fields                                            | all six native planar thermal surfaces and explicit screening/missing/inapplicable branches pass packaged CLI/MCP moisture-results-1; combined recipe assembly pending                                                                                                                                                                            |
| §6 durable lifecycle/idempotency/services                | `src/lifecycle.rs`, `src/worker.rs`, `src/storage.rs`                                       | restart/disconnect/owned tree/cancel/partial output qualified; logout/reboot/resume pending                                                                                                                                                                                                                                                       |
| §6 patched importer and operation policies               | `src/sandbox.rs`, `adapters/import_policy.py`, device sandbox                               | importer and selected single-KFD HIP scope qualified; broader solver/JIT/filter policies pending                                                                                                                                                                                                                                                  |
| §6 typed input, argument arrays and credential denial    | worker operations, bounded protocol, native sandbox                                         | unknown/path/symlink/session/network canaries tested                                                                                                                                                                                                                                                                                              |
| §7 independent observations and congestion               | `ObservationPlan`, native channel output                                                    | fixed retained times and fail-on-budget; bounded configurable probes/reductions pending                                                                                                                                                                                                                                                           |
| §7 arrays and format round trips                         | `src/fields.rs`, retained-field verifiers                                                   | Float64 image graph preserved; broad topology/ghost/node-cell round trips pending                                                                                                                                                                                                                                                                 |
| §7 atomic committed data/exports and sole-source safety  | `src/storage.rs`, `src/presentation.rs`                                                     | distinct verified source copies, staging intents, atomic bundle/checksum tests                                                                                                                                                                                                                                                                    |
| §7 compute filters vs presentation                       | `adapters/paraview_bridge.py`, `adapters/video_bridge.py`, `adapters/filters/`              | independent EGL/VAAPI and scoped HIP numerical-gradient native/worker gates recorded; broader filters pending                                                                                                                                                                                                                                     |
| §7 portable provenance and retained re-render            | immutable snapshot, source-bound v2/v3 plans                                                | packaged standalone CLI/MCP rendering and independent video qualified at the recorded revision                                                                                                                                                                                                                                                    |
| §7 Catalyst/Conduit                                      | optional optimization                                                                       | deferred until measured baseline justifies it                                                                                                                                                                                                                                                                                                     |
| §7 bounded studies and resource measurements             | `src/study.rs`, `scripts/verify_study_worker.py`                                            | explicit independently approved CPU collections and immutable partial submission recovery tested; exact packaged two-case thermal study/lifecycle/resource gate passes `c8-2`; fixed-science one/two-core equal-accuracy comparison passes `c8-4`                                                                                                 |
| §8 CLI/MCP product surfaces                              | `src/main.rs`, official MCP profiles                                                        | job/results/render/video/filter and CAD regions/export/mesh/imported-FEM surfaces implemented and scoped worker gates passed; v5/v8 exact native sample/signed same-mesh compare pass packaged results-3 CLI/MCP gate (eight fields, 40 rejections); CAD variant/prepared spectral-scene surfaces implemented, fresh native package gates pending |
| §9 CI and negative tests                                 | `scripts/check_cpu.py`, generated workflow, tests                                           | local gate green; hardware gates remain opt-in and scoped                                                                                                                                                                                                                                                                                         |

## Ordered runnable slices

### Native CAD copies and material triangles — 2026-10-08

Local commits `e0c3322` and `f8605ed` deliver v16 immutable controlled native
box copies and the opt-in reimport/lifecycle campaign. `46f7434` repairs that
campaign's shared MCP call and original inspection plan-only response. Gate 20
passes complete Rust/Clippy/locked build and 186 Python cases; gate 21 passes
all 189 Python cases and a real stdio client against the Rust worker. The
campaign API regression now covers the CAD-copy entrypoint before native work.

Builds 62 and 63 retain independent evaluation-lock refusals at
`/run/lock/canix/nix-eval.lock`, owned by pid `2395140` running the separate
secret-manager reconciliation. They realize no selected package. New v16
native importer/reimport/owned-lifecycle and repaired atmosphere/registered
transport qualification remain pending matching production packages.

The source-bound spectral preparation slice binds explicit optical materials to
every named original binary STL region. It verifies original facet order,
Float32-to-SI precision, oriented closed topology and all six planar box
surfaces with unchanged `1e-10` volume/area gates. Known opaque Lambertian
response must close explicit reflectance plus absorptivity within `1e-12` at
every wavelength; missing optical/ageing data remains missing. Preparation
does not open a CAD document, mutate source artifacts or assert transport.
See [source-bound spectral scenes](cad-spectral-scenes.md).

Gate 22 passes complete Rust tests, strict Clippy, locked build, Treefmt and
pinned Simit drift. Its retained failed Python assertion was a missing binary
attribute in the new client fixture; after that focused correction gate 23
passes all 191 Python cases, including generated Rust schema and real CLI/stdio
MCP refusal parity. Manufactured geometry fixtures establish contracts only;
native material-tagged surface transport remains unqualified.

Coupled-history sampling and moisture assessment now require an explicit
`thermal_stage` index for each approved v11 block. Standalone request/report
serialization stays unchanged. Selection binds that block's exact native DAT,
complete mesh/history, execution and numerical evidence; all original times and
nodes are rechecked before source-temperature/whole-surface reads. Source gate
24 passes complete Rust tests, strict Clippy, locked build, Treefmt, pinned Simit
drift and 192 Python cases. The dedicated closed-original CLI/MCP query campaign
requires separate matching-package evidence; see [result queries](results.md).

The guarded `c8-7/coupled-results` source diagnostic passes 12 two-node reports,
216 full-surface moisture assessments and 28 refusals against both closed native
coupling jobs. Source bytes and database job/event/artifact counts remain
unchanged; the source-built query package stays unqualified. Independently,
`c8-8/native-transport` passes nine manufactured angular-reference cases and 27
seed observations with the exact immutable Mitsuba runtime and operation
sandbox. This qualifies that native reference scope; libRadtran execution,
registered-source workers, convergence, GPU and physical validation remain
separate gates. See [exact scoped evidence](evidence/native-angular-and-coupled-queries-20261008.json).

The direct-only material-tagged triangle native candidate now consumes each
original facet with explicit native Float32 geometry-rounding and BSDF checks,
retains complete native source/normal/visibility packets, and reduces original
areas into separate incident/absorbed/outgoing-reflected power and prescribed
dose. Unknown ageing stays absent; unknown optics refuse execution. Geometry and
packet mutation regressions preserve the original `1e-10` geometry and `0.02`
numerical gates. Its independent ten-case native qualifier and operation-only
Nix outputs are implemented. Matching-package native execution, registered-source
worker approval/lifecycle, sampling convergence and interreflection remain
separate unqualified capabilities; see [native triangle reference](cad-spectral-scenes.md).

The complete source gate 26 passes Rust tests, strict Clippy, locked build,
Treefmt/Ruff and 196 Python tests. Gate 25 retains the qualifier's missing
explicit subprocess check argument as an independent lint failure; its Nix
syntax and pinned Simit drift checks passed before that failure.

Build 68 and the guarded `c8-5/variants` campaign qualify controlled CAD-copy
execution and material-triangle preparation at their exact immutable package
scope. Both origin/translated variants preserve original documents and placement,
pass native reimport and CLI/MCP preparation/substitution checks, and survive
worker restart. Separate cancellation and forced-service-death jobs preserve
terminal idempotency and release owned trees, roots and canonical reservations.
All eight refusals pass; the copied state has six succeeded jobs (two source
inspections, two variants, two reimports), one cancelled and one failed job.
Material preparation remains `prepared_not_executed` for optical transport.
See [CAD native evidence](evidence/cad-variants-native-20261008.json).

The guarded `c8-9/coupled-results` campaign then passes the same 12 original
history samples, 216 complete-surface moisture assessments and 28 refusals with
the exact build-68 CLI/MCP packages. Native source bytes and database counts
remain unchanged. Independently, `c8-10/native-optical` passes ten direct-only
triangle source-diagnostic cases and 30 seeded observations; the maximum
complete-scene projected-area power error is `0.013427734375 < 0.02`. The optical
source overlay remains package-unqualified. Both reports and resource peaks are
retained in [query and triangle evidence](evidence/coupled-queries-and-triangle-diagnostic-20261008.json).

The exact build-69 native triangle package then passes all ten manufactured
cases and 30 observations in guarded `c8-11/native-optical`, with the same
`0.013427734375 < 0.02` maximum complete-scene projected-area power error.
The operation-only sandbox and original read-only source mount pass. This is
the direct-only native reference scope, independently of registered-source
worker approval/lifecycle, sampling convergence, interreflection and physical
validation. See [exact triangle native evidence](evidence/cad-triangle-native-20261008.json).

The transport descriptor now independently binds material-scene, collimated
source, original units, prescribed history, explicit rounding and bounded
complete-facet observations. Rust resolves it against unchanged registered CAD
approvals and rejects prepared-scene identity/readiness drift, unknown optics,
overlapping boxes, source substitution and weakened gates. Missing ageing remains
unknown. This is the source-binding foundation; new-plan submission/staging,
registered-source execution and lifecycle qualification are still independent
implementation work.

Source gate 27 passes complete Rust tests, strict Clippy, the locked build,
Treefmt/Ruff, Nix syntax, pinned Simit drift and 196 Python tests after the
source-bound transport contract addition. The new contract is present in the
generated schema; older execution-plan versions gain no transport capability.

Source gate 28 passes complete Rust tests, strict Clippy, the locked build and
196 Python tests for the independent v17 direct optical/bundle contract and
transactional original-input retention. Persisted-record fixtures verify exact
version separation, distinct source inodes, post-acknowledgment source mutation,
terminal idempotency and rejection of changed acknowledged copies. Native
worker dispatch and registered-source qualification remain subsequent gates.

Source gate 29 passes complete Rust tests, strict Clippy, the locked build,
Treefmt/Ruff, Nix syntax, pinned Simit drift and 196 Python tests with the v17
worker/CLI/MCP path. Rust now independently verifies complete original facet
packets, original-area optical power/dose, native geometry conversion and
full-scene visibility. Manufactured replays refuse rehashed position, direction,
normal, PDF, spectral/material and facet-association mutations, altered
reductions and invented evidence statuses. Exact registered-source worker and
lifecycle measurements still require the newly packaged runtime.

Source gate 30 passes complete Rust tests, strict Clippy, the locked build,
Treefmt/Ruff and 196 Python tests after a reproduced pending-v17 qualification
failure. Qualification now reports the original direct-only formulation and
explicit sampling budget without requiring a fluid case or inventing execution.
The registered-CAD campaign retains a distinct read-only-source state snapshot
and checks CLI/MCP plans, independent original-area power/dose, unknown ageing,
acknowledged source mutation, unchanged service invocation across restart,
separate cancellation/forced service death and final root/reservation release.

Source gates 31 and 34 verify bounded v17 original-region/seed result views:
the complete Rust/Clippy/locked-build and 196-case Python gate passes, followed
by focused tests for wavelengths/provenance and interrupted optical retention.
Unverifiable unpublished originals remain quarantined after their runtime roots
are released, including after a later source restoration. This is preservation,
not an automatic deletion queue. The failed gate-32/33 test expectations and
the first atmospheric launch's resource-control refusal are retained in scratch.

Build 73 realizes the patched atmospheric and direct-CAD runtime closures.
Guarded `c8-13/native-transport` passes the independent original-angular native
transport campaign. The same attempt passes all three transparent native cases,
but clear-sky DISORT still refuses its exact black-boundary gate: original
upward flux is `-1.507967e-16` at 320 nm and `-6.486882e-15` at 400 nm while
all native upward user-angle radiances are zero. No original was clipped.
Pinned `libsrc_c/cdisort.c:c_fluxes` evaluates quadrature intensities by cancelling
eigenfunctions, unlike the explicit boundary expression used at user angles.
The package-local black-surface patch applies the exact zero outgoing intensity
only at the bottom of a black non-emitting Lambertian boundary, before computing
the flux, mean intensity and flux derivative. Its dry-run against the original
archive passes. Native packaging now tests original clear-sky fluxes at 16/32/64
streams and retains both patches and original check outputs. New native and
registered-source qualification is required for this changed closure.

Source gate 35 passes the full Rust/Clippy/locked-build gate, Treefmt/Ruff and
198 Python tests. The first registered-CAD optical campaign refused before
creating state because the qualified variant report contains two source rows
plus two independent lifecycle rows, rather than only two rows. The campaign
now selects exactly the successful `origin`/`translated` source rows and still
requires release evidence for every row. Regression fixtures refuse duplicate
or foreign sources, failed source jobs and unreleased lifecycle entries. The
captured queued reimport envelopes are not treated as completion evidence;
the copied durable database and registered importer checks establish completion
before any new optical approval.

Source gate 36 passes the full CPU gate and 198 Python tests. It reproduces and
repairs the optical receipt-size mismatch: independent two-box native receipts
reach 125,013 bytes, exceeding the generic 64 KiB reader. Only v17 direct-optical
receipts use the existing 256 KiB registered-JSON artifact bound; wire responses
retain their 64 KiB cap. Tests cover complete larger receipts, oversized/symlink
refusal and plans that cannot carry their full approval/submission envelope.
The campaign's explicit manufactured ageing action now stays within its original
dimensionless `[0,1]` contract. Exact latest-package worker qualification remains
required; build 77 predates this source repair.

Guarded `c8-14` passes exact native atmosphere, its three separate decreasing
refinements and original-angular spectral transport prerequisites; report hashes
are retained in [native atmospheric evidence](evidence/atmosphere-native-20261008.json).
The enclosing campaign remains failed: bundle ingestion omitted atmosphere,
atmospheric-transport and CAD-optical metadata despite the fallback ingestion
path including it. A single shared annotation function now covers both paths.
Its regression reproduces the missing association and checks closed ingested
originals retain exact wavelength/solid-angle association, SI units, absence of
invented time and pinned source provenance before registration.
Source gate 37 passes all Rust/Clippy/locked-build checks and 198 Python tests.

Guarded `c8-18` confirms latest native atmosphere and verified registered source
and angular queries, then refuses a campaign-only resource-schema mismatch.
The qualifier now checks kernel counter names in their nested `controls` object
and separately captures invocation-bound systemd enforcement properties while
the owned service runs. It checks final aggregate peaks and identity as well.
`c8-19` refuses a stale original worker socket before planning; source copying
now proves worker closure by read-only exclusive acquisition of its lifetime
lock, preserving all original SQLite/WAL bytes. Tests cover stale sockets,
distinct copied inodes, exact original bytes and active-worker refusal before
copying. Source gate 39 passes complete Rust/Clippy/build and 199 Python tests;
the retained gate-38 Ruff failure was repaired using a nonblocking async process
call. Fresh exact-package worker campaigns remain required.

Source gate 40 passes complete Rust/Clippy/locked-build, formatting/lints and
200 Python tests. The `c8-21` resource refusal is retained: systemd records the
approved 664,859,648-byte request while Linux `memory.max` is the exact stricter
whole-page 664,858,624-byte bound. The qualifier now reconstructs that bound from
the host page size and independently checks the original systemd request; tests
refuse nearby substituted bounds, swap, excessive tasks and changed CPU quotas.
The `c8-20` optical campaign also preserves a CLI invocation error after original
receipt/packet/association checks; its qualification request now uses the exact
worker socket and command grammar. Matching campaigns remain required.

Exact build-82 `c8-23` now passes the native and registered atmospheric CLI/MCP
worker campaign, including all three separate refinements, preserved native
angular/direct/diffuse originals, four receiver orientations, field/receipt
mutation refusal, exact service controls, restart/idempotency, complete-tree
death/cancellation, exports and final runtime-root/reservation release. Its
scoped report hashes are in [atmospheric worker evidence](evidence/atmosphere-worker-20261008.json).
The separate transport attempt `c8-24` retains an entrypoint fresh-directory
refusal of its own worker capture log. The entrypoint now selects the already
allowlisted `atmospheric-transport.log`, as its execution function does; the
regression reproduces refusal and checks scientific leftovers and symlinks
still refuse before native dispatch. `c8-22` optical original/query checks pass
before a wrong release-helper argument; that campaign call now uses the actual
helper signature. Source gate 42 passes all Rust/Clippy/build checks and 203
Python tests, including new source-bound cooling preparation schema and real
CLI/MCP refusals. Gate 41 retains its independent missing-artifact-directory
test assertion; the repaired assertion verifies that no artifacts are created.

Exact build-83 `c8-26` passes the complete native atmosphere, independent native
transport, registered atmospheric worker and registered-source v15 transport
chain. Two CLI/MCP transport successes retain distinct original inodes before
acknowledgment, preserve source execution/approval identity, survive restart
with exact idempotency/invocation identity and independently reconstruct all
original optical packets. Six refusals include changed originals/metadata,
weakened approval and source-loss orphan preservation. Owned service death and
complete-tree cancellation release roots/reservations. Report hashes are in
[registered atmospheric transport evidence](evidence/atmospheric-transport-worker-20261008.json);
physical validation remains unqualified.

The independent `c8-25` optical attempt completes its origin case but refuses
the original far translation: Float32 metre rounding reaches `1.1921e-8 m` and
facet area error `1.1623e-6`, above the fixed `1e-8 m`/`1e-6` screens. Planning
now predicts those original conversions before approval; the native adapter
still measures its own unchanged screen. The campaign explicitly records that
translated refusal and separately approves a known-action origin case. No
rebasing, CAD edit or increased geometry tolerance qualifies the refused input.

Source gate 46 passes complete Rust/Clippy/build/format/lint checks and 217
Python tests. Gates 43–45 preserve their respective unused import, test-module
ordering and formerly admissible far-geometry fixture failures. The persisted
far-source regression now proves scene preparation succeeds while optical
planning refuses without changing SQLite or source bytes; a separately
prescribed small-placement fixture retains all earlier successful staging,
approval-envelope and original-loss assertions.

Source gate 47 passes the complete Rust/Clippy/build/format/lint gate and 221
Python tests, including strict source-bound cooling envelopes, complete original
control/subcontrol mass and enthalpy, native-receipt preservation and refusal of
writable sources, scientific leftovers and substituted boundary summaries.
`c8-27` optical completes both admissible original-field/query cases and the
far-translation refusal, then preserves an idempotency conflict: its new owned
lifecycle submission reused a key from the copied source campaign. New optical
lifecycle keys are explicitly namespaced; the original source records retain
their keys and approvals. A fresh matching lifecycle campaign remains required.

Source gate 48 passes full source checks and 222 Python tests. The `c8-28`
campaign preserved a second key mismatch in its terminal duplicate assertion;
both initial and duplicate lifecycle submissions now share one explicit
namespaced key. Independently reconstructed cooling diagnostics pass conserved
source water/enthalpy/insulated-boundary checks and fine complete-history
spatial `2/3/4` and temporal `1/2/4` assessments. Initial identity uses the
stricter `1e-10` conservation screen separately from evolved refinement; the
original coarse-history `0.02316` refusal remains recorded.

Source gate 49 passes complete Rust/Clippy/build/format/lint checks, both touched
Nix parse checks and 222 Python tests. The native retained-cooling driver,
strict original-field/boundary verifier and separate complete-history assessment
are retained in local commits. Numerical diagnostic identities and every fixed
threshold are recorded in
[retained cooling diagnostic evidence](evidence/retained-cooling-diagnostic-20261008.json).
Operation-only source-bound native reference packages and their exact-package
campaign are exposed; package/worker execution qualification remains pending.

Exact immutable build 86 and campaign `c8-29` pass the original-CAD optical
worker lifecycle, CLI/MCP seed/region queries, before-ack distinct-inode input
retention, unchanged source, translated-geometry refusal, restart/idempotency,
cancellation/service death, portable exports and every root/reservation release.
The failed `c8-25`, `c8-27` and `c8-28` originals remain preserved. See
[optical worker evidence](evidence/cad-optical-worker-20261008.json).

Exact build 87 and guarded `c8-30` pass eleven isolated native retained-cooling
cases and four before-dispatch refusals. Complete original field, water amount,
enthalpy and half-link boundary conservation retain `1e-10`; fine uniform
Stefan and evolved spatial/temporal complete histories retain `0.02`. Both
coarse failed gates are preserved. The campaign used 190.3 MiB peak/no swap;
shared-worker and physical qualification remain separate. See
[cooling package evidence](evidence/retained-cooling-package-20261008.json).

Source gates 50 and 52 pass full Rust/Clippy/locked build/format/lint checks and
222 Python tests. Gate 51 preserves the campaign import-order lint failure.
The v18 shared-worker cooling slice binds complete source history and independent
authorization, conservatively reserves input/output/native resources, retains
distinct original inodes under the submission transaction, reconstructs every
native subcontrol and boundary-energy history in Rust, and exposes exact-time
CLI/MCP sampling/export provenance. Focused gate 53 additionally passes the
source-loss/orphan/mutated-provenance and rehashed-original regressions, strict
Clippy and locked build. Its exact-package lifecycle campaign remains pending;
source tests and archived fixtures do not qualify native execution.

Build 88 preserves its packaged-CLI compile failure: the Cargo source filter
omitted the embedded native cooling driver. `7718403` includes that driver and
the cooling fixture; exact build 89 passes CLI/MCP/runtime realization with
packaged Rust tests. Campaign `c8-31` then preserves its pre-submission failure:
the MCP simulation profile did not expose cooling planning. `1c49207` exposes
the planner in simulation/results profiles and explicitly uses the results
profile for nonexecuting initialization preparation. Focused gate 56 verifies
real CLI/MCP error parity and tool discovery for those profiles.

The worker additionally publishes complete lossless VTK XML StructuredGrid
control-point views with original child/parent IDs, exact Float64 positions and
fields, physical times, extrusion depth, subcontrol volume and empty CellData.
Historical verification reconstructs every published view from authoritative
CSV bytes. Source gate 57 passes the complete Rust/Clippy/build/format/lint and
222-case Python gate; its VTK topology/Float64/unit roundtrip regression passes.
Exact native/shared-worker qualification of this updated runtime is pending.

### Receipt and packaging continuation — 2026-10-08

Commit `413fe00` separates numerical-only atmospheric receipt reconstruction from
the strict registered-worker request/attestation gate, and repairs the adapter's
`reference` invocation and operation-specific preopened log. The full CPU gate
passes with 169 Python cases; the strengthened qualification prerequisite tests
subsequently pass the complete 171-case Python gate. Source captures and check
logs remain under `harbor-cad-resume-20261007/source-verification-7` and `-8`.

`atmospheric-receipt-replay-2` independently reconstructs all nine retained native
manufactured-source receipts, 27 seeds and 54 original component files. All 117
numerical mutations reject, and all nine originals continue to reject at the
strict worker-attestation gate. This numerical-only replay does not promote
registered source execution, a package, sandbox, worker, convergence or physical
validation. Failed `atmospheric-receipt-replay-1` remains independent. Exact hashes
are in [diagnostic evidence](evidence/cpu-native-diagnostics-20261007.json).

Normal guarded evaluation now succeeds. Build 53 compiled the CLI but failed its
packaged tests because the filtered source omitted `atmosphere-transfer.json`;
`87f0a0f` retains that fixture. Build 54 then realized the current CLI and MCP but
failed libRadtran configuration: `strictDeps` excluded the executable NetCDF
configuration helpers from PATH. `379ec20` supplies them as native build inputs.
Both failed build logs remain retained; exact remaining native/runtime realization
and worker qualification are still pending. `f1c8248` exposes the transport worker
runtime as a flake package, and `c80bd33` implements its complete registered-source
CLI/MCP qualification campaign with independently rechecked exact prerequisites.

Build 56 realizes the already-evaluated independent thermal/contact, freezing,
spectral and atmospheric-spectral operation-only packages with one job/two cores.
The exact current CLI/MCP planar contact worker passes its complete four-job
campaign as `c8-1/contact-worker`: two successful native solves, forced-tree death
and cancellation, immutable restart/idempotency, original fields, offline exports
and final admission/runtime-root release. [Current contact evidence](evidence/current-contact-worker-20261008.json)
binds its packages and source; the initial overlong Unix-socket startup attempt
remains independently failed. Build 55 timed out behind another Canix operation's
evaluation guard. The repaired libRadtran evaluation and all other current
native/worker campaigns continue independently.

## Bounded immutable studies

The first F study slice provides at most 16 explicitly named, independently
approved CPU plans through Rust CLI and simulation/all MCP. One durable SQLite
intent binds the original collection, profile and preflight execution identities;
stable child keys recover partial submission through ordinary worker jobs. All
case approvals and effective admission limits are checked before any intent or
child is written. Changed collections or execution identities reject; cancelled,
failed and foreground-interrupted children keep their original terminal jobs.

The complete CPU gate in `source-verification-12` passes Rust tests, strict Clippy,
locked build, formatting/Ruff and 173 Python tests, including real CLI/MCP study
parity, all-case preflight refusal, restart identity and checksum exports. The
initial study contract was observed red, and intermediate formatting/lint/test
attempts remain separate scratch records. [Study contract](studies.md) describes
the bounded collection and weaker foreground guarantees. Exact packaged native
study execution and equal-accuracy optimization remain independent gates.

## Exact native/worker continuation and thermal-original repair

[Current native/worker evidence](evidence/current-native-worker-20261008.json)
records six successful exact-package native campaigns (contact, thermal,
freezing, spectral, atmospheric-spectral and snow) and the complete thermal,
freezing and spectral CLI/MCP worker gates. Both enclosing attempts retain exit
1: libRadtran was unavailable in the native selection; thermal/contact failed its
original energy gate; snow failed the historical DAT-mutation-refusal assertion.
Build 57 independently refused behind the existing Canix evaluation guard.

The coupling fixture now explicitly refines its native integration to 1.25
seconds. Separate exact-driver diagnostics pass both blocks at that step and
0.625 seconds with decreasing original temperature/energy errors and unchanged
`0.02` gates. The historical thermal verifier now requires registered original
mesh/DAT/JSON bytes and complete native/retained schedules; changed/missing
original and receipt-substitution regressions pass. The complete CPU gate in
`source-verification-13` passes Rust tests, strict Clippy, locked build,
formatting/Ruff and 173 Python tests. These source repairs require fresh packaged
snow/coupling worker campaigns before their acceptance.

The fresh guarded `c8-2` exact-package repair campaign now passes prescribed-snow
native and complete CLI/MCP worker gates, one-way thermal/contact worker gates
and the bounded native thermal study. [Checksummed evidence](evidence/repaired-workers-20261008.json)
binds the repaired CLI/MCP, original independent runtimes, campaign source and
all reports. Snow rejects changed original DAT during historical qualification.
The coupled solve uses the refined explicit step and reconstructs both original
thermal sources, conservative projections and original contact fields.

The study records two independently verified native cases and their exact
120-second node comparison (`0.0005008012855682864 K`), original bundles,
kernel resource measurements and complete restart/terminal-child recovery.
The case CPU usages are `14,334,028` and `27,816,472` microseconds with job-cgroup
peaks `64,479,232` and `89,042,944` bytes. These are single-case refinement
measurements, not equal-accuracy optimization acceptance. All scoped campaign
reservations and runtime roots close; physical validation remains unqualified.

## Prescribed snow/opening geometry

`case validate-snow-openings` and simulation/all MCP `snow_openings_validate`
now evaluate explicit named planar apertures against bounded prescribed closed
snow prisms. [The geometry contract](snow-openings.md) binds original SI-capable
inputs and provenance, preserves closed-plane tangency and thin remaining slits,
and counts overlapping projected coverage once. Four Rust tests compare 64
arrangements against independent unit-square area accounting, verify all normal
axes/units and refuse unresolved spans, unknown physics and oversized responses.
Real CLI/MCP parity and typed refusals pass through the same worker.

The complete `source-verification-16` CPU gate passes Rust tests, strict Clippy,
locked build, formatting/Ruff and 174 Python tests. This closes the explicit
prescribed planar blockage calculation; blocked-opening flow, permeability and
automatic convection inference remain outside that geometric model. No native
solver execution or physical validation is inferred from the area report.

## Equal-accuracy CPU execution profile measurements

The guarded `c8-4/equal-accuracy` exact-package campaign passes 12 original
native thermal solves with identical approved science, mesh, all retained times
and point IDs. Original temperatures agree exactly, below the independent
`1e-12 K` comparison gate; every solve separately passes unchanged temperature/
energy and historical original-byte gates and releases its reservation/root.
Five paired repetitions after one retained warmup per profile yield medians of
`16.353552431042772 s` at one core and `15.458660325035453 s` at two cores through
checksum-verified export. Observed maximum job RAM is `71,417,856` versus
`192,008,192` bytes. [Evidence](evidence/equal-accuracy-cpu-20261008.json)
binds the complete originals, package/runtime/campaign identity, samples and
separate kernel measurements. This closes the bounded fixed-case CPU profile
comparison in F; GPU/JIT optimization and other workloads remain independent
unqualified gates. Dispatch defaults and scientific approvals are unchanged.

## Controlled native CAD-copy variants

Version 16 binds one original authorized native CAD source, its document and
geometric evidence, explicit new dimensions and unchanged box placement. The
CAD/all CLI/MCP planning path prepares a new immutable approval; the ordinary
worker retains distinct-inode original copies and exact source execution
provenance before acknowledgment, under shared staging admission. Only fixed
native box dimensions are editable. Patched FreeCAD recomputes the copied
document and Rust independently verifies original/recomputed bounds, volume,
named regions, placement and closed originals before publication and historical
qualification. No solver or physical-validation status is inferred.

The full `source-verification-20` gate passes Rust tests, strict Clippy, locked
build, formatting/Ruff and 186 Python tests. New storage tests cover original
mutation, acknowledgement identity and preserved unverifiable staging orphans;
contract tests reject older-version injection, resource understatement and
foreign physics. Python allowlist tests refuse extra objects, formulas,
assemblies and changed original context before recompute. Exact native variants,
reimport correspondence and worker lifecycle remain independently unqualified
until the rebuilt importer/CLI/MCP and matching campaign execute. See
[controlled variants](cad-variants.md).

The original/export verifier additionally rejects coherent false recompute
metadata and substitutions of all nine bound original/provenance files. The
persisted envelope fixtures do not claim native execution. `source-verification-19`
retains a failed assertion using a nonexistent error-code spelling; gate 20
confirms the real CLI/MCP worker emits the existing typed `unqualified` refusal.
Build 61 retains the independent Canix evaluation-lock refusal; it is not a
package-build or physics failure.

| Slice   | Acceptance                                                                                       | State                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ------- | ------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P0      | baseline locks/tests/generated-workflow drift; requirement/prerequisite ledger                   | complete                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| P1a     | explicit presentation approval; atomic retained-source copies; render/optional video CLI/MCP     | complete at the recorded package/hardware scope; packaged mutation/lifecycle/exports pass                                                                                                                                                                                                                                                                                                                                                |
| P1b     | independent registered-frame video plans, lifecycle and source mutation rejection                | complete at the recorded package/hardware scope; CLI/MCP frame mutation, restart/retry and release pass                                                                                                                                                                                                                                                                                                                                  |
| P2      | versioned recipe inputs, stage-local artifacts, conservative typed transfers and native policies | transfer/unit/material/cold-input foundation and independent v5 static FEM implemented; exact CLI/MCP native mesh/fields and operation-specific isolation qualified; closed BREP/imported-box correspondence and origin-aware FEM native gates pass; v7 mesh worker passes at build 22; v8 imported static FEM CLI/MCP/exports/lifecycle passes with matching native prerequisite at build 23; broader geometries/recipes remain pending |
| P3 / B2 | exact compatible HIP numerical-filter stack, observations, RAM/VRAM telemetry and format checks  | scoped v4 native/CLI/MCP analytical, CPU-HIP, byte/topology, source/lifecycle and aggregate RAM/CPU gates pass at the recorded revision; instruction trace, whole-card VRAM and broader filters pending                                                                                                                                                                                                                                  |
| P4 / A1 | controlled CAD/regions/Gmsh; thermal, FEM, wetting and flow reference gates                      | synthetic steady conduction/free expansion at 2/4/8 and CLI/MCP lifecycle pass; imported origin/translated BREP correspondence and static FEM pass native/worker gates; airflow separately scoped; repaired wetting-native-9 passes six solves, twelve rejections and two decreasing-error sequences; wetting-worker-3 passes fresh CLI/MCP/restart/exports/cancel/kill gates                                                            |
| P5 / C  | native thermal/contact/moisture/coupling slice                                                   | synthetic transient thermal, contact, exact original-field queries, full-surface moisture and conservative thermal→contact coupling pass native/CLI/MCP/export/lifecycle gates; repaired v10 worker passes `c8-1`, v11 `c8-2`, coupled history/moisture `c8-9`; hybrid CUDA FEM and physical product inputs remain unqualified                                                                                                           |
| P6 / D  | native local water, prescribed snow and retained-water freezing                                  | planar wetting, prescribed dry snow insulation/opening geometry and independent fixed-volume Stefan references pass scoped native/CLI/MCP gates; signed original wetting extrusion and exact retained-cooling package `c8-30` preserve phase/conservation/refinement; v18 shared-worker lifecycle pending; moving/overshooting source cooling, physical water-air and blocked-opening flow unsupported                                   |
| P7 / E  | native atmosphere/spectral irradiance/dose slice                                                 | isolated CPU spectral and libRadtran original direct/diffuse reference plus registered-source worker pass `c8-26`; original-CAD direct-only opaque Lambertian irradiance/power/dose passes `c8-29`; GPU spectral, general interreflection and physical material lifetime remain unqualified                                                                                                                                              |
| P8 / F  | bounded studies, retention/recovery/resume, measurements and product closure                     | bounded immutable native studies/lifecycle/resource gate passes `c8-2`, fixed-science equal-accuracy profile comparison `c8-4`; active runtime roots/shared reservations and failure originals are retained; actual logout/reboot, solver checkpoint/resume, broader GPU measurements and final product closure remain separate                                                                                                          |

The next results slice adds strict registered-v6 `results sample-thermal` and
`compare-thermal` plus matching results-profile MCP tools. Complete JSON histories
are independently checked against their authoritative DAT counterparts; the
complete native schedule/node coverage is separately verified. Only declared
field-observation times authorize sampling. Native time-serialization
error is exposed separately from exact requested times; signed comparisons keep
both physical states. Treefmt, full CPU checks (81 Python cases) and generated
Simit CI drift pass. The first development diagnostic exposed the differing
JSON observation/DAT energy schedules and remains retained as failed evidence.
The corrected diagnostic matches both qualified thermal jobs at 10/120 s and
their signed differences, including generated report-schema checks. Build 32
predates that correction. Static regression `results-4` passes eight field checks
and 40 rejections. Corrected build 33 passes `thermal-results-2` with eight field
checks and 52 rejections; build 34 also repeats the complete thermal gate.

Source-bound `results moisture` and matching results-profile MCP now reduce
complete geometrically verified native thermal box surfaces at an exact retained
time. Air inputs, missing data and justified inapplicability remain explicit;
subzero surfaces retain the separate unsupported ice/frost status. The full CPU
gate (82 Python cases), Treefmt and Simit drift pass. A retained development
diagnostic checks both thermal jobs, native surface minima and all three
assessment branches. Build 34 passes `moisture-results-1` with 44 thermal/surface
checks and 62 rejections, including all six complete surfaces. All query gates
leave the original closed source trees and copied database records unchanged.
Exact identity/report hashes are in [result evidence](evidence/thermal-results-cpu.json).

Native `wetting-native-7` passes at build 31 with unchanged scientific gates and
explicit equal-duration refinements. `wetting-worker-1` fails before solve due
to its preopened stage log. Repair `8ee49c2` passes the full CPU gate (86 Python
cases). Build 35 passes `wetting-native-8` with all six solves, twelve rejections
and both refinement sequences. `wetting-worker-2` exposes the same preopened log
at the C++ directory guard and fails before solving. Repair `7fb436c` tests the
actual native guard without OpenLB/hardware, including valid worker/standalone
layouts and seven stale/alias/type rejections. Build 40 will repeat the complete
native gate across both initial directory layouts, then its matching worker gate.
All failed native campaigns and both worker attempts remain retained independently.

The latest full CPU gate passes with 95 Python cases, Clippy warnings denied,
locked Rust build/tests, formatter and linter checks. `contact-native-2` passes
at build 37; v10 contact CLI/MCP/worker packaging is in progress at build 38.
Complete native C3D8 temperature projection conserves the source material's
thermal capacitance, retains independent pointwise approximation loss and binds
the exact registered mesh/field/DAT/time identities. Both original closed thermal
sources pass the development diagnostic; its new exact packaged CLI/MCP,
mutation, source-immutability and conservation gate is pending at build 39.

## Continuation — 2026-10-07

The approved Canix parent environment executes project subprocesses again.
Local commit `305458a` retains the verified thermal-contact input contract.
Version 11 now binds separate lower/upper native thermal histories, explicit
capacitance projection and pointwise loss limits, derived static contact inputs,
native six-surface moisture entries and a final original-field bundle. CPU tests
cover nonuniform capacitance weighting, exact schedules, source/report mutation,
older-schema capability rejection, immutable plan identities and real CLI/MCP
planning. Exact-package native coupling qualification is pending.

The earlier launcher directory, original native reports and most worker state
files under the recorded scratch paths are absent. Historical committed hashes
remain historical evidence; missing artifacts cannot support fresh rechecks.
New campaigns use protected `harbor-cad-resume-20261007` source captures and
fresh output names. `wetting-native-9` now passes all six solves and twelve
rejections, including standalone and preopened-worker-log directory layouts.
Both equal-duration refinement sequences and all unchanged mass/angle/settling
gates pass; fresh original fields and the report are retained. See the
`recovered_standalone` entry in [wetting evidence](evidence/wetting-native-cpu.json).
Follow-on CPU requalification uses the normal bounded host lease. Version-11
implementation `0d995d6` passes all Rust tests, strict Clippy, 97 Python cases,
Treefmt, Simit drift and whitespace checks. New package evaluation has been
refused at 15, 18 and 19 GiB available against the required 24 GiB start headroom;
build-41 captures the exact `0d995d6` request and rejection. Exact guarded
packaging remains pending.

Fresh `contact-native-3` and `thermal-native-7` reports now retain replacement
originals for the missing historical scratch. Contact passes 15 solves, eight
rejections and five fixed-law spatial assessments. Thermal passes eight solves,
nine rejections and its separate original-field, energy, spatial and temporal
gates. Maximum thermal temperature/energy errors are 0.0075584/0.0153847 against
the unchanged 0.02 limits; successive temporal differences decrease with
observed order 0.999904. The exact report/runtime identities are recorded in
[recovery evidence](evidence/native-recovery-20261007.json). Later worker and
projection campaign stages remain separate gates.

The recovery campaign completed all five subprocesses successfully. Fresh
`thermal-worker-4` and `wetting-worker-3` pass CLI/MCP native solves, restart and
idempotency, independent original-field checks, shared admission, historical
qualification, exports, owned failures/cancellation and reservation release.
`projection-results-1` passes CLI/MCP read-only capacitance projection and
original-field mutation/loss rejection from the retained thermal jobs; source
bytes and database counts remain unchanged. These gates bind the recovery
CLI/MCP packages recorded in recovery evidence; they do not qualify the new
v11 package or repaired typed MCP errors. Guarded build-42/build-43 were refused
at 21 GiB headroom; both attempt logs are retained.

## Freezing queries and prescribed snow continuation

Source commits `b769b3e` and `b0894b3` complete bounded exact-time original
freezing samples/comparisons through CLI/MCP and preserve the JSON fixture in
filtered Cargo packaging. Registered-byte substitution and unexecuted-fixture
rejections pass with complete historical science verification. The Rust
development replay accepts the same six attempt-10 native cases as the Python
verifier; its receipt/sandbox fields are explicit fixture scaffolding and do not
qualify a runtime, worker or sandbox. Exact production gates remain pending.

The prescribed dry-snow planning slice uses the supported native plane-wall
thermal solver with an approval-bound series resistance, explicit omitted-storage
screens and conservative subzero applicability. Full CPU verification passes
all Rust tests, strict Clippy, locked build and 112 Python tests, including real
CLI/MCP approval/error parity. Treefmt, Nix syntax, Simit drift and whitespace
checks pass. The [snow reference](snow-reference.md) defines its limited scope
and dedicated native campaign; exact native/worker and physical qualification
remain pending. D also requires separate blocked-opening geometry and broader
environmental comparisons.

Source-bound `results retain-wetting` and matching result-profile MCP now retain
complete original phase/velocity distributions through an explicit translated
nodal extrusion, with original byte identities, signed overshoot accounting and
an independent `1e-10` mass conservation gate. Missing native temperature and
downstream cooling prerequisites stay explicit. The full CPU gate passes all
Rust tests, strict Clippy, locked build, Ruff and 112 Python tests, plus Treefmt,
Simit drift and whitespace checks. See [retained wetting](retained-wetting.md).
Exact-package queries and downstream native retained-distribution cooling
remain pending independent qualification.

## External qualification prerequisites

The E reference now has a strict Rust SI/angular-source contract and matching
CLI/MCP preparation. Exact piecewise-linear optical-product quadrature preserves
absorbed heating separately from ageing-weighted dose, with explicit sensor area
and prescribed complete temporal amplitude. UV nm/metre conversions, cosine/
occlusion/isotropic references and real protocol rejection tests pass the full
CPU gate (45 Rust unit tests plus integration suites, strict Clippy, locked build
and 112 Python tests). Treefmt, Ruff, pinned Simit drift and whitespace checks
pass. Official Mitsuba 3.9.1 / Dr.Jit 1.5.0 wheel identities and pinned spectral
plugin semantics are recorded in [spectral reference](spectral-reference.md).
Native packaging, transport, reflection, atmospheric integration and GPU
evidence remain unqualified.

The standalone spectral CPU adapter and original-field campaign are implemented,
with independent Simpson optical-product quadrature, native emitter visibility
and unchanged EXR spectral round trips. Pure CPU gates pass 123 Python tests,
all Rust tests, strict Clippy, locked builds, Ruff/Treefmt and pinned Simit drift.
Build 45 refused evaluation at 12 GiB combined headroom against the normal 24 GiB
start gate, before realization. Snow-native-2 and retained-phase-1 both timed out
without execution behind the Atlas lease held by PID 3337737,
`chaosbox-full-snapshot-sync`, on `/run/lock/canix/switch.lock`. Native qualification
remains pending normal lease release/headroom; all refusal logs are retained.

Protocol repair `e51c02a` preserves valid request IDs on nested schema failures,
duplicate-field rejection and strict empty operations; the real MCP regression
passes. Reflection commit `98690d4` retains uncovered sky and explicit UV disk
reflectance, separately bounding finite sensor footprint and illumination
shadow. The full CPU gate passed with 124 Python cases.

The independent version-13 directional spectral worker slice binds approved
angular source, geometry, optical curves, three seeds and complete prescribed
dose history through `spectral → bundle`. CLI/MCP plan/approval parity, canonical
authority rejection, immutable resource minima and native closure retention are
implemented. Rust independently verifies every original CSV sample/knot and
registered identity, reconstructs optical products/dose and prevents manufactured
receipt promotion. The full CPU gate passed with 47 Rust unit tests plus all
integration suites, strict Clippy and 126 Python tests; pinned Simit drift and
both touched Nix files parse. Review exposed ordinary native sum drift at 65536
samples; compensated Float64 reduction preserves original Float32 knots and the
existing scientific gate. Its red/green regression and binding-policy check pass.
Exact packaged standalone/worker execution remains pending.
Build 46 refused before evaluation at 9 GiB combined headroom against 24 GiB;
spectral ABI attempt 2 refused before execution behind the same Atlas lease.
Exact-package worker qualification is wired through
`scripts/verify_spectral_worker.py`, requiring the complete matching native
angular/reflection/refinement prerequisite. Build 47, at committed worker source
`3ad8dbc`, refused before evaluation at 10 GiB headroom. Consolidated original
command/log hashes are in [prerequisite refusal evidence](evidence/qualification-refusals-20261007.json).

The source-built spectral native diagnostic also refused before execution after
its bounded 1800-second Atlas lease wait; its original refusal identity is now
retained in the same evidence file. Independent source work has established the
official content pin and inspected public CPU DISORT APIs for libRadtran 2.0.6.
The [molecular UV reference](atmosphere-reference.md) retains full original
anisotropic angular radiances, explicit source/profile/solar units and provenance,
native Float32/text resolution and independent diffuse-flux/cosine-law gates.
Strict Rust CLI/MCP validation and isolated native packaging are implemented;
exact native execution, separate convergence and angular transport integration
remain unqualified.
Full CPU verification passes with 129 Python tests, the new Rust atmospheric
contract and all existing Rust suites, strict Clippy, locked build, pinned Simit
drift, Nix syntax and whitespace. The official archive and all twelve selected
source/data hashes match retained originals. Native decimal-wavelength resolution
and source-energy rejection tests were observed red before the focused fixes.
The exact atmospheric campaign is now wired for twelve native cases, separate
stream/angular-shape/wavelength refinement and eight pre-output rejections.
Original AFGL data/native executable identities and compact receipt field
metadata preserve authoritative angular arrays in files. Guarded build 48,
including the new atmospheric outputs, refused before evaluation at 9 GiB
combined headroom. Atlas lease PID `3337737` remained active at the subsequent
inspection. All scientific/package qualification stays pending these prerequisites.

Version-14 atmospheric worker source integration is implemented: immutable
`atmosphere → bundle` approvals, authority-before-runtime admission, separate
closure/sandbox policy, original full-sphere fields with registered units and
wavelength/solid-angle association, independent Rust field/energy reconstruction
and read-only historical evidence. Legacy approvals reject atmospheric injection
including null. The full CPU gate passes 49 Rust unit tests and every integration
suite, strict Clippy, locked build, 131 Python tests with real CLI/MCP planning
parity and authority refusal, pinned Simit drift and touched Nix syntax. Exact
packaged native atmospheric worker execution and atmospheric-to-Mitsuba transport
remain pending; manufactured numerical fixtures cannot promote runtime status.

The exact atmospheric worker qualification campaign is wired for matching
standalone originals and separately reconstructed stream/angular/spectral
refinements, CLI/MCP parity, immutable approval/refusal, restart, native-tree
death/cancellation, checksum export and resource release. Its API/signature and
refinement-error/changed-original regressions pass; development runners refuse
before output creation. Guarded build 49 binds `2cace05` and refused before Nix
evaluation at 8 GiB combined headroom against the 24 GiB start minimum. The
standalone and worker atmospheric campaigns remain unexecuted.

Atlas released its lease before snow attempt 3 and retained-phase attempt 2.
Both launched under the normal lease/resource controls and exposed campaign
prerequisite defects, preserved in [launch-repair evidence](evidence/campaign-launch-repairs-20261007.json).
The standalone thermal runtime now supplies the closure needed by the snow
sandbox. Development retained-phase MCP keeps its virtualenv interpreter path;
a real clean-environment child import verifies the SDK ABI. All 133 Python tests,
lint/format checks and thermal Nix syntax pass. These source repairs require
corrected package/campaign execution before qualification.

Registered native atmospheric originals now support a strict read-only
anisotropic transfer preparation through Rust CLI and the results MCP profile.
The descriptor preserves the native direct/diffuse split and full-sphere angular
midpoints, original source/receipt/execution identities and explicit receiver
orientation. Original-to-emitter values are back-reconstructed under `1e-10`;
area-dependent power, absorption and ageing-weighted prescribed dose stay
distinct. Isotropic/orientation/SI conversion and unexecuted-source rejection
tests pass. Exact original worker qualification is wired to independently check
four receiver normals and mutation refusal; native Mitsuba transport remains
pending and the preparation reports `executed=false`.
The full CPU gate passes with 49 Rust unit tests, all integration suites including
four transfer references, strict Clippy, locked build, formatting/lints and 134
Python tests; pinned Simit drift is clean. A real results MCP regression also
verifies the repaired CLI/worker typed rejection of unknown fields, including
null. Failed retained-phase attempt 4 is preserved as a failure.

After bounded lease waits, retained-phase attempt 3 and spectral diagnostic 2
refused behind Atlas lease PID 944486. Build 50 refused before evaluation at
9 GiB combined headroom. Their immutable commands/logs remain in
[prerequisite evidence](evidence/qualification-refusals-20261007.json).

- Multi-GPU KFD exclusion needs multiple supported live devices and matched
  running-kernel source evidence. The observed host exposes one live AMD KFD GPU.
- Hybrid CUDA FEM and CUDA/OptiX spectral transport need suitable NVIDIA hardware.
  CUDA remains best-effort; available AMD work continues independently.
- The compatible VTK/Viskores/Kokkos HIP filter passes scoped build, dispatch,
  numerical/roundtrip, worker and resource gates; instruction traces, whole-card
  VRAM and broader operations remain separate qualification work.
- Real environmental claims need geometry, material/contact/wetting/optical data,
  operating histories, acceptance limits and prototype evidence. Synthetic native
  reference implementation can progress; missing physical inputs stay explicit.
- Paperclip revised-attempt-2 audit remains independent. Its recorded exit 1 and
  failed tests/build do not establish full application qualification.

## Original-midpoint native spectral transport continuation

The separate atmospheric-spectral adapter and operation-only package outputs now
preserve every original direct/diffuse midpoint, native emitter identity, source
PDF and four-wavelength Float32 packet. Independent reconstruction requires both
canonical component artifacts, the actual rectangular sensor footprint and the
source-derived emitter PMF. Eight review reproductions failed before repair and
pass afterward, including empty/missing/duplicated originals, renamed artifacts,
extra reported components, compensated PDF/weight substitution and off-rectangle
points. The campaign now has nine manufactured-source native cases, including
positive direct illumination, empty diffuse scenes, mixed illumination and six
varying spectral knots with separate optical curves. These source checks do not
qualify native renderer execution or registered atmospheric sources.

The complete bounded CPU gate passes all Rust tests, strict Clippy, locked build,
Treefmt, Ruff and 153 Python tests; pinned Simit drift and both touched Nix files
pass. Original commands/logs are retained at
`harbor-cad-resume-20261007/source-verification-1`. Additional entrypoint checks
cover checksum/duplicate-field/isolation/read-only refusal before native outputs.
Exact package realization, guarded native API execution and the independent
version-15 registered-source transport approval remain pending.

Build 51 and the three later lease refusals (`retained-phase-5`,
`spectral-native-diagnostic-3`, `cpu-remainder-1`) are now consolidated in the
[prerequisite evidence](evidence/qualification-refusals-20261007.json). All exited
1 before evaluation or native execution; the 3600-second remainder wait did not
start a systemd service. Their originals remain independent from future retries.

## Native diagnostic recovery and unchanged scientific gates

The normal Atlas lease is available again. `cpu-remainder-2/retention` passes
16 original-source phase/velocity extrusions and 14 CLI/MCP rejections. Source
bytes and database counts `[4,32,74]` remain unchanged; maximum mass-conservation
error is `2.220446049250313e-16` at the unchanged `1e-10` gate. Its source-built
CLI and Python-module MCP remain development-only, and no cooling solve is
claimed.

`spectral-native-diagnostic-5/atmospheric-spectral` passes nine manufactured
native midpoint cases and 27 seed observations at the unchanged `0.02` gate,
including the repaired original coverage, PMF and rectangle checks. Its enclosing
attempt retains exit 1 from the separate failed spectral campaign. The complete
spectral campaign then exposed and repaired named EXR ordering, exact-zero
irregular-plugin rejection and a qualifier reading the CLI stdout error envelope
from stderr. `spectral-native-diagnostic-8` passes 11 cases, 33 observations,
11 pre-output rejections and decreasing three-level isotropic sampling error.
The named EXR comparisons remain bit-exact; zero optical spectra use the pinned
uniform plugin over their original wavelength band with no invented epsilon.

Snow's original `1e-8` geometry gate exposed OCC bounding-box padding in the
transient thermal mesh launcher. Commit `677127c` uses the existing exact planar
vertex API and preserves the gate. `snow-source-diagnostic-1` then passes six
native solves, eight rejections, decreasing n2/n4/n8 continuum errors and
separate fixed-mesh temporal differences with observed order `0.9987997501061895`.
Maximum energy error is `0.019576357114523216` below the unchanged `0.02` limit.
This explicitly recorded read-only source overlay uses the recovery native
dependencies and does not qualify the old packaged adapter or a sandbox.

All commands, report hashes, ABI/source-overlay identities, failures and scoped
results are consolidated in [continuation evidence](evidence/cpu-native-diagnostics-20261007.json).
The complete CPU gate and pinned generator/Nix checks pass at the spectral/thermal
repair source; the additional real-CLI refusal regression passes separately.
Build 52 includes both new atmospheric-spectral outputs and refused evaluation
at 11 GiB combined headroom against the normal 24 GiB start requirement. Latest
read-only headroom increased to 15.7 GiB, still below that gate. Exact production
packages and repaired contact/v11/freezing/snow/spectral/atmospheric worker
campaigns remain pending; development diagnostics do not promote them.
