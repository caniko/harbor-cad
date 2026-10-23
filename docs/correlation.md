# Measured-data correlation

`harbor-cad results correlate REQUEST.json` and results/all-profile MCP
`results_correlate` score a physical test log (thermocouple or climate-chamber
data) against one succeeded, numerically verified registered thermal history.
Both go through the same read-only Rust worker. The report is the input a later
calibration of uncertain inputs (contact conductance, film coefficient,
emissivity) can minimize via `correlation::objective` (pooled RMSE, SI).

## Measurement CSV

UTF-8, at most 64 MiB, 1e6 data rows and 64 distinct probes:

```text
time,probe,value,unit
0,tc1,21.4,degC
5,tc1,21.6,degC
0,tc2,294.7,K
```

`time` is seconds on the test clock. `probe` is a token (`[A-Za-z0-9_-]`, at most
128 bytes). `value`/`unit` are converted with the shared SI table for the
declared dimension (`degC`, `K` for temperature). Rows may interleave probes, but
time must strictly increase per probe. Non-finite numbers, unknown units, extra
or missing fields and blank lines refuse.

The CSV path is relative to the worker profile's `allowed_input_root`; absolute,
traversing and symlinked paths refuse. The report retains the CSV SHA-256.

## Request

```json
{"schema_version":1,"job_id":"<uuid>","measurement_csv":"chamber/run7.csv",
 "dimension":"temperature","thermal_stage":0,
 "probe_map":{"tc1":{"association":"node","node_id":12}},
 "time_offset_s":-30.0,"time_window":[60.0,3600.0],
 "tolerance":{"abs":{"value":1.0,"unit":"delta_degC"},"rel":0.0},
 "provenance":{"test_id":"CC-007","rig":"chamber A","sensor_type":"type T thermocouple",
   "sensor_accuracy":{"value":0.5,"unit":"K"},"date":"2026-10-01"}}
```

`thermal_stage` follows the `sample-thermal` rule: required for a coupled
thermal/contact source, omitted for standalone. Every CSV probe must be mapped
and every mapped probe measured; mapped nodes must exist in the native result.
Only `temperature` is supported, because the simulated source is a Kelvin
thermal history. Provenance strings are required.

## Alignment and metrics

The simulated series per mapped node is the approved initial condition (t = 0)
plus every approved retained snapshot, each verified against the native DAT.
A measurement at test time `t` is compared at simulated time `t + time_offset_s`
when that lies in the inclusive `time_window`. The window must lie inside the
simulated history: values are linearly interpolated between snapshots, never
extrapolated. A probe without a measurement inside the window refuses.

Error is simulated minus measured, in SI. Per probe and pooled overall: `n`,
`bias` (mean error), `mae`, `rmse`, `max_abs_error` with its test-clock time,
`failures` and `pass`, where a point passes if
`|error| <= abs + rel * |measured_si|` (relative to Kelvin for temperature).

`sensor_accuracy_si` is reported as an explicit uncertainty term. It is neither
subtracted from the errors nor added to the tolerance; decide explicitly whether
an RMSE comparable to sensor accuracy is meaningful.

## What it does not claim

A pass shows agreement for this rig, test, sensor placement and history only.
It is not validation of untested geometry, materials, environments or operating
histories, and it does not upgrade the source job's qualification. Linear
interpolation between sparse retained snapshots adds error the report does not
separate from model error; retain enough snapshots for the transients you score.
