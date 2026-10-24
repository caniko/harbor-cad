//! Explicit pinned molecular UV atmosphere and retained native angular samples.
pub(crate) use crate::atmosphere_fields::{annotate_fields, registered, verify};
use crate::{
    Result,
    contracts::{digest, invalid},
    science::Quantity,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SOURCE_SHA256: &str = "64930cc40b6e4a37aa220520974d330fc1563796f466a649b2238131f2d69840";
pub const SANDBOX_POLICY: &str = "harbor-cad-atmosphere-cpu-v1";
pub const ORIGINAL_PATH: &str = "stages/atmosphere/uvspec-original.txt";
pub const FIELD_ASSOCIATION: &str = "wavelength_propagation_solid_angle_sample";
pub const FIELD_UNITS: &str =
    "lambda:nm,edir:W/(m2*nm),edn:W/(m2*nm),eup:W/(m2*nm),uu:W/(m2*sr*nm)";
pub fn profile_sha256(profile: &str) -> Result<&'static str> {
    match profile {
        "afglms" => Ok("875ada621ca86fb24ed49bbca44e540e9bfe81e2a2af7760a8ce7de991652b14"),
        "afglmw" => Ok("4425063a390b9c19f286abb051fcc98fda5cdc014c9e701a6be226e9932e816c"),
        _ => Err(invalid(
            "supported content-pinned atmospheric profile required",
        )),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AtmosphericReferenceSpec {
    pub schema_version: u32,
    pub synthetic: bool,
    pub backend: String,
    pub solver: String,
    pub model: String,
    pub profile: String,
    pub wavelengths: Vec<Quantity>,
    pub toa_irradiance: Vec<Quantity>,
    pub solar_zenith_deg: f64,
    pub solar_azimuth_deg: f64,
    pub albedo: f64,
    pub streams: u32,
    pub mu_bins: u32,
    pub phi_bins: u32,
    pub relative_tolerance: f64,
    pub source_provenance: String,
    pub atmosphere_provenance: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreparedAtmosphericReference {
    pub schema_version: u32,
    pub executed: bool,
    pub request_sha256: String,
    pub source_sha256: String,
    pub wavelengths_nm: Vec<f64>,
    pub toa_irradiance_w_m2_nm: Vec<f64>,
    /// Propagation in explicit East, North, Up coordinates, not a view direction.
    pub propagation_direction: [f64; 3],
    pub umu: Vec<f64>,
    pub phi_deg: Vec<f64>,
    pub angular_cell_solid_angle_sr: f64,
    pub transparent_horizontal_reference_w_m2_nm: Option<Vec<f64>>,
    pub physical_validation: String,
}

impl AtmosphericReferenceSpec {
    pub(crate) fn validate_plan(&self, plan: &crate::contracts::ExecutionPlan) -> Result<()> {
        use crate::contracts::StageOperation;
        self.prepare()?;
        if plan.policy == "ci"
            || plan.stages.len() != 2
            || plan.stages[0].id != "atmosphere"
            || plan.stages[0].operation != StageOperation::AtmosphericReference
            || !plan.stages[0].dependencies.is_empty()
            || plan.stages[1].id != "bundle"
            || plan.stages[1].operation != StageOperation::Bundle
            || plan.stages[1].dependencies != ["atmosphere"]
            || !plan.transfers.is_empty()
            || !plan.observation.metrics.is_empty()
            || !plan.observation.probes.is_empty()
            || !plan.observation.retained_times_s.is_empty()
            || !plan.observation.preview_times_s.is_empty()
            || !plan.observation.checkpoint_times_s.is_empty()
            || plan.observation.preview_may_drop
        {
            return Err(invalid(
                "exact native atmospheric original angular sphere and bundle DAG required; wavelength/angle are not physical time",
            ));
        }
        Ok(())
    }
    pub fn prepare(&self) -> Result<PreparedAtmosphericReference> {
        let bins = [8, 16, 32, 64];
        if self.schema_version != 1
            || !self.synthetic
            || self.backend != "cpu"
            || self.solver != "disort"
            || !["clear_sky_molecular_crs", "transparent_reference"].contains(&self.model.as_str())
            || !["afglms", "afglmw"].contains(&self.profile.as_str())
            || !bins.contains(&self.streams)
            || !bins.contains(&self.mu_bins)
            || !bins.contains(&self.phi_bins)
            || !self.solar_zenith_deg.is_finite()
            || !(0. ..=80.).contains(&self.solar_zenith_deg)
            || !self.solar_azimuth_deg.is_finite()
            || !(0. ..360.).contains(&self.solar_azimuth_deg)
            || !self.albedo.is_finite()
            || !(0. ..=1.).contains(&self.albedo)
            || (self.model == "transparent_reference" && self.albedo != 0.)
            || !self.relative_tolerance.is_finite()
            || self.relative_tolerance <= 0.
            || self.relative_tolerance > 0.02
            || !(2..=64).contains(&self.wavelengths.len())
            || self.toa_irradiance.len() != self.wavelengths.len()
            || [&self.source_provenance, &self.atmosphere_provenance]
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 2048)
        {
            return Err(invalid(
                "explicit bounded synthetic DISORT molecular UV/transparent model, atmospheric/solar provenance and unchanged angular/analytic gate required",
            ));
        }
        let wavelengths_nm = self
            .wavelengths
            .iter()
            .map(|q| Ok(q.si("length")? * 1e9))
            .collect::<Result<Vec<_>>>()?;
        if wavelengths_nm.iter().any(|w| {
            *w < 280. - 1e-12 || *w > 400. + 1e-12 || (w * 1000. - (w * 1000.).round()).abs() > 1e-6
        }) || wavelengths_nm
            .windows(2)
            .any(|w| (w[0] * 1000.).round() >= (w[1] * 1000.).round())
        {
            return Err(invalid(
                "2–64 ordered original UV knots in supported molecular-cross-section band 280–400 nm on the native 0.001 nm decimal-output grid required",
            ));
        }
        let toa = self
            .toa_irradiance
            .iter()
            .map(|q| Ok(q.si("spectral_irradiance")? * 1e-9))
            .collect::<Result<Vec<_>>>()?;
        if toa.iter().any(|v| *v < 0. || *v > 1e6) || !toa.iter().any(|v| *v > 0.) {
            return Err(invalid(
                "explicit finite nonnegative top-of-atmosphere irradiance with a nonzero UV knot required",
            ));
        }
        let zenith = self.solar_zenith_deg.to_radians();
        let azimuth = self.solar_azimuth_deg.to_radians();
        let umu = (0..2 * self.mu_bins)
            .map(|i| -1. + (f64::from(i) + 0.5) / f64::from(self.mu_bins))
            .collect();
        let phi_deg = (0..self.phi_bins)
            .map(|i| (f64::from(i) + 0.5) * 360. / f64::from(self.phi_bins))
            .collect();
        let reference = (self.model == "transparent_reference")
            .then(|| toa.iter().map(|e| e * zenith.cos()).collect());
        Ok(PreparedAtmosphericReference {
            schema_version: 1,
            executed: false,
            request_sha256: digest(self)?,
            source_sha256: SOURCE_SHA256.into(),
            wavelengths_nm,
            toa_irradiance_w_m2_nm: toa,
            propagation_direction: [
                zenith.sin() * azimuth.sin(),
                zenith.sin() * azimuth.cos(),
                -zenith.cos(),
            ],
            umu,
            phi_deg,
            angular_cell_solid_angle_sr: 2. * std::f64::consts::PI
                / (f64::from(self.mu_bins) * f64::from(self.phi_bins)),
            transparent_horizontal_reference_w_m2_nm: reference,
            physical_validation: "unqualified".into(),
        })
    }
}

impl crate::contracts::ExecutionPlan {
    pub fn atmospheric_reference(spec: AtmosphericReferenceSpec, policy: String) -> Result<Self> {
        use crate::contracts::*;
        let mut plan = Self {
            schema_version: 14,
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
            spectral: None,
            atmosphere: Some(spec),
            atmospheric_transport: None,
            cad_variant: None,
            cad_sheet_variant: None,
            cad_transport: None,
            retained_cooling: None,
            cad_solids: None,
            stages: vec![
                Stage {
                    id: "atmosphere".into(),
                    dependencies: vec![],
                    operation: StageOperation::AtmosphericReference,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 1,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec!["atmosphere".into()],
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

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AtmosphericObservations {
    pub direct_horizontal_w_m2_nm: Vec<f64>,
    pub direct_normal_w_m2_nm: Vec<f64>,
    pub diffuse_downward_w_m2_nm: Vec<f64>,
    pub diffuse_upward_w_m2_nm: Vec<f64>,
    pub angular_order: String,
    pub maximum_angular_flux_error: f64,
    pub tolerance: f64,
    pub physical_validation: String,
}

// Streaming compensated Float64 reduction, preserving native Float32 originals.
fn add(total: &mut f64, correction: &mut f64, value: f64) {
    let y = value - *correction;
    let next = *total + y;
    *correction = (next - *total) - y;
    *total = next;
}

pub fn observations(
    spec: &AtmosphericReferenceSpec,
    text: &str,
) -> Result<AtmosphericObservations> {
    let prepared = spec.prepare()?;
    let mut result = AtmosphericObservations {
        direct_horizontal_w_m2_nm: vec![],
        direct_normal_w_m2_nm: vec![],
        diffuse_downward_w_m2_nm: vec![],
        diffuse_upward_w_m2_nm: vec![],
        angular_order:
            "umu-major phi-minor; full original propagation sphere; negative umu downwelling".into(),
        maximum_angular_flux_error: 0.,
        tolerance: spec.relative_tolerance,
        physical_validation: "unqualified".into(),
    };
    let mut rows = text.lines();
    for (wl, toa) in prepared
        .wavelengths_nm
        .iter()
        .zip(&prepared.toa_irradiance_w_m2_nm)
    {
        let row = rows
            .next()
            .ok_or_else(|| invalid("native atmospheric wavelength rows truncated"))?;
        let mut columns = row.split_whitespace();
        let mut number = || -> Result<f64> {
            columns
                .next()
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.)
                .ok_or_else(|| {
                    invalid("complete finite nonnegative native atmospheric columns required")
                })
        };
        let wave = number()?;
        let direct = number()?;
        let down = number()?;
        let up = number()?;
        if wave != (wl * 1000.).round() / 1000. {
            return Err(invalid("native atmospheric wavelength identity changed"));
        }
        let expected = toa * (-prepared.propagation_direction[2]);
        if direct > expected * (1. + spec.relative_tolerance)
            || (toa == &0. && (direct != 0. || down != 0. || up != 0.))
            || direct + down - up > expected * (1. + spec.relative_tolerance)
        {
            return Err(invalid(
                "native direct/net surface energy exceeds explicit TOA power",
            ));
        }
        let reflected = spec.albedo * (direct + down);
        if (reflected == 0. && up != 0.)
            || (reflected != 0. && (up / reflected - 1.).abs() > spec.relative_tolerance)
        {
            return Err(invalid(
                "native Lambertian surface reflection conservation failed",
            ));
        }
        let mut totals = [0.; 2];
        let mut corrections = [0.; 2];
        for mu in &prepared.umu {
            let hemisphere = usize::from(*mu > 0.);
            for _ in &prepared.phi_deg {
                let value = number()?;
                if toa == &0. && value != 0. {
                    return Err(invalid("zero source cannot create native angular radiance"));
                }
                add(
                    &mut totals[hemisphere],
                    &mut corrections[hemisphere],
                    value * mu.abs() * prepared.angular_cell_solid_angle_sr,
                );
            }
        }
        if columns.next().is_some() {
            return Err(invalid("unexpected native angular/wavelength columns"));
        }
        for (total, flux) in totals.into_iter().zip([down, up]) {
            if flux == 0. && total != 0. {
                return Err(invalid(
                    "zero native diffuse flux with nonzero angular energy",
                ));
            }
            let error = if flux == 0. {
                0.
            } else {
                (total / flux - 1.).abs()
            };
            if !error.is_finite() || error > spec.relative_tolerance {
                return Err(invalid(
                    "original atmospheric angular flux is unresolved at unchanged gate",
                ));
            }
            result.maximum_angular_flux_error = result.maximum_angular_flux_error.max(error);
        }
        if spec.model == "transparent_reference" {
            let error = if expected == 0. {
                0.
            } else {
                (direct / expected - 1.).abs()
            };
            if error > spec.relative_tolerance || down != 0. || up != 0. {
                return Err(invalid(
                    "native transparent cosine/zero diffuse reference failed",
                ));
            }
            result.maximum_angular_flux_error = result.maximum_angular_flux_error.max(error);
        }
        result.direct_horizontal_w_m2_nm.push(direct);
        result
            .direct_normal_w_m2_nm
            .push(direct / (-prepared.propagation_direction[2]));
        result.diffuse_downward_w_m2_nm.push(down);
        result.diffuse_upward_w_m2_nm.push(up);
    }
    if rows.next().is_some() {
        return Err(invalid("unexpected original native atmosphere rows"));
    }
    Ok(result)
}
