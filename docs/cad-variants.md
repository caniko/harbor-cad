# Controlled CAD-copy variants

`cad variant REQUEST.json` and CAD/all MCP `cad_plan_variant` prepare a distinct
version-16 approval for one registered, originally authorized CAD document.
Ordinary `job submit` executes the approved variant through the patched,
closure-only FreeCAD importer sandbox. Preparation opens no CAD document.

The first allowlist accepts a document containing exactly one top-level native
`Part::Box`, identified by its registered object name, with no expressions,
linked objects or assembly context. Explicit length/width/height quantities,
geometry tolerance and provenance are required. The original axis-aligned
placement is preserved. Native execution checks the original dimensions, world
bounds and volume against registered source evidence before changing the
allowlisted dimension properties, recomputes, rechecks all named geometry and
exports a new FCStd, BREP and controlled STL. Face numbering and healing are
never used as identity.

Original document bytes and the original science/execution/binding/authorization
identities are bound in the new approval. Distinct-inode verified source copies
and provenance are retained before submission acknowledgment, under independent
shared staging admission. Only the copy is opened; the source is mounted
read-only and native execution verifies its checksum again after saving the new
document. Interrupted unpublished staging is recovered only while the original
registered source remains verifiable; otherwise originals remain quarantined.

Rust reconstructs expected volume, world bounds and unchanged placement from
the original source and explicit dimensions. It checks the native original and
recomputed region receipts before publication and repeats registered-byte checks
for historical qualification. Native geometry execution does not constitute a
physics solve, mesh convergence or physical validation. Assemblies, rotations,
arbitrary object properties, spreadsheet formulas and multi-solid variants are
outside this initial allowlist.

`examples/cad-variant.json` is an explicit request template. Replace its all-zero
source UUID with an authorized single-box inspection job; dimensions never
change automatically to fit a resource limit. Source CPU contract/allowlist and
staging tests are independent of native package and lifecycle qualification.

The native allowlist follows the pinned FreeCAD
[`Part::Box` implementation](https://github.com/FreeCAD/FreeCAD/blob/4fd3bf320d9566a27e60069fc8387448aaa3a094/src/Mod/Part/App/FeaturePartBox.cpp):
`Length`, `Width` and `Height` are native properties, recomputation builds an
OpenCASCADE box from their values and the native primitive preserves placement.
The importer uses the same pinned document/placement APIs as the qualified
original import. World-space region and BREP semantics are rechecked after the
edit rather than relying on native property names alone.

`scripts/verify_cad_variant_worker.py` is the opt-in exact-package gate. It
generates pinned native origin/translated boxes, inspects their originals,
checks CLI/MCP planning parity and runs controlled copies after independent
approval. Pre-submit substitution is refused; acknowledged original copies
survive subsequent source mutation and a worker crash under the same service
invocation. New documents are opened by a second independent patched import to
check saved geometry/placement, followed by owned cancellation/forced-death
and reservation/runtime-root release. Development packages are refused before
creating a campaign output. This gate does not pass until executed successfully.

The shared campaign API regression includes this entrypoint. A real stdio MCP
client/worker test checks its CAD profile and expected refusal path; source gate
21 passes all 189 Python cases. These tests repaired a missing shared call and
the original inspection's plan-only MCP response shape before native execution.

## Version 2: spreadsheet-alias variants

A request with `schema_version: 2` goes through the same `cad variant` /
`cad_plan_variant` entrypoint and becomes a version-16 plan carrying
`cad_sheet_variant` instead of `cad_variant` (exactly one of the two):

```json
{"schema_version": 2, "source_job": "<uuid>", "sheet_name": "Params",
 "parameters": {"wall": {"value": 4.0, "unit": "mm"}, "fin_count": {"value": 10, "unit": "1"}},
 "regions": ["housing", "lid"], "geometry_tolerance": {"value": 1e-5, "unit": "m"},
 "provenance": "..."}
```

Planning opens no document. The source job must have registered `sheets.json`
alias evidence, written by the importer whenever a document contains a
`Spreadsheet::Sheet`. It must list exactly one sheet, with the requested name.
Only aliases that already exist may be set; at most 32 per request. Values are
lengths (`m`, `mm`, `nm`, 1e-5 to 10 m) or dimensionless (`1`, 0 to 1e6), and
must keep the alias's registered dimension. The adapter writes cell text from
these validated numbers only; caller text never reaches FreeCAD. Requested
regions must be registered in the source job. An empty `parameters` map
re-exports the baseline with metrics. Source identity, approval binding,
distinct-inode staging, orphan recovery and historical byte rechecks reuse the
version-1 path.

Expressions are the edit mechanism, so v2 replaces the "no expressions"
guarantee with "only Spreadsheet cells change". Before editing, the native side
requires every object to be valid, not touched and error-free, and every
requested region to match its registered bounds, volume and placement. It then
snapshots every non-sheet object's type, expression bindings and unbound,
non-output input properties, plus every sheet cell's contents and alias. It
sets only the approved alias cells and compares the snapshot before
recomputing: other objects must be identical and other cells unchanged. After
`recompute()` it refuses any invalid, touched or errored object, any change to
non-computed state, and any alias value that did not land exactly. It then
saves `variant.FCStd` and re-exports the named regions through the inspection
path (BREP, STL, `regions.json`, `brep-manifest.json`).

`cad-variant-recompute.json` (schema 2) records the cell changes and the
original (`before`) and recomputed (`after`) bounds, volume and transform per
region. `after` also carries `surface_area_m2` and `min_wall_m`. The wall
estimate fires one inward face-normal ray per face, from the centroid of the
face's largest tessellation triangle, and reports the shortest material chord
starting at that probe. It is deterministic and bounded to 2048 faces. It can
miss a thin spot away from those probes, so treat it as a screening value, not
a guaranteed minimum. Before publication, Rust checks that every requested
region is present, that volumes are finite and positive and no larger than
their bounding box, that bounds are ordered and agree with `regions.json`, that
area satisfies the isoperimetric bound, and that `min_wall_m` is positive and
no larger than the smallest extent.

### Enclosure seed fixture

`harbor-cad cad enclosure-fixture` plans the fixed `enclosure_fixture` bridge
operation; submit it with `job submit`. It builds a deterministic ~200×150×80 mm
outdoor enclosure from native Part primitives and booleans only, with no Python
feature proxies. Every dimension is an expression on the single `Params` sheet:
length, width, height, wall, fin_count, fin_height, fin_pitch, fin_thickness,
groove_width, groove_depth, lid_overlap, window_width, window_height,
drain_diameter, boss_count and pcb_power_area_side. The named solids are
`housing` (shell, back fins, PCB bosses, rim groove, drain hole), `lid`
(overlapping skirt, window aperture), `gasket` (ring filling the groove),
`window` (pane), `pcb` (heat-source block) and `mount` (L wall bracket). Fins
and bosses use fixed slots (16 and 4); higher counts clamp. The fixture is
synthetic geometry, not a validated product design.

The rim groove is centred in the wall, so each rim lip beside it is
(wall − groove_width)/2. Planning a v2 variant of an `enclosure_fixture`
source refuses an effective wall below groove_width + 1.5 mm (two 0.75 mm
lips) with the `geometry_precondition` error code; the effective value is the
requested edit, else the source sheet's registered value. Thinner lips are
sub-millimetre slivers: in the first pilot, 2.5–3.0 mm walls left 0.25–0.5 mm
lips that failed the 0.05 mesh quality gate (minimum 0.013–0.0495 at 6 mm) and
the process minimum wall. A design loop records the refusal as a blocked child
(`chained planning refused: geometry precondition: …`), so the candidate stays
unknown and never reaches the mesher.

The same planner refuses a window pane that does not sit inside the cavity
opening with 10 mm clearance per side: window_width + 2 × (wall + 10 mm) must
not exceed length, and window_height + 2 × (wall + 10 mm) must not exceed
width (effective values as above; unknown values refuse). A wider pane would
rest on the housing rim and gasket, or cut through the lid skirt, which the
fixture does not model as a window.
`examples/enclosure-variant.json` is a v2 request template for it.
