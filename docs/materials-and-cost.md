# Materials, processes and cost estimates

`data/materials.json` and `data/processes.json` hold typical published ranges
for an outdoor wall/pole-mounted electronics enclosure at pilot volume
(10–100 units). `src/manufacturing.rs` embeds them (`Library::builtin()`) and
provides three screens a design loop can call:

- `unit_cost` — per-unit USD low/high with a breakdown (material, process,
  setup, tooling) and an explicit `missing` list.
- `manufacturability` — material/process compatibility, envelope fit (any
  90° orientation) and minimum wall, as `violation`, `marginal` (inside the
  sourced spread) or `unknown` (data missing; never a pass).
- `service_margins` — temperature against the conservative end of the
  service limits, and von Mises stress against yield low / safety factor.

CLI: `harbor-cad materials list`, `harbor-cad materials show <id>`,
`harbor-cad cost estimate REQUEST.json` (a `CostRequest`; see
`harbor-cad schema`). These run locally and need no worker. The worker
exposes the same `materials_list` and `cost_estimate` operations, and so do the
MCP `simulation` and `all` profiles.

## Estimates, not quotes

The numbers are screening estimates built from datasheets and published cost
guides. They are not supplier quotes and not qualified material properties.
They leave out finishing, inserts, gaskets, assembly, inspection, shipping
and tax. Every estimate lists its `assumptions` and the `price_years` of the
money ranges it used. Get real quotes before committing to a process.

Model:

- **Material**: stock volume × density × USD/kg × `MATERIAL_SCRAP_FACTOR`
  (1.1, a stated assumption that is not sourced). Subtractive processes
  (`time_basis: removed_volume`) buy their bounding box as stock. If the
  process rate already includes material (`rate_includes_material`), the
  material term is 0.
- **Process**: `per_part` is the rate itself. `per_cm3` multiplies the rate by
  part volume. `per_hour` multiplies the rate by (processed volume ÷ sourced
  `throughput`), where the processed volume is the removed volume (bounding
  box minus part) or the part volume. This heuristic ignores feature count,
  finishing passes, nesting and orientation, so for real machine times use
  CAM or slicer output.
- **Setup**: `setup_cost` ÷ quantity.
- **Tooling**: ⌈quantity ÷ tool life⌉ tools × tool cost ÷ quantity. The low
  estimate pairs the cheap tool with the long life, and the high estimate
  pairs the expensive tool with the short life.

The low total is the sum of every term's low value, so it pairs the most
favourable ends of every range, and the same goes for the high total. If any
needed input is missing, `low` and `high` are `null`, the breakdown keeps the
terms that are known, and `missing` names the absent fields
(`<id>.<field>`). Nothing is filled in with a default.

## Provenance policy

- Every value is a `PhysicalInput`. A `known` value carries a `provenance`
  string, `"<url> | <title> | accessed <date> | <note>"` (multiple sources are
  joined with ` ;; `), and `synthetic: false`.
- If no source was found, the value is `{"availability": "missing",
  "reason": ...}`. Fields that do not apply, such as compression set for a
  metal, are `missing` with the reason `not applicable`. A process with no
  part-specific tooling uses `null` for `tooling_cost` and `tool_life_parts`.
- Ranges cover the spread across datasheets, grades and vendors. A single
  value is stored as low == high.
- Costs are in USD with a price `year`. The note records any currency
  conversion or derivation (for example, sheet price ÷ sheet mass).
- `Library::load` rejects duplicate ids, unknown material references, empty
  provenance, low > high, wrong units for a field's dimension, negative
  physical values, non-USD money and wrong money bases. The
  `builtin_library_loads_and_every_known_value_is_cited` test also requires
  that every built-in known value is non-synthetic and has a URL and an access
  date.

## Updating the data

1. Edit the JSON and change only values you have a source for. Update the
   provenance and its access date.
2. Run `cargo test --test manufacturing`.
3. Keep the units in the field table in `Material::quantities()`. Temperatures
   use `degC` or `K`, moduli and strengths `MPa`, fractions `"1"` (compression
   set 25 % → 0.25), throughput `cm3/h`, and lead time `d`.
4. A new process with a `per_hour` rate needs a sourced `throughput` and a
   `time_basis`. Other rate bases must set both to `null`.
