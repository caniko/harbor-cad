//! Independently approved native cooling of one exact registered wetting state.
use crate::{
    Result,
    contracts::{digest, invalid},
    qualification::EvidenceRecord,
    retained_cooling::{PreparedRetainedCooling, RetainedCoolingRequest},
    storage::Store,
    wetting::WettingReferenceSpec,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const STAGE: &str = "retained-cooling";
pub const SANDBOX_POLICY: &str = "harbor-cad-retained-cooling-cpu-v1";
pub const RECEIPT_PATH: &str = "stages/retained-cooling/retained-cooling-receipt.json";

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoolingExecutionRequest {
    pub schema_version: u32,
    pub initialization: RetainedCoolingRequest,
    /// Congruent original-parent subdivision, preserving every water fraction.
    pub spatial_refinement: u32,
    pub integration_substeps: u32,
    /// Original-spacing thermal steps, multiplied by q²*substeps natively.
    pub base_steps: u64,
    pub observation_base_steps: Vec<u64>,
}

impl CoolingExecutionRequest {
    pub fn validate(&self) -> Result<()> {
        self.initialization.validate()?;
        if self.schema_version != 1
            || !(1..=4).contains(&self.spatial_refinement)
            || ![1, 2, 4].contains(&self.integration_substeps)
            || !(1..=16384).contains(&self.base_steps)
            || !(2..=16).contains(&self.observation_base_steps.len())
            || self.observation_base_steps.first() != Some(&0)
            || self.observation_base_steps.last() != Some(&self.base_steps)
            || self.observation_base_steps.windows(2).any(|p| p[0] >= p[1])
            || self.native_steps() > 1_000_000
        {
            return Err(invalid(
                "strict independently approved retained cooling levels, bounded integration and exact complete physical observations required",
            ));
        }
        Ok(())
    }
    fn factor(&self) -> u64 {
        u64::from(self.spatial_refinement)
            .pow(2)
            .saturating_mul(u64::from(self.integration_substeps))
    }
    pub fn native_steps(&self) -> u64 {
        self.base_steps.saturating_mul(self.factor())
    }
    pub fn observation_steps(&self) -> Vec<u64> {
        self.observation_base_steps
            .iter()
            .map(|v| v.saturating_mul(self.factor()))
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoolingExecutionSpec {
    pub schema_version: u32,
    pub request: CoolingExecutionRequest,
    pub prepared: PreparedRetainedCooling,
    pub source_wetting: WettingReferenceSpec,
    pub source_authorization_digest: String,
    /// Complete authoritative wetting history, native request and receipt.
    pub originals: Vec<EvidenceRecord>,
}

fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl CoolingExecutionSpec {
    pub fn validate_plan(&self, plan: &crate::contracts::ExecutionPlan) -> Result<()> {
        use crate::contracts::{GpuRequirement, StageOperation};
        self.validate()?;
        if plan.policy == "ci"
            || plan.stages.len() != 2
            || plan.stages[0].id != STAGE
            || plan.stages[0].operation != StageOperation::FreezingReference
            || !plan.stages[0].dependencies.is_empty()
            || plan.stages[1].id != "bundle"
            || plan.stages[1].operation != StageOperation::Bundle
            || plan.stages[1].dependencies != [STAGE]
            || plan.stages.iter().any(|s| {
                s.gpu != GpuRequirement::CpuOnly || s.selection.is_some() || s.vram_bytes != 0
            })
            || !plan.transfers.is_empty()
            || plan.observation.retained_times_s != self.times_s()
            || !plan.observation.metrics.is_empty()
            || !plan.observation.probes.is_empty()
            || !plan.observation.checkpoint_times_s.is_empty()
            || !plan.observation.preview_times_s.is_empty()
            || plan.observation.preview_may_drop
            || plan.observation.scientific_congestion != "fail"
        {
            return Err(invalid(
                "strict independent version-18 original-source cooling/bundle CPU DAG and native physical observations required",
            ));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.request.validate()?;
        self.source_wetting.validate()?;
        let retained = &self.prepared.retained;
        let mut prepared = self.prepared.clone();
        let id = std::mem::take(&mut prepared.initialization_id);
        let nx = retained.extrusion.source_grid_shape[0];
        let ny = retained.extrusion.source_grid_shape[1];
        let expected_paths = std::iter::once("native-wetting-request.json".to_string())
            .chain([
                "stages/wetting/wetting-receipt.json".to_string(),
                "stages/wetting/verified-wetting-receipt.json".to_string(),
            ])
            .chain(
                self.source_wetting
                    .observation_steps
                    .iter()
                    .map(|s| format!("stages/wetting/wetting-{s}.csv")),
            )
            .collect::<std::collections::BTreeSet<_>>();
        if self.schema_version != 1
            || digest(&prepared)? != id
            || digest(&self.prepared.request)? != digest(&self.request.initialization)?
            || digest(&retained.request)? != digest(&self.request.initialization.retained)?
            || digest(&self.prepared.normalized)?
                != digest(&self.request.initialization.thermal.normalize()?)?
            || retained.source_science_id != digest(&self.source_wetting)?
            || self.prepared.executed
            || self.prepared.physical_validation != "unqualified"
            || !retained.preserved_original_distribution
            || retained.executed
            || !retained.extrusion.phase_fraction_bounds_satisfied
            || retained.extrusion.maximum_speed_m_s != 0.
            || self.prepared.normalized.density_kg_m3 != self.source_wetting.density_liquid_kg_m3
            || nx < 3
            || ny < 4
            || nx > 241
            || ny > 145
            || self.prepared.initialization.original_active_controls != nx * (ny - 2)
            || self.prepared.initialization.original_velocity_state != "exactly_zero_preserved"
            || self.prepared.initialization.original_fraction_state
                != "complete_original_bounded_unclipped"
            || !self
                .prepared
                .initialization
                .relative_initial_energy_error
                .is_finite()
            || self.prepared.initialization.relative_initial_energy_error
                > self
                    .request
                    .initialization
                    .thermal
                    .maximum_relative_conservation_error
            || [
                &retained.source_science_id,
                &retained.source_execution_id,
                &retained.source_execution_binding_digest,
                &self.source_authorization_digest,
            ]
            .iter()
            .any(|v| !hash(v))
            || !(5..=19).contains(&self.originals.len())
            || self.originals.windows(2).any(|p| p[0].path >= p[1].path)
            || self
                .originals
                .iter()
                .any(|r| !hash(&r.sha256) || r.bytes == 0 || r.bytes > 16 * 1024 * 1024)
            || self
                .originals
                .iter()
                .map(|r| r.path.clone())
                .collect::<std::collections::BTreeSet<_>>()
                != expected_paths
            || !self.originals.iter().any(|r| {
                r.path == retained.original_field.path
                    && r.sha256 == retained.original_field.sha256
                    && r.bytes == retained.original_field.bytes
            })
            || !self.originals.iter().any(|r| {
                r.path == retained.original_receipt.path
                    && r.sha256 == retained.original_receipt.sha256
                    && r.bytes == retained.original_receipt.bytes
            })
            || !self
                .originals
                .iter()
                .any(|r| r.path == "native-wetting-request.json")
            || self.source_wetting.observation_steps.iter().any(|s| {
                !self
                    .originals
                    .iter()
                    .any(|r| r.path == format!("stages/wetting/wetting-{s}.csv"))
            })
        {
            return Err(invalid(
                "complete unchanged source-bound cooling initialization, wetting approvals and original history required",
            ));
        }
        let dt = self.prepared.initialization.thermal_physical_step_s;
        if !dt.is_finite() || dt <= 0. || !self.duration_s().is_finite() {
            return Err(invalid(
                "finite positive independently prescribed cooling time scale required",
            ));
        }
        Ok(())
    }
    pub fn duration_s(&self) -> f64 {
        self.request.base_steps as f64 * self.prepared.initialization.thermal_physical_step_s
    }
    pub fn times_s(&self) -> Vec<f64> {
        self.request
            .observation_base_steps
            .iter()
            .map(|s| *s as f64 * self.prepared.initialization.thermal_physical_step_s)
            .collect()
    }
    pub fn native_request(&self) -> Result<serde_json::Value> {
        self.validate()?;
        let retained = &self.prepared.retained;
        Ok(
            serde_json::json!({"schema_version":1,"native_request":{"schema_version":1,"synthetic":true,"formulation":"stationary_equal_property_retained_phase_conduction","source_shape":retained.extrusion.source_grid_shape,"spacing_m":retained.extrusion.spacing_m,"extrusion_m":retained.extrusion.extrusion_m,"destination_origin_m":self.request.initialization.retained.destination_origin_m,"thermal":self.prepared.normalized,"steps":self.request.native_steps(),"observation_steps":self.request.observation_steps(),"integration_substeps":self.request.integration_substeps,"spatial_refinement":self.request.spatial_refinement},"original_sha256":retained.original_field.sha256,"original_bytes":retained.original_field.bytes,"maximum_relative_conservation_error":self.request.initialization.thermal.maximum_relative_conservation_error}),
        )
    }
}

impl crate::contracts::ExecutionPlan {
    pub fn retained_cooling(spec: CoolingExecutionSpec, policy: String) -> Result<Self> {
        use crate::contracts::*;
        spec.validate()?;
        let mut plan = Self {
            schema_version: 18,
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
            atmosphere: None,
            atmospheric_transport: None,
            cad_variant: None,
            cad_sheet_variant: None,
            cad_transport: None,
            observation: ObservationPlan {
                retained_times_s: spec.times_s(),
                metrics: vec![],
                probes: vec![],
                checkpoint_times_s: vec![],
                preview_times_s: vec![],
                max_artifact_bytes: 0,
                scientific_congestion: "fail".into(),
                preview_may_drop: false,
            },
            retained_cooling: Some(spec),
            stages: vec![
                Stage {
                    id: STAGE.into(),
                    operation: StageOperation::FreezingReference,
                    dependencies: vec![],
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 0,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    operation: StageOperation::Bundle,
                    dependencies: vec![STAGE.into()],
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

pub fn resolve(store: &Store, request: CoolingExecutionRequest) -> Result<CoolingExecutionSpec> {
    request.validate()?;
    let prepared = crate::retained_cooling::prepare(store, request.initialization.clone())?;
    let id = &request.initialization.retained.source_job;
    let original = store.recorded_plan(id)?;
    let source_wetting = original
        .wetting
        .clone()
        .ok_or_else(|| invalid("registered original native wetting plan required"))?;
    let mut originals = std::collections::BTreeMap::new();
    for path in [
        "native-wetting-request.json".to_string(),
        "stages/wetting/wetting-receipt.json".to_string(),
        prepared.retained.original_receipt.path.clone(),
    ]
    .into_iter()
    .chain(
        source_wetting
            .observation_steps
            .iter()
            .map(|s| format!("stages/wetting/wetting-{s}.csv")),
    ) {
        let (record, _) = crate::results::registered_bytes(
            store,
            id,
            &path,
            if path.ends_with(".csv") {
                "csv"
            } else {
                "json"
            },
        )?;
        originals.insert(path, record);
    }
    let source_authorization_digest = digest(&store.execution_authorization(id)?)?;
    let spec = CoolingExecutionSpec {
        schema_version: 1,
        request,
        prepared,
        source_wetting,
        source_authorization_digest,
        originals: originals.into_values().collect(),
    };
    spec.validate()?;
    Ok(spec)
}

pub fn source(store: &Store, spec: &CoolingExecutionSpec) -> Result<std::path::PathBuf> {
    spec.validate()?;
    if digest(&resolve(store, spec.request.clone())?)? != digest(spec)? {
        return Err(invalid(
            "approved complete original wetting source, cooling science or execution authorization changed",
        ));
    }
    store.job_dir(&spec.request.initialization.retained.source_job)
}

pub(crate) fn retain(
    store: &Store,
    id: &str,
    plan: &crate::contracts::ExecutionPlan,
) -> Result<()> {
    let Some(spec) = &plan.retained_cooling else {
        return Ok(());
    };
    let original = source(store, spec)?;
    let root = store.job_dir(id)?;
    let intent = crate::storage::commit_artifact(
        &root,
        "cooling-source-retention.json",
        &serde_json::to_vec(spec)?,
        "json",
        "durable exact original wetting history and independent cooling staging intent; preserve unverifiable orphans",
    )?;
    for record in &spec.originals {
        let raw = crate::worker::read_bounded(
            &crate::storage::safe_path(&original, &record.path)?,
            record.bytes,
        )?;
        let retained = crate::storage::commit_artifact(
            &root,
            &format!("source-wetting/{}", record.path),
            &raw,
            if record.path.ends_with(".csv") {
                "csv"
            } else {
                "json"
            },
            "unchanged distinct-inode registered native wetting original retained before acknowledgment; no qualification upgrade",
        )?;
        if retained.sha256 != record.sha256 || retained.bytes != record.bytes {
            return Err(invalid(
                "original wetting source changed during cooling retention",
            ));
        }
        store.add_artifact(id, &retained)?;
    }
    source(store, spec)?;
    let source_id = &spec.request.initialization.retained.source_job;
    let provenance = serde_json::json!({"source":spec,"plan":store.recorded_plan(source_id)?,"host_profile":store.job_profile(source_id)?,"execution_binding":store.execution_binding(source_id)?,"execution_authorization":store.execution_authorization(source_id)?});
    store.add_artifact(
        id,
        &crate::storage::commit_artifact(
            &root,
            "cooling-source-execution.json",
            &serde_json::to_vec_pretty(&provenance)?,
            "json",
            "complete independently authorized original wetting source runtime and approvals",
        )?,
    )?;
    store.add_artifact(id, &intent)?;
    std::fs::File::open(&root)?.sync_all()?;
    registered(store, id, plan)?;
    Ok(())
}

/// Verify retained originals without requiring the source worker to remain online.
fn verify_provenance(
    spec: &CoolingExecutionSpec,
    provenance: &serde_json::Value,
) -> Result<crate::contracts::ExecutionPlan> {
    let source_plan: crate::contracts::ExecutionPlan =
        serde_json::from_value(provenance["plan"].clone())?;
    let binding: crate::execution::ExecutionBinding =
        serde_json::from_value(provenance["execution_binding"].clone())?;
    let authorization: Option<crate::authority::ExecutionAuthorization> =
        serde_json::from_value(provenance["execution_authorization"].clone())?;
    let profile: crate::contracts::HostExecutionProfile =
        serde_json::from_value(provenance["host_profile"].clone())?;
    if provenance["source"] != serde_json::to_value(spec)?
        || source_plan.id()? != spec.prepared.retained.source_execution_id
        || source_plan.science_id()? != spec.prepared.retained.source_science_id
        || binding.plan_digest != source_plan.id()?
        || binding.host_profile_digest != digest(&profile)?
        || digest(&binding)? != spec.prepared.retained.source_execution_binding_digest
        || digest(&authorization)? != spec.source_authorization_digest
    {
        return Err(invalid(
            "retained original cooling source plan, execution identity or independent authorization changed",
        ));
    }
    Ok(source_plan)
}

pub(crate) fn registered(
    store: &Store,
    id: &str,
    plan: &crate::contracts::ExecutionPlan,
) -> Result<std::path::PathBuf> {
    let spec = plan
        .retained_cooling
        .as_ref()
        .ok_or_else(|| invalid("source-bound cooling plan required"))?;
    spec.validate()?;
    let (_, intent) = crate::results::registered(store, id, "cooling-source-retention.json")?;
    if intent != serde_json::to_value(spec)? {
        return Err(invalid("immutable cooling source staging intent changed"));
    }
    let root = store.job_dir(id)?;
    let original_root = root.join("source-wetting");
    for record in &spec.originals {
        let (copy, _) = crate::results::registered_bytes(
            store,
            id,
            &format!("source-wetting/{}", record.path),
            if record.path.ends_with(".csv") {
                "csv"
            } else {
                "json"
            },
        )?;
        if copy.sha256 != record.sha256 || copy.bytes != record.bytes {
            return Err(invalid(
                "retained cooling source differs from original authorized bytes",
            ));
        }
    }
    let (_, provenance) = crate::results::registered(store, id, "cooling-source-execution.json")?;
    let source_plan = verify_provenance(spec, &provenance)?;
    let raw = crate::worker::read_bounded(
        &crate::storage::safe_path(&original_root, &spec.prepared.retained.original_field.path)?,
        16 * 1024 * 1024,
    )?;
    if digest(&crate::retained_cooling::reconstruct(
        &spec.source_wetting,
        &spec.request.initialization,
        &raw,
    )?)? != digest(&spec.prepared.initialization)?
        || digest(&crate::wetting_retention::reconstruct(
            &spec.source_wetting,
            &spec.request.initialization.retained,
            &raw,
        )?)? != digest(&spec.prepared.retained.extrusion)?
    {
        return Err(invalid(
            "retained wetting original controls no longer reconstruct the approved cooling initialization",
        ));
    }
    let receipt: serde_json::Value = serde_json::from_slice(&crate::worker::read_bounded(
        &original_root.join("stages/wetting/verified-wetting-receipt.json"),
        256 * 1024,
    )?)?;
    crate::wetting::verify_receipt(
        &source_plan,
        &original_root.join("stages/wetting"),
        &receipt,
    )?;
    crate::storage::safe_path(&original_root, &spec.prepared.retained.original_field.path)
}

pub(crate) fn recover_orphan(store: &Store, id: &str) -> Result<()> {
    let root = store.root.join("artifacts").join(id);
    let path = root.join("cooling-source-retention.json");
    if !path.exists() {
        return Ok(());
    }
    let spec: CoolingExecutionSpec = serde_json::from_slice(&crate::worker::read_bounded(
        &path,
        crate::contracts::MAX_MESSAGE,
    )?)?;
    source(store, &spec)?;
    let mut removal = vec![path, root.join("cooling-source-execution.json")];
    for record in &spec.originals {
        let copy = root.join("source-wetting").join(&record.path);
        if copy.exists() {
            let observed = crate::storage::native_manifest(
                &root,
                &format!("source-wetting/{}", record.path),
                record.bytes,
                "verify orphan original cooling input",
            )?;
            if observed.sha256 != record.sha256 || observed.bytes != record.bytes {
                return Err(invalid(
                    "unverifiable original cooling staging orphan preserved",
                ));
            }
            removal.push(copy);
        }
    }
    if removal[1].exists() {
        let value: serde_json::Value =
            serde_json::from_slice(&crate::worker::read_bounded(&removal[1], 256 * 1024)?)?;
        verify_provenance(&spec, &value)?;
    }
    for p in removal {
        if p.exists() {
            std::fs::remove_file(p)?;
        }
    }
    Ok(())
}
