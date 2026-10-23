//! Input and SI conversion for the pinned conduction-induced Stefan reference.
//! Native OpenLB owns the total-enthalpy collision and phase-change coupling.
use crate::{Result, contracts::invalid, moisture_results::NativeMoistureAssessment};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SANDBOX_POLICY: &str = "harbor-cad-freezing-cpu-v1";
const FIELD_UNITS: &str =
    "i:1,j:1,x_m:m,y_m:m,material:1,specific_enthalpy_j_kg:J/kg,temperature_k:K,liquid_fraction:1";

pub(crate) fn annotate_fields(
    plan: &crate::contracts::ExecutionPlan,
    artifacts: &mut [crate::contracts::ArtifactManifest],
) -> Result<()> {
    let Some(spec) = &plan.freezing else {
        return Ok(());
    };
    let scale = spec.scale()?;
    for step in &spec.observation_steps {
        for extension in ["csv", "vti"] {
            let path = format!("stages/freezing/freezing-{step}.{extension}");
            let records = artifacts
                .iter_mut()
                .filter(|a| a.path == path)
                .collect::<Vec<_>>();
            if records.len() != 1 {
                return Err(invalid(
                    "each approved freezing CSV/VTK observation must be registered exactly once",
                ));
            }
            for record in records {
                if record.format != extension
                    || record.bytes == 0
                    || record.bytes > 16 * 1024 * 1024
                {
                    return Err(invalid("bounded freezing field artifact required"));
                }
                record.time_s = Some(*step as f64 * scale.physical_step_s);
                record.association = Some("native_lattice_point".into());
                record.units = Some(FIELD_UNITS.into());
                record.provenance = format!(
                    "complete original OpenLB Float64 enthalpy/temperature/phase fields at native step {step}; CSV is authoritative and VTK preserves all point values; boundary nodes have zero mass; interior control volume dx^2*extrusion from native-freezing-request.json; synthetic conduction reference"
                );
            }
        }
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
        .freezing
        .as_ref()
        .ok_or_else(|| invalid("approved freezing recipe required"))?;
    let hashes = value["original_files_sha256"]
        .as_object()
        .ok_or_else(|| invalid("original freezing/export hashes required"))?;
    let root = store.job_dir(id)?;
    let mut records = Vec::new();
    for (name, hash) in hashes {
        let path = format!("stages/freezing/{name}");
        let record = store
            .artifact_record(id, &path)?
            .ok_or_else(|| invalid("registered original freezing artifact absent"))?;
        let observed = crate::storage::native_manifest(
            &root,
            &path,
            32 * 1024 * 1024,
            "verify registered original freezing bytes",
        )?;
        if record.path != path
            || record.bytes == 0
            || record.bytes != observed.bytes
            || record.sha256 != observed.sha256
            || *hash != record.sha256
        {
            return Err(invalid(
                "registered freezing originals or export bytes changed",
            ));
        }
        records.push(record);
    }
    let recorded = records.clone();
    annotate_fields(plan, &mut records)?;
    if recorded
        .iter()
        .zip(&records)
        .any(|(a, b)| a.time_s != b.time_s || a.association != b.association || a.units != b.units)
    {
        return Err(invalid(
            "registered freezing field time, units or association changed",
        ));
    }
    crate::freezing_fields::verify(
        spec,
        &crate::storage::safe_path(&root, "stages/freezing")?,
        value,
    )
}

impl crate::contracts::ExecutionPlan {
    pub fn freezing_reference(spec: FreezingReferenceSpec, policy: String) -> Result<Self> {
        use crate::contracts::*;
        spec.scale()?;
        let mut plan = Self {
            schema_version: 12,
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
            observation: ObservationPlan {
                retained_times_s: spec.times_s()?,
                metrics: vec![],
                probes: vec![],
                checkpoint_times_s: vec![],
                preview_times_s: vec![],
                max_artifact_bytes: 0,
                scientific_congestion: "fail".into(),
                preview_may_drop: false,
            },
            freezing: Some(spec),
            spectral: None,
            atmosphere: None,
            atmospheric_transport: None,
            cad_variant: None,
            cad_sheet_variant: None,
            cad_transport: None,
            retained_cooling: None,
            stages: vec![
                Stage {
                    id: "freezing".into(),
                    dependencies: vec![],
                    operation: StageOperation::FreezingReference,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 0,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec!["freezing".into()],
                    operation: StageOperation::Bundle,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 0,
                    vram_bytes: 0,
                },
            ],
            transfers: vec![],
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
pub struct FreezingReferenceSpec {
    pub schema_version: u32,
    pub synthetic: bool,
    pub backend: String,
    pub formulation: String,
    /// Native x/y dimensions and explicitly prescribed out-of-plane extrusion.
    pub size_m: [f64; 3],
    pub resolution: u32,
    /// Equal constant properties in solid and liquid; no expansion or flow.
    pub density_kg_m3: f64,
    pub specific_heat_j_kg_k: f64,
    pub conductivity_w_m_k: f64,
    pub latent_heat_j_kg: f64,
    pub melting_temperature_k: f64,
    pub initial_temperature_k: f64,
    pub cold_wall_temperature_k: f64,
    pub material_temperature_domain_k: [f64; 2],
    pub steps: u64,
    pub observation_steps: Vec<u64>,
    pub front_tolerance: f64,
    pub temperature_tolerance: f64,
    pub mass_tolerance: f64,
    pub energy_tolerance: f64,
    pub material_provenance: String,
    pub boundary_provenance: String,
    pub geometry_provenance: String,
    pub moisture_risk: NativeMoistureAssessment,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FreezingScale {
    pub spacing_m: f64,
    pub physical_step_s: f64,
    pub duration_s: f64,
    pub stefan_number: f64,
    /// h = cp*(T-Tcold) + L*f_liquid. This is a declared energy zero.
    pub initial_specific_enthalpy_j_kg: f64,
    pub shape: [u32; 2],
    /// Uniform nodal control volumes; Dirichlet/reflecting shells have no mass.
    pub active_volume_m3: f64,
    pub cell_mass_kg: f64,
    pub active_control_bounds_m: [[f64; 2]; 3],
}

impl FreezingReferenceSpec {
    pub fn times_s(&self) -> Result<Vec<f64>> {
        let scale = self.scale()?;
        Ok(self
            .observation_steps
            .iter()
            .map(|step| *step as f64 * scale.physical_step_s)
            .collect())
    }
    pub(crate) fn validate_plan(&self, plan: &crate::contracts::ExecutionPlan) -> Result<()> {
        use crate::contracts::StageOperation;
        self.scale()?;
        if plan.policy == "ci"
            || plan.stages.len() != 2
            || plan.stages[0].id != "freezing"
            || plan.stages[1].id != "bundle"
            || plan.stages[0].operation != StageOperation::FreezingReference
            || plan.stages[1].operation != StageOperation::Bundle
            || !plan.stages[0].dependencies.is_empty()
            || plan.stages[1].dependencies != ["freezing"]
            || !plan.transfers.is_empty()
            || !plan.observation.metrics.is_empty()
            || !plan.observation.probes.is_empty()
            || plan.observation.retained_times_s != self.times_s()?
            || !plan.observation.checkpoint_times_s.is_empty()
            || !plan.observation.preview_times_s.is_empty()
            || plan.observation.preview_may_drop
        {
            return Err(invalid(
                "exact independent version-12 CPU solidification DAG and native physical observations required",
            ));
        }
        Ok(())
    }
    pub fn inspect(&self) -> Result<serde_json::Value> {
        let scale = self.scale()?;
        Ok(serde_json::json!({"valid":true,"executed":false,
            "science_id":crate::contracts::digest(self)?,"scale":scale,
            "moisture_risk":self.moisture_risk.inspect(self.cold_wall_temperature_k)?,
            "physical_validation":"unqualified",
            "scope":"synthetic fixed-volume conduction-induced solidification reference; retained-water transfer and native execution require separate qualification",
            "native_scaling":{"temperature":"(T-Tcold)/(Tm-Tcold)",
                "specific_enthalpy":"h/(cp*(Tm-Tcold)); energy zero at solid Tcold",
                "heat_descriptor":"D2Q5","relaxation_time":1.0,"diffusivity_lattice":1.0/6.0},
            "boundaries":{"xmin":"prescribed constant cold temperature","xmax":"insulated",
                "y":"periodic","velocity":"zero; conduction only","out_of_plane":"prescribed extrusion"}}))
    }

    pub fn scale(&self) -> Result<FreezingScale> {
        let positive = |v: f64| v.is_finite() && v > 0.;
        let n = u64::from(self.resolution);
        let span = self.melting_temperature_k - self.cold_wall_temperature_k;
        let domain = self.material_temperature_domain_k;
        if self.schema_version != 1
            || !self.synthetic
            || self.backend != "cpu"
            || self.formulation != "conduction_stefan_solidification_2d"
            || !(32..=256).contains(&self.resolution)
            || !self.resolution.is_multiple_of(8)
            || self.size_m.iter().any(|v| !(1e-6..=1.).contains(v))
            || self.size_m[1] != self.size_m[0] / 8.
            || ![
                self.density_kg_m3,
                self.specific_heat_j_kg_k,
                self.conductivity_w_m_k,
                self.latent_heat_j_kg,
            ]
            .into_iter()
            .all(positive)
            || ![
                self.melting_temperature_k,
                self.initial_temperature_k,
                self.cold_wall_temperature_k,
            ]
            .into_iter()
            .all(|v| (100. ..=1000.).contains(&v))
            || !positive(span)
            || self.initial_temperature_k != self.melting_temperature_k
            || !domain.into_iter().all(|v| (100. ..=1000.).contains(&v))
            || domain[0] >= domain[1]
            || self.cold_wall_temperature_k < domain[0]
            || self.melting_temperature_k > domain[1]
            || self.steps < n * n / 2
            || self.steps > n * n
            || self.observation_steps.len() < 2
            || self.observation_steps.len() > 16
            || self.observation_steps.first() != Some(&0)
            || self.observation_steps.last() != Some(&self.steps)
            || self.observation_steps.windows(2).any(|v| v[0] >= v[1])
            || self
                .observation_steps
                .iter()
                .any(|v| *v != 0 && (*v < n * n / 2 || *v > self.steps))
            || [self.front_tolerance, self.temperature_tolerance]
                .into_iter()
                .any(|v| !positive(v) || v > 0.02)
            || [self.mass_tolerance, self.energy_tolerance]
                .into_iter()
                .any(|v| !positive(v) || v > 1e-10)
            || [
                &self.material_provenance,
                &self.boundary_provenance,
                &self.geometry_provenance,
            ]
            .into_iter()
            .any(|v| v.trim().is_empty() || v.len() > 4096)
        {
            return Err(invalid(
                "explicit bounded synthetic equal-property conduction solidification inputs, material domain and unchanged gates required",
            ));
        }
        self.moisture_risk.inspect(self.cold_wall_temperature_k)?;
        let stefan = self.specific_heat_j_kg_k * span / self.latent_heat_j_kg;
        let spacing = self.size_m[0] / f64::from(self.resolution);
        let diffusivity =
            self.conductivity_w_m_k / (self.density_kg_m3 * self.specific_heat_j_kg_k);
        // D2Q5 cs²=1/3, fixed tau=1 => alpha_lattice=1/6. No physical
        // property is changed to hold tau constant while refining the grid.
        let step = spacing * spacing / (6. * diffusivity);
        let duration = self.steps as f64 * step;
        let enthalpy = self.specific_heat_j_kg_k * span + self.latent_heat_j_kg;
        let cell_volume = spacing * spacing * self.size_m[2];
        let cell_mass = self.density_kg_m3 * cell_volume;
        let active_volume = (n - 1) as f64 * (n / 8) as f64 * cell_volume;
        if !(0.05..=0.2).contains(&stefan)
            || ![
                spacing,
                diffusivity,
                step,
                duration,
                enthalpy,
                cell_mass,
                active_volume,
                cell_mass * enthalpy * (n - 1) as f64 * (n / 8) as f64,
            ]
            .into_iter()
            .all(positive)
        {
            return Err(invalid(
                "finite SI conversion and Stefan number 0.05..0.2 required for the reference",
            ));
        }
        Ok(FreezingScale {
            spacing_m: spacing,
            physical_step_s: step,
            duration_s: duration,
            stefan_number: stefan,
            initial_specific_enthalpy_j_kg: enthalpy,
            shape: [self.resolution + 1, self.resolution / 8],
            active_volume_m3: active_volume,
            cell_mass_kg: cell_mass,
            active_control_bounds_m: [
                [spacing / 2., self.size_m[0] - spacing / 2.],
                [0., self.size_m[1]],
                [0., self.size_m[2]],
            ],
        })
    }
}
