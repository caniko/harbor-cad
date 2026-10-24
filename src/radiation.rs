//! Explicit synthetic UV irradiance, optical weighting and prescribed exposure.
//! This prepares independent references; native transport execution is separate.
use crate::{
    Result,
    contracts::{digest, invalid},
    science::Quantity,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SANDBOX_POLICY: &str = "harbor-cad-spectral-cpu-v1";

fn field_units(spec: &SpectralReferenceSpec) -> String {
    format!(
        "sample:1,x_m:m,y_m:m,z_m:m,native_cosine:1,{}",
        (0..spec.wavelengths.len())
            .map(|i| format!("emitter_weight_w_m2_nm_{i}:W/(m2*nm)"))
            .collect::<Vec<_>>()
            .join(",")
    )
}

pub(crate) fn annotate_fields(
    plan: &crate::contracts::ExecutionPlan,
    artifacts: &mut [crate::contracts::ArtifactManifest],
) -> Result<()> {
    let Some(spec) = &plan.spectral else {
        return Ok(());
    };
    let units = field_units(spec);
    for seed in spec.seeds {
        let path = format!("stages/spectral/directional-{seed}.csv");
        let mut records = artifacts.iter_mut().filter(|a| a.path == path);
        let record = records
            .next()
            .ok_or_else(|| invalid("complete retained native spectral seed originals required"))?;
        if records.next().is_some() || record.format != "csv" || record.bytes == 0 {
            return Err(invalid(
                "unique complete native directional CSV per seed required",
            ));
        }
        record.units = Some(units.clone());
        record.time_s = None;
        record.association = Some("native_surface_sample".into());
        record.provenance = format!(
            "authoritative scalar_spectral Float32 directional emitter knots/position/cosine at native seed {seed}; Float64 optical/dose reductions; seeds are not physical times"
        );
    }
    Ok(())
}

pub(crate) fn verify_registered(
    store: &crate::storage::Store,
    id: &str,
    plan: &crate::contracts::ExecutionPlan,
    value: &serde_json::Value,
) -> Result<crate::qualification::NumericalEvidence> {
    let spec = plan
        .spectral
        .as_ref()
        .ok_or_else(|| invalid("approved spectral worker recipe required"))?;
    let originals = value["observations"]
        .as_array()
        .filter(|v| v.len() == spec.seeds.len())
        .ok_or_else(|| invalid("complete registered spectral observations required"))?;
    for observation in originals {
        let seed = observation["seed"]
            .as_u64()
            .filter(|v| spec.seeds.iter().any(|s| u64::from(*s) == *v))
            .ok_or_else(|| invalid("approved spectral seed required"))?;
        let path = format!("stages/spectral/directional-{seed}.csv");
        let record = store
            .artifact_record(id, &path)?
            .ok_or_else(|| invalid("registered original native directional spectral CSV absent"))?;
        let actual = crate::storage::native_manifest(
            &store.job_dir(id)?,
            &path,
            u64::from(spec.samples) * (256 + 32 * spec.wavelengths.len() as u64),
            "verify immutable spectral originals",
        )?;
        if record.format != "csv"
            || record.sha256 != actual.sha256
            || record.bytes != actual.bytes
            || record.time_s.is_some()
            || record.association.as_deref() != Some("native_surface_sample")
            || record.units.as_deref() != Some(field_units(spec).as_str())
            || observation["sha256"] != record.sha256
            || observation["bytes"].as_u64() != Some(record.bytes)
        {
            return Err(invalid(
                "native spectral originals differ from committed registered identities",
            ));
        }
    }
    crate::spectral_fields::verify(spec, &store.job_dir(id)?.join("stages/spectral"), value)
}

impl crate::contracts::ExecutionPlan {
    pub fn spectral_reference(spec: SpectralReferenceSpec, policy: String) -> Result<Self> {
        use crate::contracts::*;
        let mut plan = Self {
            schema_version: 13,
            case: None,
            source: None,
            frames: None,
            filter: None,
            fem: None,
            thermal: None,
            cad_source: None,
            imported_fem: None,
            wetting: None,
            contact: None,
            thermal_contact: None,
            freezing: None,
            spectral: Some(spec),
            atmosphere: None,
            atmospheric_transport: None,
            cad_variant: None,
            cad_transport: None,
            retained_cooling: None,
            stages: vec![
                Stage {
                    id: "spectral".into(),
                    dependencies: vec![],
                    operation: StageOperation::SpectralReference,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 1,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec!["spectral".into()],
                    operation: StageOperation::Bundle,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 16 * 1024 * 1024,
                    vram_bytes: 0,
                },
            ],
            transfers: vec![],
            observation: ObservationPlan {
                metrics: vec![],
                probes: vec![],
                retained_times_s: vec![],
                checkpoint_times_s: vec![],
                preview_times_s: vec![],
                max_artifact_bytes: 0,
                scientific_congestion: "fail".into(),
                preview_may_drop: false,
            },
            fleetix_revision: FLEETIX_REV.into(),
            fleetix_contract_digest: fleetix_digest(),
            policy,
        };
        crate::estimates::minimum(&plan)?.apply(&mut plan);
        plan.validate()?;
        Ok(plan)
    }
}

crate::json::tagged_enum! {
"kind";
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpectralSource {
    Directional {
        propagation_direction: [f64; 3],
        irradiance: Vec<Quantity>,
    },
    Isotropic {
        radiance: Vec<Quantity>,
    },
}

}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RadiantHistoryPoint {
    pub time: Quantity,
    pub scale: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpectralReferenceSpec {
    pub schema_version: u32,
    pub synthetic: bool,
    pub backend: String,
    pub variant: String,
    pub precision: String,
    pub wavelengths: Vec<Quantity>,
    pub source: SpectralSource,
    pub source_provenance: String,
    pub sensor_width: Quantity,
    pub sensor_height: Quantity,
    pub sensor_normal: [f64; 3],
    pub occlusion: String,
    pub absorptivity: Vec<f64>,
    pub optical_provenance: String,
    pub ageing_action: Vec<f64>,
    pub ageing_provenance: String,
    pub history: Vec<RadiantHistoryPoint>,
    pub history_interpolation: String,
    pub history_provenance: String,
    pub samples: u32,
    pub seeds: [u32; 3],
    pub relative_tolerance: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NormalizedSpectralReference {
    pub wavelengths_m: Vec<f64>,
    /// Per metre spectral density: W/(m2*m) or W/(m2*sr*m).
    pub source_values_si: Vec<f64>,
    pub source_unit: String,
    pub sensor_size_m: [f64; 2],
    pub sensor_area_m2: f64,
    pub angular_reference_factor: f64,
    pub history_times_s: Vec<f64>,
    pub history_scales: Vec<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreparedSpectralReference {
    pub schema_version: u32,
    pub preparation_id: String,
    pub input: SpectralReferenceSpec,
    pub normalized: NormalizedSpectralReference,
    pub incident_irradiance_w_m2: f64,
    pub absorbed_irradiance_w_m2: f64,
    pub ageing_weighted_irradiance_w_m2: f64,
    pub incident_power_w: f64,
    pub absorbed_power_w: f64,
    pub integrated_history_scale_s: f64,
    pub incident_exposure_j_m2: f64,
    pub absorbed_exposure_j_m2: f64,
    pub ageing_weighted_exposure_j_m2: f64,
    pub executed: bool,
    pub physical_validation: String,
    pub limitations: Vec<String>,
}
fn unit_vector(v: [f64; 3]) -> bool {
    v.into_iter().all(f64::is_finite)
        && (v.into_iter().map(|x| x * x).sum::<f64>() - 1.).abs() <= 1e-12
}

/// Separate strict envelope; no extra capability can be injected into v1 inputs.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpectralReflectionSpec {
    pub schema_version: u32,
    pub formulation: String,
    pub incident: SpectralReferenceSpec,
    pub disk_radius: Quantity,
    pub sensor_height: Quantity,
    pub reflectance: f64,
    pub reflectance_provenance: String,
    pub geometry_provenance: String,
    pub maximum_model_error: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreparedSpectralReflection {
    pub schema_version: u32,
    pub preparation_id: String,
    pub input: SpectralReflectionSpec,
    pub incident_reference: PreparedSpectralReference,
    pub disk_radius_m: f64,
    pub sensor_height_m: f64,
    pub disk_view_factor: f64,
    pub reflection_factor: f64,
    pub sensor_view_factor_bounds: [f64; 2],
    pub black_sensor_shadow_relative_error_bound: f64,
    pub model_relative_error_bound: f64,
    pub incident_irradiance_w_m2: f64,
    pub absorbed_irradiance_w_m2: f64,
    pub ageing_weighted_irradiance_w_m2: f64,
    pub incident_power_w: f64,
    pub absorbed_power_w: f64,
    pub incident_exposure_j_m2: f64,
    pub absorbed_exposure_j_m2: f64,
    pub ageing_weighted_exposure_j_m2: f64,
    pub executed: bool,
    pub physical_validation: String,
    pub limitations: Vec<String>,
}
impl SpectralReflectionSpec {
    pub fn prepare(&self) -> Result<PreparedSpectralReflection> {
        let incident = self.incident.prepare()?;
        if self.schema_version != 1
            || self.formulation != "isotropic_lambertian_disk"
            || !matches!(self.incident.source, SpectralSource::Isotropic { .. })
            || self.incident.sensor_normal != [0., 0., -1.]
            || self.incident.occlusion != "none"
            || !self.reflectance.is_finite()
            || !(0. ..=1.).contains(&self.reflectance)
            || !self.maximum_model_error.is_finite()
            || self.maximum_model_error <= 0.
            || self.maximum_model_error > 1e-5
            || self.maximum_model_error > self.incident.relative_tolerance / 8.
            || [&self.reflectance_provenance, &self.geometry_provenance]
                .into_iter()
                .any(|p| p.trim().is_empty() || p.len() > 4096)
        {
            return Err(invalid(
                "strict synthetic isotropic UV Lambertian disk with explicit reflectance, downward black sensor and independent bounded model error required",
            ));
        }
        let radius = self.disk_radius.si("length")?;
        let height = self.sensor_height.si("length")?;
        if !(0.1..=100.).contains(&radius) || !(0.1..=10.).contains(&height) {
            return Err(invalid(
                "bounded positive circular disk radius and sensor height required",
            ));
        }
        let size = incident.normalized.sensor_size_m;
        let offset = size[0].hypot(size[1]) / 2.;
        if offset >= radius {
            return Err(invalid(
                "complete sensor footprint must be inside the circular reflection fixture",
            ));
        }
        let view = |r: f64| r * r / (r * r + height * height);
        let factor = view(radius);
        let bounds = [view(radius - offset), view(radius + offset)];
        let reflection = 1. - (1. - self.reflectance) * factor;
        // For every rectangle point, centred disks R-a and R+a bound the
        // projected solid angle. The black sensor removes at most A/h²
        // incoming solid angle from the isotropic disk illumination.
        let footprint =
            (1. - self.reflectance) * (factor - bounds[0]).max(bounds[1] - factor) / reflection;
        let shadow = self.reflectance * bounds[1] * incident.normalized.sensor_area_m2
            / (std::f64::consts::PI * height * height * reflection);
        let model = footprint + shadow;
        if !model.is_finite() || model > self.maximum_model_error {
            return Err(invalid(
                "finite sensor footprint/shadow exceed the unchanged independent reflection model-error limit",
            ));
        }
        let mut report=PreparedSpectralReflection {schema_version:1,preparation_id:String::new(),input:self.clone(),
            disk_radius_m:radius,sensor_height_m:height,disk_view_factor:factor,reflection_factor:reflection,
            sensor_view_factor_bounds:bounds,black_sensor_shadow_relative_error_bound:shadow,model_relative_error_bound:model,
            incident_irradiance_w_m2:incident.incident_irradiance_w_m2*reflection,
            absorbed_irradiance_w_m2:incident.absorbed_irradiance_w_m2*reflection,
            ageing_weighted_irradiance_w_m2:incident.ageing_weighted_irradiance_w_m2*reflection,
            incident_power_w:incident.incident_power_w*reflection,absorbed_power_w:incident.absorbed_power_w*reflection,
            incident_exposure_j_m2:incident.incident_exposure_j_m2*reflection,
            absorbed_exposure_j_m2:incident.absorbed_exposure_j_m2*reflection,
            ageing_weighted_exposure_j_m2:incident.ageing_weighted_exposure_j_m2*reflection,
            incident_reference:incident,executed:false,physical_validation:"unqualified".into(),limitations:vec![
                "finite centred upward Lambertian disk under isotropic radiance; constant explicit UV reflectance and a downward black planar sensor".into(),
                "uncovered projected solid angle retains the original isotropic environment; finite sensor footprint and illumination shadow have separate conservative model-error bounds".into(),
                "native transport, sampling refinement, atmospheric inputs, imported optics and physical validation require separate qualification".into()]};
        report.preparation_id = digest(&report)?;
        Ok(report)
    }
}
// Exact product integral for two piecewise-linear functions on one shared grid.
// Endpoint-product trapezoids are not exact when optical weights vary.
pub(crate) fn product_integral(x: &[f64], a: &[f64], b: &[f64]) -> f64 {
    x.windows(2)
        .enumerate()
        .map(|(i, x)| {
            let da = a[i + 1] - a[i];
            let db = b[i + 1] - b[i];
            (x[1] - x[0]) * (a[i] * b[i] + (a[i] * db + b[i] * da) / 2. + da * db / 3.)
        })
        .sum()
}
impl SpectralReferenceSpec {
    pub(crate) fn validate_plan(&self, plan: &crate::contracts::ExecutionPlan) -> Result<()> {
        use crate::contracts::StageOperation;
        self.prepare()?;
        if !matches!(self.source, SpectralSource::Directional { .. }) {
            return Err(crate::Error::Unqualified("version-13 directional worker retains native CSV knots; hemispherical/reflected EXR requires separate original-reader worker qualification".into()));
        }
        if plan.policy == "ci"
            || plan.stages.len() != 2
            || plan.stages[0].id != "spectral"
            || plan.stages[0].operation != StageOperation::SpectralReference
            || !plan.stages[0].dependencies.is_empty()
            || plan.stages[1].id != "bundle"
            || plan.stages[1].operation != StageOperation::Bundle
            || plan.stages[1].dependencies != ["spectral"]
            || !plan.transfers.is_empty()
            || !plan.observation.metrics.is_empty()
            || !plan.observation.probes.is_empty()
            || !plan.observation.retained_times_s.is_empty()
            || !plan.observation.preview_times_s.is_empty()
            || !plan.observation.checkpoint_times_s.is_empty()
            || plan.observation.preview_may_drop
        {
            return Err(invalid(
                "exact independent native directional UV and bundle DAG with retained seeds, complete original knots and prescribed dose required",
            ));
        }
        Ok(())
    }
    pub fn prepare(&self) -> Result<PreparedSpectralReference> {
        let n = self.wavelengths.len();
        if self.schema_version != 1
            || !self.synthetic
            || self.backend != "cpu"
            || self.variant != "scalar_spectral"
            || self.precision != "Float32"
            || !(2..=64).contains(&n)
            || self.absorptivity.len() != n
            || self.ageing_action.len() != n
            || self
                .absorptivity
                .iter()
                .chain(&self.ageing_action)
                .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
            || !unit_vector(self.sensor_normal)
            || !matches!(
                self.occlusion.as_str(),
                "none" | "full_directional_occluder"
            )
            || !(2..=32).contains(&self.history.len())
            || self.history_interpolation != "piecewise_linear_prescribed_scale"
            || self
                .history
                .iter()
                .any(|point| !point.scale.is_finite() || point.scale < 0. || point.scale > 1e6)
            || !self.samples.is_power_of_two()
            || !(1024..=65536).contains(&self.samples)
            || self.seeds[0] >= self.seeds[1]
            || self.seeds[1] >= self.seeds[2]
            || !self.relative_tolerance.is_finite()
            || self.relative_tolerance <= 0.
            || self.relative_tolerance > 0.02
            || [
                &self.source_provenance,
                &self.optical_provenance,
                &self.ageing_provenance,
                &self.history_provenance,
            ]
            .into_iter()
            .any(|p| p.trim().is_empty() || p.len() > 4096)
        {
            return Err(invalid(
                "bounded synthetic scalar Float32 spectral reference with explicit UV properties, angular source, complete prescribed history and unchanged acceptance required",
            ));
        }
        let wavelengths = self
            .wavelengths
            .iter()
            .map(|q| q.si("length"))
            .collect::<Result<Vec<_>>>()?;
        if wavelengths.iter().any(|v| !(200e-9..=2500e-9).contains(v))
            || wavelengths
                .windows(2)
                .any(|p| p[0] >= p[1] || (p[1] - p[0]) < 1e-12)
        {
            return Err(invalid(
                "strictly ordered explicitly sampled 200–2500 nm spectrum with no extrapolation required",
            ));
        }
        let size = [
            self.sensor_width.si("length")?,
            self.sensor_height.si("length")?,
        ];
        if size
            .into_iter()
            .any(|v| !v.is_finite() || !(1e-5..=0.01).contains(&v))
        {
            return Err(invalid(
                "finite bounded planar sensor dimensions in metres required",
            ));
        }
        let (values, dimension, unit, factor) = match &self.source {
            SpectralSource::Directional {
                propagation_direction,
                irradiance,
            } => {
                if !unit_vector(*propagation_direction) {
                    return Err(invalid(
                        "explicit unit propagation direction required; angular source cannot be inferred from scalar irradiance",
                    ));
                }
                let cosine = -propagation_direction
                    .iter()
                    .zip(self.sensor_normal)
                    .map(|(a, b)| a * b)
                    .sum::<f64>();
                (
                    irradiance,
                    "spectral_irradiance",
                    "W/(m2*m)",
                    if self.occlusion == "full_directional_occluder" {
                        0.
                    } else {
                        cosine.max(0.)
                    },
                )
            }
            SpectralSource::Isotropic { radiance } => {
                if self.occlusion != "none" {
                    return Err(invalid(
                        "directional full-cover screen cannot authorize hemispherical diffuse occlusion",
                    ));
                }
                (
                    radiance,
                    "spectral_radiance",
                    "W/(m2*sr*m)",
                    std::f64::consts::PI,
                )
            }
        };
        let values = values
            .iter()
            .map(|q| q.si(dimension))
            .collect::<Result<Vec<_>>>()?;
        if values.len() != n
            || values
                .iter()
                .any(|v| !v.is_finite() || *v < 0. || *v > 1e15)
            || values.iter().all(|v| *v == 0.)
        {
            return Err(invalid(
                "complete nonnegative finite spectral source and explicit spectral density units required",
            ));
        }
        let times = self
            .history
            .iter()
            .map(|p| p.time.si("time"))
            .collect::<Result<Vec<_>>>()?;
        if times[0] != 0.
            || times.windows(2).any(|p| p[0] >= p[1])
            || *times.last().unwrap() > 86400. * 365.
        {
            return Err(invalid(
                "complete strictly ordered prescribed exposure from t=0 with bounded duration required",
            ));
        }
        let scales = self.history.iter().map(|p| p.scale).collect::<Vec<_>>();
        let time_integral = times
            .windows(2)
            .enumerate()
            .map(|(i, p)| (p[1] - p[0]) * (scales[i] + scales[i + 1]) / 2.)
            .sum::<f64>();
        let incident = factor * product_integral(&wavelengths, &values, &vec![1.; n]);
        let absorbed = factor * product_integral(&wavelengths, &values, &self.absorptivity);
        let ageing = factor * product_integral(&wavelengths, &values, &self.ageing_action);
        let area = size[0] * size[1];
        if [
            incident,
            absorbed,
            ageing,
            time_integral,
            incident * time_integral,
            absorbed * time_integral,
            ageing * time_integral,
        ]
        .into_iter()
        .any(|v| !v.is_finite() || v < 0.)
        {
            return Err(invalid(
                "finite nonnegative spectral irradiance and radiant exposure integrals required",
            ));
        }
        let mut report=PreparedSpectralReference{schema_version:1,preparation_id:String::new(),input:self.clone(),
            normalized:NormalizedSpectralReference{wavelengths_m:wavelengths,source_values_si:values,source_unit:unit.into(),sensor_size_m:size,sensor_area_m2:area,
                angular_reference_factor:factor,history_times_s:times,history_scales:scales},
            incident_irradiance_w_m2:incident,absorbed_irradiance_w_m2:absorbed,ageing_weighted_irradiance_w_m2:ageing,
            incident_power_w:incident*area,absorbed_power_w:absorbed*area,integrated_history_scale_s:time_integral,
            incident_exposure_j_m2:incident*time_integral,absorbed_exposure_j_m2:absorbed*time_integral,ageing_weighted_exposure_j_m2:ageing*time_integral,
            executed:false,physical_validation:"unqualified".into(),limitations:vec![
                "synthetic fixed-angular source; no atmospheric derivation, sky-map reduction or imported CAD".into(),
                "piecewise-linear spectra and prescribed temporal amplitude; interpolation is the declared model, not validation of solar history sampling".into(),
                "absorbed heating, incident dose and dimensionless ageing-weighted exposure remain distinct; no temperature or ageing lifetime inference".into(),
                "Float32 scalar transport is a declared CPU reference; no CUDA, Vulkan/HIP or GPU transport qualification".into()]};
        report.preparation_id = digest(&report)?;
        Ok(report)
    }
}
