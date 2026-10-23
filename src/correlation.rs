//! Score measured physical-test series against one registered native thermal
//! history. Correlation of a tested condition is not validation of untested ones.
use crate::{
    Result,
    contracts::{invalid, token},
    results::SampleLocation,
    science::Quantity,
    storage::{Store, safe_path},
    thermal_results::{self, ThermalField, ThermalSampleRequest},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_CSV_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_ROWS: usize = 1_000_000;
pub const MAX_PROBES: usize = 64;
/// (time_s, SI value) pairs, strictly time-ordered.
pub type Series = Vec<(f64, f64)>;
const HEADER: &str = "time,probe,value,unit";
const CLAIM: &str = "correlation of this tested condition only; not validation of untested geometry, materials, environments or operating histories";

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Tolerance {
    /// Absolute error allowance, a temperature interval (K or delta_degC).
    pub abs: Quantity,
    /// Relative allowance applied to |measured| in SI (Kelvin for temperature).
    pub rel: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeasurementProvenance {
    pub test_id: String,
    pub rig: String,
    pub sensor_type: String,
    /// Stated sensor accuracy, a temperature interval; reported, never hidden.
    pub sensor_accuracy: Quantity,
    pub date: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CorrelationRequest {
    pub schema_version: u32,
    pub job_id: String,
    /// CSV path relative to the worker's allowed input root.
    pub measurement_csv: String,
    pub dimension: String,
    /// Required for a coupled thermal/contact source; omitted for standalone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_stage: Option<u32>,
    /// Measured probe name to a registered native thermal node.
    pub probe_map: BTreeMap<String, SampleLocation>,
    /// Added to measured time to obtain simulated time.
    pub time_offset_s: f64,
    /// Inclusive window in simulated time; must lie inside the simulated history.
    pub time_window: [f64; 2],
    pub tolerance: Tolerance,
    pub provenance: MeasurementProvenance,
}
impl CorrelationRequest {
    pub fn validate(&self) -> Result<()> {
        let p = &self.provenance;
        let text = |s: &str| !s.trim().is_empty() && s.len() <= 256;
        if self.schema_version != 1
            || uuid::Uuid::parse_str(&self.job_id).is_err()
            || self.measurement_csv.is_empty()
            || self.probe_map.is_empty()
            || self.probe_map.len() > MAX_PROBES
            || self.probe_map.keys().any(|k| !token(k))
            || !self.time_offset_s.is_finite()
            || !self.time_window.iter().all(|t| t.is_finite())
            || self.time_window[0] >= self.time_window[1]
            || !self.tolerance.rel.is_finite()
            || self.tolerance.rel < 0.
            || ![&p.test_id, &p.rig, &p.sensor_type, &p.date]
                .iter()
                .all(|s| text(s))
        {
            return Err(invalid(
                "versioned correlation request with 1–64 token probes, finite ordered window, non-negative tolerance and complete provenance required",
            ));
        }
        if self.dimension != "temperature" {
            return Err(invalid(
                "only temperature correlates against registered thermal histories",
            ));
        }
        for q in [&self.tolerance.abs, &p.sensor_accuracy] {
            if q.si("temperature_interval")? < 0. {
                return Err(invalid("non-negative tolerance/accuracy required"));
            }
        }
        Ok(())
    }
}

/// Parsed measurements in SI, keyed by probe, each strictly time-ordered.
#[derive(Clone, Debug)]
pub struct Measurements {
    pub sha256: String,
    pub rows: usize,
    pub probes: BTreeMap<String, Series>,
}

pub fn parse_measurements(data: &[u8], dimension: &str) -> Result<Measurements> {
    if data.len() as u64 > MAX_CSV_BYTES {
        return Err(invalid("measurement CSV exceeds 64 MiB"));
    }
    let text = std::str::from_utf8(data).map_err(|_| invalid("UTF-8 measurement CSV required"))?;
    let mut lines = text.strip_suffix('\n').unwrap_or(text).split('\n');
    let strip = |l: &'_ str| l.strip_suffix('\r').unwrap_or(l).to_owned();
    if lines.next().map(strip).as_deref() != Some(HEADER) {
        return Err(invalid(format!("measurement CSV header must be {HEADER}")));
    }
    let mut probes: BTreeMap<String, Series> = BTreeMap::new();
    let mut rows = 0;
    for (index, line) in lines.enumerate() {
        let row = index + 2;
        let line = line.strip_suffix('\r').unwrap_or(line);
        rows += 1;
        if rows > MAX_ROWS {
            return Err(invalid("measurement CSV exceeds 1e6 rows"));
        }
        let fields: Vec<_> = line.split(',').collect();
        let [time, probe, value, unit] = fields[..] else {
            return Err(invalid(format!("row {row}: exactly four fields required")));
        };
        let number = |s: &str| s.trim().parse::<f64>().ok().filter(|v| v.is_finite());
        let (Some(time), Some(value)) = (number(time), number(value)) else {
            return Err(invalid(format!(
                "row {row}: finite time and value required"
            )));
        };
        if !token(probe) {
            return Err(invalid(format!("row {row}: token probe name required")));
        }
        let value = Quantity {
            value,
            unit: unit.trim().into(),
        }
        .si(dimension)
        .map_err(|e| invalid(format!("row {row}: {e}")))?;
        if !probes.contains_key(probe) && probes.len() == MAX_PROBES {
            return Err(invalid("measurement CSV exceeds 64 probes"));
        }
        let series = probes.entry(probe.into()).or_default();
        if series.last().is_some_and(|(t, _)| *t >= time) {
            return Err(invalid(format!(
                "row {row}: strictly increasing time per probe required"
            )));
        }
        series.push((time, value));
    }
    if rows == 0 {
        return Err(invalid("measurement CSV has no rows"));
    }
    Ok(Measurements {
        sha256: format!("{:x}", Sha256::digest(data)),
        rows,
        probes,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Metrics {
    pub n: usize,
    /// Mean of simulated minus measured, SI.
    pub bias: f64,
    pub mae: f64,
    pub rmse: f64,
    pub max_abs_error: f64,
    /// Measured-clock time of the maximum absolute error.
    pub max_abs_error_time_s: f64,
    pub failures: usize,
    pub pass: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProbeReport {
    pub measured_probe: String,
    pub simulated: SampleLocation,
    pub excluded_outside_window: usize,
    pub metrics: Metrics,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SimulationSource {
    pub job_id: String,
    pub science_id: String,
    pub execution_id: String,
    pub execution_binding_digest: String,
    pub native_artifact_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_stage: Option<u32>,
    /// Approved initial condition (t = 0) followed by every retained snapshot.
    pub snapshot_times_s: Vec<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CorrelationReport {
    pub schema_version: u32,
    pub job_id: String,
    pub dimension: String,
    pub unit: String,
    pub error_definition: String,
    pub interpolation: String,
    pub time_offset_s: f64,
    pub time_window_s: [f64; 2],
    pub tolerance: Tolerance,
    pub tolerance_abs_si: f64,
    pub tolerance_rule: String,
    pub measurement_csv_sha256: String,
    pub measurement_rows: usize,
    pub provenance: MeasurementProvenance,
    pub sensor_accuracy_si: f64,
    pub uncertainty_treatment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulation: Option<SimulationSource>,
    pub probes: Vec<ProbeReport>,
    pub overall: Metrics,
    pub pass: bool,
    pub claim: String,
}

/// Calibration objective: pooled RMSE over every probe, SI.
pub fn objective(report: &CorrelationReport) -> f64 {
    report.overall.rmse
}

fn check_probes(request: &CorrelationRequest, measured: &Measurements) -> Result<()> {
    if let Some(p) = measured
        .probes
        .keys()
        .find(|p| !request.probe_map.contains_key(*p))
    {
        return Err(invalid(format!("measured probe {p} is not mapped")));
    }
    if let Some(p) = request
        .probe_map
        .keys()
        .find(|p| !measured.probes.contains_key(*p))
    {
        return Err(invalid(format!("mapped probe {p} has no measurements")));
    }
    Ok(())
}

fn interpolate(series: &[(f64, f64)], t: f64) -> f64 {
    let i = series.partition_point(|(s, _)| *s < t);
    if series[i].0 == t {
        return series[i].1;
    }
    let ((t0, v0), (t1, v1)) = (series[i - 1], series[i]);
    v0 + (v1 - v0) * (t - t0) / (t1 - t0)
}

#[derive(Default)]
struct Accumulator {
    n: usize,
    sum: f64,
    abs: f64,
    sq: f64,
    max: f64,
    max_t: f64,
    failures: usize,
}
impl Accumulator {
    fn push(&mut self, error: f64, time: f64, allowed: f64) {
        self.n += 1;
        self.sum += error;
        self.abs += error.abs();
        self.sq += error * error;
        if self.n == 1 || error.abs() > self.max {
            self.max = error.abs();
            self.max_t = time;
        }
        self.failures += usize::from(error.abs() > allowed);
    }
    fn metrics(&self) -> Metrics {
        let n = self.n as f64;
        Metrics {
            n: self.n,
            bias: self.sum / n,
            mae: self.abs / n,
            rmse: (self.sq / n).sqrt(),
            max_abs_error: self.max,
            max_abs_error_time_s: self.max_t,
            failures: self.failures,
            pass: self.failures == 0,
        }
    }
}

/// Pure scoring. `simulated` maps each measured probe to its SI simulated
/// series in simulated time; values are interpolated only inside its range.
pub fn correlate(
    request: &CorrelationRequest,
    measured: &Measurements,
    simulated: &BTreeMap<String, Series>,
) -> Result<CorrelationReport> {
    request.validate()?;
    check_probes(request, measured)?;
    let abs = request.tolerance.abs.si("temperature_interval")?;
    let [w0, w1] = request.time_window;
    let mut overall = Accumulator::default();
    let mut probes = Vec::with_capacity(request.probe_map.len());
    for (name, location) in &request.probe_map {
        let series = simulated
            .get(name)
            .filter(|s| s.len() >= 2)
            .ok_or_else(|| invalid(format!("simulated series for {name} required")))?;
        if !series.iter().all(|(t, v)| t.is_finite() && v.is_finite())
            || series.windows(2).any(|w| w[0].0 >= w[1].0)
        {
            return Err(invalid(
                "strictly time-ordered finite simulated series required",
            ));
        }
        if w0 < series[0].0 || w1 > series[series.len() - 1].0 {
            return Err(invalid(
                "time window outside simulated history; extrapolation is forbidden",
            ));
        }
        let mut probe = Accumulator::default();
        let mut excluded = 0;
        for &(time, value) in &measured.probes[name] {
            let t = time + request.time_offset_s;
            if !(w0..=w1).contains(&t) {
                excluded += 1;
                continue;
            }
            let error = interpolate(series, t) - value;
            let allowed = abs + request.tolerance.rel * value.abs();
            probe.push(error, time, allowed);
            overall.push(error, time, allowed);
        }
        if probe.n == 0 {
            return Err(invalid(format!(
                "probe {name} has no measurements inside the time window"
            )));
        }
        probes.push(ProbeReport {
            measured_probe: name.clone(),
            simulated: location.clone(),
            excluded_outside_window: excluded,
            metrics: probe.metrics(),
        });
    }
    let overall = overall.metrics();
    Ok(CorrelationReport {
        schema_version: 1,
        job_id: request.job_id.clone(),
        dimension: request.dimension.clone(),
        unit: "K".into(),
        error_definition: "simulated minus measured, SI".into(),
        interpolation: "linear between simulated snapshots onto measured times + offset, inside the window only; no extrapolation".into(),
        time_offset_s: request.time_offset_s,
        time_window_s: request.time_window,
        tolerance: request.tolerance.clone(),
        tolerance_abs_si: abs,
        tolerance_rule: "|error| <= abs + rel * |measured_si|".into(),
        measurement_csv_sha256: measured.sha256.clone(),
        measurement_rows: measured.rows,
        provenance: request.provenance.clone(),
        sensor_accuracy_si: request.provenance.sensor_accuracy.si("temperature_interval")?,
        uncertainty_treatment: "sensor accuracy is reported separately; it is neither subtracted from errors nor added to the tolerance".into(),
        simulation: None,
        pass: overall.pass,
        probes,
        overall,
        claim: CLAIM.into(),
    })
}

/// Worker entry: read the bounded CSV under `input_root`, sample the verified
/// registered thermal history at every retained time and score it.
pub fn run(
    store: &Store,
    input_root: &std::path::Path,
    request: &CorrelationRequest,
) -> Result<CorrelationReport> {
    request.validate()?;
    let data = crate::worker::read_bounded(
        &safe_path(input_root, &request.measurement_csv)?,
        MAX_CSV_BYTES,
    )?;
    let measured = parse_measurements(&data, &request.dimension)?;
    check_probes(request, &measured)?;
    let plan = store.plan(&request.job_id)?;
    let (spec, _) = thermal_results::source_stage(&plan, request.thermal_stage)?;
    let locations: Vec<SampleLocation> = request
        .probe_map
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut by_location: BTreeMap<&SampleLocation, Vec<(f64, f64)>> = locations
        .iter()
        .map(|l| (l, vec![(0., spec.initial_temperature_k)]))
        .collect();
    let mut source = None;
    // ponytail: each retained time re-verifies the whole native history (≤64×);
    // a single-pass multi-time verifier would remove the repetition.
    for &time in &spec.observation_times_s {
        let report = thermal_results::sample(
            store,
            &ThermalSampleRequest {
                schema_version: 1,
                job_id: request.job_id.clone(),
                field: ThermalField::Temperature,
                physical_time_s: time,
                thermal_stage: request.thermal_stage,
                locations: locations.clone(),
            },
        )?
        .sample;
        for sample in &report.samples {
            let [value] = sample.value[..] else {
                return Err(invalid("scalar native temperature required"));
            };
            by_location
                .get_mut(&sample.location)
                .ok_or_else(|| invalid("native sample location mismatch"))?
                .push((time, value));
        }
        source.get_or_insert(SimulationSource {
            job_id: report.job_id,
            science_id: report.science_id,
            execution_id: report.execution_id,
            execution_binding_digest: report.execution_binding_digest,
            native_artifact_sha256: report.native_artifact.sha256,
            thermal_stage: request.thermal_stage,
            snapshot_times_s: std::iter::once(0.)
                .chain(spec.observation_times_s.iter().copied())
                .collect(),
        });
    }
    let simulated = request
        .probe_map
        .iter()
        .map(|(name, location)| (name.clone(), by_location[location].clone()))
        .collect();
    let mut report = correlate(request, &measured, &simulated)?;
    report.simulation = source;
    Ok(report)
}
