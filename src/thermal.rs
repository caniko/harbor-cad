//! Explicit synthetic transient CPU heat transfer; prescribed SI inputs only.
use crate::{Result, contracts::*, recipes::MoistureRisk};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThermalReferenceSpec {
    pub schema_version: u32,
    pub synthetic: bool,
    pub backend: String,
    pub formulation: String,
    pub size_m: [f64; 3],
    pub resolution: u32,
    pub geometry_tolerance_m: f64,
    pub initial_temperature_k: f64,
    pub density_kg_m3: f64,
    pub specific_heat_j_kg_k: f64,
    pub conductivity_w_m_k: f64,
    pub material_temperature_domain_k: [f64; 2],
    pub convection_w_m2_k: f64,
    pub duration_s: f64,
    pub max_step_s: f64,
    pub integration_substeps: u32,
    pub observation_times_s: Vec<f64>,
    pub ambient_history: Vec<[f64; 2]>,
    pub heater_history: Vec<[f64; 2]>,
    pub numerical_tolerance: f64,
    pub energy_tolerance: f64,
    pub geometry_provenance: String,
    pub material_provenance: String,
    pub history_provenance: String,
    pub convection_provenance: String,
    pub moisture_risk: MoistureRisk,
}

impl ThermalReferenceSpec {
    pub fn integration_step(&self) -> f64 {
        self.max_step_s / f64::from(self.integration_substeps)
    }
    pub fn output_times(&self) -> Vec<f64> {
        let count = (self.duration_s / self.max_step_s).ceil() as u32;
        let mut times: Vec<_> = (1..=count)
            .map(|i| self.duration_s * f64::from(i) / f64::from(count))
            .collect();
        times.extend_from_slice(&self.observation_times_s);
        times.extend(
            self.ambient_history
                .iter()
                .chain(&self.heater_history)
                .filter_map(|pair| (pair[0] > 0.).then_some(pair[0])),
        );
        times.sort_by(f64::total_cmp);
        times.dedup();
        times
    }
    pub fn heater_energy(&self, stamp: f64) -> Result<f64> {
        if !stamp.is_finite() || stamp < 0. || stamp > self.duration_s {
            return Err(invalid("no heater-history extrapolation"));
        }
        let mut total = 0.;
        for pair in self.heater_history.windows(2) {
            let [left, a] = pair[0];
            let [right, b] = pair[1];
            let end = stamp.min(right);
            if end > left {
                total += (end - left) * (a + a + (b - a) * (end - left) / (right - left)) / 2.;
            }
        }
        if !total.is_finite() || total < 0. {
            return Err(invalid("finite prescribed heater-energy integral required"));
        }
        Ok(total)
    }
    pub fn validate(&self) -> Result<()> {
        let positive = |v: f64| v.is_finite() && v > 0.;
        let minimum = self.size_m.iter().copied().fold(f64::INFINITY, f64::min);
        let maximum = self.size_m.iter().copied().fold(0., f64::max);
        if self.schema_version != 1
            || !self.synthetic
            || self.backend != "cpu"
            || self.formulation != "plane_wall_robin"
            || !self.size_m.iter().copied().all(positive)
            || maximum / minimum > 1000.
            || !(2..=32).contains(&self.resolution)
            || !self.geometry_tolerance_m.is_finite()
            || self.geometry_tolerance_m < 1e-10
            || self.geometry_tolerance_m >= 0.001 * minimum
            || ![
                self.initial_temperature_k,
                self.density_kg_m3,
                self.specific_heat_j_kg_k,
                self.conductivity_w_m_k,
                self.duration_s,
                self.max_step_s,
            ]
            .into_iter()
            .all(positive)
            || self.duration_s / self.max_step_s > 1024.
            || self.max_step_s > self.duration_s
            || !(1..=64).contains(&self.integration_substeps)
            || ![self.numerical_tolerance, self.energy_tolerance]
                .into_iter()
                .all(|v| positive(v) && v <= 0.02)
        {
            return Err(invalid(
                "bounded explicit synthetic SI CPU thermal recipe and unchanged acceptance required",
            ));
        }
        for text in [
            &self.geometry_provenance,
            &self.material_provenance,
            &self.history_provenance,
            &self.convection_provenance,
        ] {
            if text.trim().is_empty() || text.len() > 4096 {
                return Err(invalid(
                    "bounded explicit geometry/material/history/convection provenance required",
                ));
            }
        }
        for (history, ambient) in [(&self.ambient_history, true), (&self.heater_history, false)] {
            if !(2..=64).contains(&history.len())
                || history[0][0] != 0.
                || history.last().unwrap()[0] != self.duration_s
            {
                return Err(invalid(
                    "full bounded prescribed histories required; no extrapolation",
                ));
            }
            let mut previous = -1.;
            for [time, value] in history {
                if !time.is_finite()
                    || *time < 0.
                    || *time <= previous
                    || !value.is_finite()
                    || *value < 0.
                    || (ambient && *value == 0.)
                {
                    return Err(invalid(
                        "ordered physical times and absolute ambient/nonnegative heater SI values required",
                    ));
                }
                previous = *time;
            }
        }
        let diffusivity =
            self.conductivity_w_m_k / (self.density_kg_m3 * self.specific_heat_j_kg_k);
        let capacity =
            self.density_kg_m3 * self.specific_heat_j_kg_k * self.size_m.iter().product::<f64>();
        let biot = self.convection_w_m2_k * self.size_m[0] / (2. * self.conductivity_w_m_k);
        if !positive(diffusivity)
            || !positive(capacity)
            || !self.convection_w_m2_k.is_finite()
            || self.convection_w_m2_k < 0.
            || (self.convection_w_m2_k > 0. && !(1e-5..=1000.).contains(&biot))
        {
            return Err(invalid(
                "finite capacity/diffusivity and prescribed bounded convection required",
            ));
        }
        if !(1..=64).contains(&self.observation_times_s.len())
            || self.observation_times_s.last().copied() != Some(self.duration_s)
        {
            return Err(invalid(
                "bounded physical observations through the final time required",
            ));
        }
        let mut previous = 0.;
        for stamp in &self.observation_times_s {
            if !stamp.is_finite()
                || *stamp <= previous
                || *stamp > self.duration_s
                || (self.convection_w_m2_k > 0.
                    && diffusivity * stamp / (self.size_m[0] / 2.).powi(2) < 1e-4)
            {
                return Err(invalid(
                    "ordered positive resolved physical-time observations required",
                ));
            }
            previous = *stamp;
        }
        let times = self.output_times();
        if times.len() as u64 * (u64::from(self.resolution) + 1).pow(3) * 48 > 64 * 1024 * 1024
            || times
                .iter()
                .scan(0., |last, stamp| {
                    let delta = stamp - *last;
                    *last = *stamp;
                    Some(delta)
                })
                .any(|delta| delta < 1e-6 * self.duration_s)
        {
            return Err(invalid(
                "bounded native field output and distinct printed times required",
            ));
        }
        let [low, high] = self.material_temperature_domain_k;
        let lower = self
            .ambient_history
            .iter()
            .fold(self.initial_temperature_k, |a, p| a.min(p[1]));
        let upper = self
            .ambient_history
            .iter()
            .fold(self.initial_temperature_k, |a, p| a.max(p[1]))
            + self.heater_energy(self.duration_s)? / capacity;
        if !positive(low)
            || !positive(high)
            || low >= high
            || !upper.is_finite()
            || lower < low
            || upper > high
        {
            return Err(invalid(
                "full thermal bound must remain inside declared material temperature domain",
            ));
        }
        if self.heater_energy(self.duration_s)? == 0.
            && (self.convection_w_m2_k == 0.
                || self
                    .ambient_history
                    .iter()
                    .all(|p| p[1] == self.initial_temperature_k))
        {
            return Err(invalid("nontrivial explicit transient forcing required"));
        }
        match &self.moisture_risk {
            MoistureRisk::Missing { .. } | MoistureRisk::Inapplicable { .. } => {
                self.moisture_risk.inspect()?;
            }
            _ => {
                return Err(invalid(
                    "native cold reference supports explicit missing moisture or justified inapplicability only",
                ));
            }
        }
        crate::snow::verify_binding(self)?;
        Ok(())
    }
}

impl ExecutionPlan {
    pub fn thermal_reference(spec: ThermalReferenceSpec, policy: String) -> Result<Self> {
        spec.validate()?;
        let retained_times_s = spec.observation_times_s.clone();
        let mut plan = Self {
            schema_version: 6,
            case: None,
            fem: None,
            thermal: Some(spec),
            cad_source: None,
            imported_fem: None,
            wetting: None,
            contact: None,
            thermal_contact: None,
            freezing: None,
            spectral: None,
            atmosphere: None,
            atmospheric_transport: None,
            cad_variant: None,
            cad_sheet_variant: None,
            cad_transport: None,
            retained_cooling: None,
            cad_solids: None,
            source: None,
            frames: None,
            filter: None,
            stages: vec![
                Stage {
                    id: "thermal".into(),
                    dependencies: vec![],
                    operation: StageOperation::ThermalReference,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 2 * 1024 * 1024 * 1024,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec!["thermal".into()],
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
                retained_times_s,
                checkpoint_times_s: vec![],
                preview_times_s: vec![],
                max_artifact_bytes: 256 * 1024 * 1024,
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

pub fn temperature_patch_sha256() -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(include_bytes!(
            "../nix/patches/calculix-temperature-precision.patch"
        ))
    )
}

pub fn verify_receipt(
    plan: &ExecutionPlan,
    value: &serde_json::Value,
) -> Result<crate::qualification::NumericalEvidence> {
    let spec = plan
        .thermal
        .as_ref()
        .ok_or_else(|| invalid("approved thermal recipe required"))?;
    verify_spec(spec, value)
}

pub(crate) fn verify_spec(
    spec: &ThermalReferenceSpec,
    value: &serde_json::Value,
) -> Result<crate::qualification::NumericalEvidence> {
    spec.validate()?;
    let n = u64::from(spec.resolution);
    if value["schema_version"] != 1
        || value["adapter"] != "CalculiX"
        || value["backend"] != "cpu"
        || value["factorization"] != "SPOOLES"
        || value["executed"] != true
        || value["software_fallback"] != false
        || value["synthetic"] != true
        || value["precision"] != "float64"
        || value["request_sha256"] != digest(spec)?
        || value["formulation"] != spec.formulation
        || value["calculix_version"] != "2.23"
        || value["gmsh_version"] != "4.15.2"
        || value["calculix_source_sha256"]
            != "9c88385c10fb04f5dc6c4e98027a51bebdd8aee3920e05190d6c1dd08357d6e7"
        || value["gmsh_source_sha256"]
            != "be3f66f225d27ba9fa014f07e83169285da8a051b0e8ab7103d88066b39bdd3e"
        || value["temperature_serialization_patch_sha256"] != temperature_patch_sha256()
        || value["temperature_serialization"]
            != "E23.15; 16 significant decimal digits from native real*8"
        || value["nodes"] != (n + 1).pow(3)
        || value["elements"] != n.pow(3)
        || value["physical_validation"] != "unqualified"
        || value["moisture_risk"] != serde_json::to_value(&spec.moisture_risk)?
        || value["physical_times_s"] != serde_json::to_value(&spec.observation_times_s)?
        || value["energy_output_times_s"] != serde_json::to_value(spec.output_times())?
        || value["maximum_native_step_s"].as_f64() != Some(spec.integration_step())
        || value["integration_substeps"] != spec.integration_substeps
    {
        return Err(invalid(
            "native transient thermal recipe, stack, precision or physical-time identities changed",
        ));
    }
    let names = [
        "operation_closure_only",
        "no_gpu_nodes",
        "no_sysfs",
        "no_host_home",
        "no_session_bus",
        "no_worker_socket",
        "network_namespace_isolated",
        "descriptor_readonly",
    ];
    let sandbox = &value["sandbox"];
    if sandbox["policy"] != crate::execution::THERMAL_SANDBOX_POLICY
        || sandbox["checks"]
            .as_object()
            .is_none_or(|m| m.len() != names.len())
        || names.iter().any(|name| sandbox["checks"][name] != true)
    {
        return Err(invalid(
            "complete exact operation-specific thermal CPU sandbox evidence required",
        ));
    }
    let checks = value["numerical_verification"]
        .as_object()
        .ok_or_else(|| invalid("native transient temperature and energy verification required"))?;
    if checks.len() != 2 {
        return Err(invalid("exact complete thermal numerical checks required"));
    }
    let mut utilization: f64 = 0.;
    let mut references = Vec::new();
    for (key, error_key, tolerance, unit, samples) in [
        (
            "temperature",
            "normalized_max_abs_error",
            spec.numerical_tolerance,
            "K",
            (n + 1).pow(3) * spec.observation_times_s.len() as u64,
        ),
        (
            "energy",
            "maximum_relative_balance_error",
            spec.energy_tolerance,
            "J",
            spec.output_times().len() as u64,
        ),
    ] {
        let check = checks
            .get(key)
            .ok_or_else(|| invalid("native transient field check missing"))?;
        let error = check[error_key]
            .as_f64()
            .filter(|e| e.is_finite() && *e >= 0. && *e <= tolerance)
            .ok_or_else(|| invalid("unchanged native transient numerical or energy gate failed"))?;
        let reference = check["reference"]
            .as_str()
            .filter(|v| !v.is_empty() && v.len() <= 1024)
            .ok_or_else(|| invalid("independent thermal numerical reference required"))?;
        if check["passed"] != true
            || check["tolerance"].as_f64() != Some(tolerance)
            || check["unit"] != unit
            || check["samples"] != samples
        {
            return Err(invalid(
                "thermal acceptance, field units or complete coverage changed",
            ));
        }
        utilization = utilization.max(error / tolerance);
        references.push(format!(
            "{key}: {reference}; error={error}, approved tolerance={tolerance}"
        ));
    }
    let mut scope = "synthetic transient constant-property plane wall; separate temperature and cumulative energy gates".to_string();
    if let Some(boundary) = crate::snow::verify_binding(spec)? {
        scope.push_str(&format!("; prescribed full-face dry snow series resistance {}; omitted snow heat capacity ratio={}, diffusion/forcing ratio={}; no deposition, melting or blocked-opening flow", boundary.preparation_id, boundary.omitted_capacity_ratio, boundary.diffusion_timescale_ratio));
    }
    Ok(crate::qualification::NumericalEvidence {
        reference: references.join("; "),
        scope,
        error_kind: "maximum_approved_gate_fraction".into(),
        error: utilization,
        tolerance: 1.,
    })
}
