//! Independent direct-only optical approval inputs for original material-tagged CAD.
use crate::{
    Result,
    cad_spectral::{CadSpectralSceneRequest, PreparedCadSpectralScene},
    contracts::{digest, invalid},
    radiation::{
        PreparedSpectralReference, RadiantHistoryPoint, SpectralReferenceSpec, SpectralSource,
    },
    science::Quantity,
    storage::Store,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SANDBOX_POLICY: &str = "harbor-cad-cad-spectral-direct-cpu-v1";
pub const STAGE: &str = "cad-spectral";
pub const RECEIPT_PATH: &str = "stages/cad-spectral/cad-spectral-receipt.json";

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CadSpectralTransportRequest {
    pub schema_version: u32,
    pub scene: CadSpectralSceneRequest,
    pub formulation: String,
    pub source: SpectralSource,
    pub source_provenance: String,
    pub history: Vec<RadiantHistoryPoint>,
    pub history_interpolation: String,
    pub history_provenance: String,
    pub samples_per_triangle: u32,
    pub seeds: [u32; 3],
    pub relative_tolerance: f64,
    pub maximum_geometry_rounding_error_m: f64,
}

impl CadSpectralTransportRequest {
    /// Share the original source-unit and history quadrature contract. The
    /// auxiliary reference sensor is solely validation; it supplies no CAD area,
    /// normal, material, visibility, optical result or physical qualification.
    pub fn illumination(&self) -> Result<PreparedSpectralReference> {
        self.scene.validate()?;
        if self.schema_version != 1
            || self.formulation != "opaque_lambertian_direct_only"
            || !matches!(self.source, SpectralSource::Directional { .. })
            || !self.samples_per_triangle.is_power_of_two()
            || !(64..=4096).contains(&self.samples_per_triangle)
            || !self.maximum_geometry_rounding_error_m.is_finite()
            || self.maximum_geometry_rounding_error_m <= 0.
            || self.maximum_geometry_rounding_error_m > 1e-8
            || self.maximum_geometry_rounding_error_m
                > self.scene.geometry_tolerance.si("length")?
        {
            return Err(invalid(
                "explicit direct-only collimated optical source, bounded per-facet samples and unchanged native rounding budget required",
            ));
        }
        SpectralReferenceSpec {
            schema_version: 1,
            synthetic: true,
            backend: "cpu".into(),
            variant: "scalar_spectral".into(),
            precision: "Float32".into(),
            wavelengths: self.scene.wavelengths.clone(),
            source: self.source.clone(),
            source_provenance: self.source_provenance.clone(),
            sensor_width: Quantity {
                value: 1.,
                unit: "mm".into(),
            },
            sensor_height: Quantity {
                value: 1.,
                unit: "mm".into(),
            },
            sensor_normal: [0., 0., 1.],
            occlusion: "none".into(),
            absorptivity: vec![1.; self.scene.wavelengths.len()],
            optical_provenance:
                "unit numerical normalization; no inferred original region material".into(),
            ageing_action: vec![1.; self.scene.wavelengths.len()],
            ageing_provenance: "unit numerical normalization; no inferred calibrated ageing curve"
                .into(),
            history: self.history.clone(),
            history_interpolation: self.history_interpolation.clone(),
            history_provenance: self.history_provenance.clone(),
            samples: 1024,
            seeds: self.seeds,
            relative_tolerance: self.relative_tolerance,
        }
        .prepare()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CadSpectralTransportSpec {
    pub schema_version: u32,
    pub request: CadSpectralTransportRequest,
    pub scene: PreparedCadSpectralScene,
}

impl CadSpectralTransportSpec {
    pub fn validate_plan(&self, plan: &crate::contracts::ExecutionPlan) -> Result<()> {
        use crate::contracts::{GpuRequirement, StageOperation};
        self.validate()?;
        let triangles: usize = self
            .scene
            .regions
            .iter()
            .map(|r| r.geometry.triangles)
            .sum();
        if plan.policy == "ci"
            || triangles > 24
            || serde_json::to_vec(self)?.len() > 64 * 1024
            || serde_json::to_vec(plan)?.len() as u64 > crate::contracts::MAX_MESSAGE - 4096
            || plan.stages.len() != 2
            || plan.stages[0].id != STAGE
            || plan.stages[0].operation != StageOperation::SpectralReference
            || !plan.stages[0].dependencies.is_empty()
            || plan.stages[1].id != "bundle"
            || plan.stages[1].operation != StageOperation::Bundle
            || plan.stages[1].dependencies != [STAGE]
            || plan.stages.iter().any(|s| {
                s.gpu != GpuRequirement::CpuOnly || s.selection.is_some() || s.vram_bytes != 0
            })
            || !plan.transfers.is_empty()
            || !plan.observation.metrics.is_empty()
            || !plan.observation.probes.is_empty()
            || !plan.observation.retained_times_s.is_empty()
            || !plan.observation.checkpoint_times_s.is_empty()
            || !plan.observation.preview_times_s.is_empty()
            || plan.observation.preview_may_drop
        {
            return Err(invalid(
                "strict independently approved direct CAD optical/bundle CPU DAG, at most 24 original facets and bounded receipt required; seeds are not times",
            ));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.request.illumination()?;
        let missing = self.request.scene.validate()?;
        let mut scene = self.scene.clone();
        scene.scene_id.clear();
        if self.schema_version != 1
            || self.scene.schema_version != 1
            || digest(&scene)? != self.scene.scene_id
            || digest(&self.scene.request)? != digest(&self.request.scene)?
            || self.scene.executed
            || self.scene.physical_validation != "unqualified"
            || self.scene.transport_readiness != "prepared_not_executed"
            || missing.iter().any(|s| s.ends_with(".response"))
            || missing != self.scene.missing_inputs
            || self.scene.ageing_readiness
                != if missing.is_empty() {
                    "prepared_not_executed"
                } else {
                    "missing_inputs"
                }
            || self.scene.regions.len() != self.request.scene.assignments.len()
            || self.scene.regions.is_empty()
        {
            return Err(invalid(
                "unchanged complete original material scene, explicit known optics and independent preparation identity required; missing ageing remains unknown",
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        let mut facets = 0u32;
        let synthetic = self.scene.regions[0].source.geometry.synthetic;
        for region in &self.scene.regions {
            region.source.validate()?;
            let geometry = &region.source.geometry;
            let original = &region.original_triangles;
            if !names.insert(&region.assignment.region_name)
                || !self
                    .request
                    .scene
                    .assignments
                    .iter()
                    .any(|a| digest(a).ok() == digest(&region.assignment).ok())
                || region.assignment.region_name != geometry.region_name
                || region.source.job_id != self.request.scene.source_job
                || original.schema_version != 1
                || original.format != "stl"
                || original.path != format!("{}.stl", geometry.region_name)
                || original.sha256.len() != 64
                || !original
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || !(12..=192).contains(&region.geometry.triangles)
                || original.bytes != 84 + 50 * region.geometry.triangles as u64
                || geometry.synthetic != synthetic
                || self.request.scene.geometry_tolerance.si("length")?
                    > geometry.geometry_tolerance_m
                || self.request.maximum_geometry_rounding_error_m > geometry.geometry_tolerance_m
            {
                return Err(invalid(
                    "complete unique original closed STL identity and named source context required",
                ));
            }
            facets += region.geometry.triangles as u32;
        }
        if facets * self.request.samples_per_triangle > 65536 {
            return Err(invalid(
                "aggregate complete original-facet sample budget exhausted",
            ));
        }
        for (i, a) in self.scene.regions.iter().enumerate() {
            for b in &self.scene.regions[i + 1..] {
                let a = a.source.geometry.bounds_m;
                let b = b.source.geometry.bounds_m;
                if !(0..3)
                    .any(|axis| a[2 * axis + 1] < b[2 * axis] || b[2 * axis + 1] < a[2 * axis])
                {
                    return Err(invalid(
                        "separated disjoint opaque original boxes required; no touching or overlapping region inference",
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn native_request(&self) -> Result<serde_json::Value> {
        self.validate()?;
        let request = &self.request;
        Ok(
            serde_json::json!({"schema_version":1,"synthetic":self.scene.regions[0].source.geometry.synthetic,
            "backend":"cpu","variant":"scalar_spectral","precision":"Float32","formulation":request.formulation,
            "scene":self.scene,"source":request.source,"source_provenance":request.source_provenance,
            "history":request.history,"history_interpolation":request.history_interpolation,"history_provenance":request.history_provenance,
            "samples_per_triangle":request.samples_per_triangle,"seeds":request.seeds,"relative_tolerance":request.relative_tolerance,
            "maximum_geometry_rounding_error_m":request.maximum_geometry_rounding_error_m}),
        )
    }
}

impl crate::contracts::ExecutionPlan {
    pub fn cad_spectral_transport(spec: CadSpectralTransportSpec, policy: String) -> Result<Self> {
        use crate::contracts::{GpuRequirement, ObservationPlan, Stage, StageOperation};
        spec.validate()?;
        let mut plan = Self {
            schema_version: 17,
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
            cad_transport: Some(spec),
            retained_cooling: None,
            stages: vec![
                Stage {
                    id: STAGE.into(),
                    dependencies: vec![],
                    operation: StageOperation::SpectralReference,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 1,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec![STAGE.into()],
                    operation: StageOperation::Bundle,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 1,
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
            fleetix_revision: crate::contracts::FLEETIX_REV.into(),
            fleetix_contract_digest: crate::contracts::fleetix_digest(),
            policy,
        };
        crate::estimates::minimum(&plan)?.apply(&mut plan);
        plan.validate()?;
        Ok(plan)
    }
}

pub fn resolve(
    store: &Store,
    request: CadSpectralTransportRequest,
) -> Result<CadSpectralTransportSpec> {
    request.illumination()?;
    let scene = crate::cad_spectral::prepare(store, request.scene.clone())?;
    let spec = CadSpectralTransportSpec {
        schema_version: 1,
        request,
        scene,
    };
    spec.validate()?;
    let root = store.job_dir(&spec.request.scene.source_job)?;
    for region in &spec.scene.regions {
        let record = &region.original_triangles;
        let bytes = crate::worker::read_bounded(
            &crate::storage::safe_path(&root, &record.path)?,
            record.bytes,
        )?;
        if bytes.len() as u64 != record.bytes
            || format!("{:x}", Sha256::digest(&bytes)) != record.sha256
        {
            return Err(invalid(
                "unchanged complete original optical geometry bytes required before approval",
            ));
        }
        let original = crate::cad_triangles::verify_binary_box(&bytes, &region.source.geometry)?;
        for facet in &original.triangles {
            screen_native_geometry(facet, spec.request.maximum_geometry_rounding_error_m)?;
        }
    }
    Ok(spec)
}

/// Predict the immutable metre-to-native conversion before approving a solve.
/// The native adapter still measures its own area and independently screens it.
fn screen_native_geometry(facet: &crate::cad_triangles::Triangle, budget: f64) -> Result<()> {
    let native = facet.vertices_m.map(|p| p.map(|v| f64::from(v as f32)));
    let rounding = facet
        .vertices_m
        .into_iter()
        .flatten()
        .zip(native.into_iter().flatten())
        .map(|(a, b)| (a - b).abs())
        .fold(0_f64, f64::max);
    let a: [f64; 3] = std::array::from_fn(|i| native[1][i] - native[0][i]);
    let b: [f64; 3] = std::array::from_fn(|i| native[2][i] - native[0][i]);
    let cross: [f64; 3] =
        std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3]);
    let area = cross.into_iter().map(|v| v * v).sum::<f64>().sqrt() / 2.;
    if !budget.is_finite()
        || budget <= 0.
        || budget > 1e-8
        || !rounding.is_finite()
        || rounding > budget
        || !area.is_finite()
        || area <= 0.
        || !facet.area_m2.is_finite()
        || facet.area_m2 <= 0.
        || (area / facet.area_m2 - 1.).abs() > 1e-6
    {
        return Err(invalid(
            "original metre-to-native Float32 vertex budget or fixed 1e-6 facet area screen exceeded; preserve original geometry without rebasing or repair",
        ));
    }
    Ok(())
}

pub fn source(store: &Store, spec: &CadSpectralTransportSpec) -> Result<std::path::PathBuf> {
    spec.validate()?;
    if digest(&resolve(store, spec.request.clone())?)? != digest(spec)? {
        return Err(invalid(
            "approved original CAD optical scene, approvals or bytes changed",
        ));
    }
    store.job_dir(&spec.request.scene.source_job)
}

pub(crate) fn originals(
    spec: &CadSpectralTransportSpec,
) -> Result<std::collections::BTreeMap<String, (String, u64, String)>> {
    let mut records = std::collections::BTreeMap::new();
    for region in &spec.scene.regions {
        let entries = [
            (
                &region.original_triangles.path,
                &region.original_triangles.sha256,
                region.original_triangles.bytes,
                "stl",
            ),
            (
                &region.source.brep.path,
                &region.source.brep.sha256,
                region.source.brep.bytes,
                "brep",
            ),
            (
                &region.source.region_evidence.path,
                &region.source.region_evidence.sha256,
                region.source.region_evidence.bytes,
                "json",
            ),
            (
                &region.source.manifest.path,
                &region.source.manifest.sha256,
                region.source.manifest.bytes,
                "json",
            ),
        ];
        for (path, hash, bytes, format) in entries {
            let record = (hash.clone(), bytes, format.into());
            if let Some(previous) = records.insert(path.clone(), record.clone())
                && previous != record
            {
                return Err(invalid("unambiguous complete CAD source identity required"));
            }
        }
    }
    Ok(records)
}

pub(crate) fn retain(
    store: &Store,
    id: &str,
    plan: &crate::contracts::ExecutionPlan,
) -> Result<()> {
    let Some(spec) = &plan.cad_transport else {
        return Ok(());
    };
    let original = source(store, spec)?;
    let root = store.job_dir(id)?;
    let intent = crate::storage::commit_artifact(
        &root,
        "cad-optical-retention.json",
        &serde_json::to_vec(spec)?,
        "json",
        "durable original-CAD optical input intent; preserve unverifiable interrupted inputs",
    )?;
    for (path, (hash, bytes, format)) in originals(spec)? {
        let data =
            crate::worker::read_bounded(&crate::storage::safe_path(&original, &path)?, bytes)?;
        let copied = crate::storage::commit_artifact(
            &root,
            &format!("source-cad/{path}"),
            &data,
            &format,
            "unchanged distinct-inode original CAD/triangle/geometry evidence retained before acknowledgment",
        )?;
        if copied.sha256 != hash || copied.bytes != bytes {
            return Err(invalid("CAD optical input changed during source retention"));
        }
        store.add_artifact(id, &copied)?;
    }
    source(store, spec)?;
    let source_id = &spec.request.scene.source_job;
    let provenance = serde_json::json!({"source":spec,"plan":store.recorded_plan(source_id)?,"host_profile":store.job_profile(source_id)?,"execution_binding":store.execution_binding(source_id)?,"execution_authorization":store.execution_authorization(source_id)?});
    store.add_artifact(id,&crate::storage::commit_artifact(&root,"cad-optical-source-execution.json",&serde_json::to_vec_pretty(&provenance)?,"json","complete original CAD approvals and runtime authorization; no upgraded optical or physical qualification")?)?;
    store.add_artifact(id, &intent)?;
    std::fs::File::open(&root)?.sync_all()?;
    registered(store, id, plan)?;
    Ok(())
}

pub fn registered(
    store: &Store,
    id: &str,
    plan: &crate::contracts::ExecutionPlan,
) -> Result<std::path::PathBuf> {
    let spec = plan
        .cad_transport
        .as_ref()
        .ok_or_else(|| invalid("approved source-bound CAD optics required"))?;
    spec.validate()?;
    let root = store.job_dir(id)?;
    let (_, intent) =
        crate::qualification::registered_json(store, id, "cad-optical-retention.json")?
            .ok_or_else(|| {
                invalid("complete registered optical input retention intent required")
            })?;
    if intent != serde_json::to_value(spec)? {
        return Err(invalid("CAD optical source staging intent changed"));
    }
    let (_, provenance) =
        crate::qualification::registered_json(store, id, "cad-optical-source-execution.json")?
            .ok_or_else(|| invalid("retained original CAD source execution provenance required"))?;
    let original: crate::contracts::ExecutionPlan =
        serde_json::from_value(provenance["plan"].clone())?;
    let binding: crate::execution::ExecutionBinding =
        serde_json::from_value(provenance["execution_binding"].clone())?;
    let profile: crate::contracts::HostExecutionProfile =
        serde_json::from_value(provenance["host_profile"].clone())?;
    let authorization: crate::authority::ExecutionAuthorization =
        serde_json::from_value(provenance["execution_authorization"].clone())?;
    authorization.verify(&original, &profile, &binding)?;
    if provenance["source"] != serde_json::to_value(spec)?
        || spec.scene.regions.iter().any(|r| {
            original.id().ok().as_ref() != Some(&r.source.execution_id)
                || original.science_id().ok().as_ref() != Some(&r.source.science_id)
                || digest(&binding).ok().as_ref() != Some(&r.source.execution_binding_digest)
                || digest(&authorization).ok().as_ref() != Some(&r.source.authorization_digest)
        })
    {
        return Err(invalid(
            "original CAD science/execution/binding/authorization changed",
        ));
    }
    for (path, (hash, bytes, format)) in originals(spec)? {
        let path = format!("source-cad/{path}");
        let registered = store
            .artifact_record(id, &path)?
            .ok_or_else(|| invalid("complete registered original optical input required"))?;
        let observed = crate::storage::native_manifest(
            &root,
            &path,
            bytes,
            "recheck immutable original optical input",
        )?;
        if registered.format != format
            || registered.sha256 != hash
            || registered.bytes != bytes
            || observed.sha256 != hash
            || observed.bytes != bytes
        {
            return Err(invalid(
                "retained optical original bytes or metadata changed",
            ));
        }
    }
    crate::storage::safe_path(&root, "source-cad")
}

pub(crate) fn recover_orphan(store: &Store, id: &str) -> Result<()> {
    let root = crate::storage::safe_path(&store.root, &format!("artifacts/{id}"))?;
    let path = root.join("cad-optical-retention.json");
    if path.exists() {
        let spec: CadSpectralTransportSpec = serde_json::from_slice(&crate::worker::read_bounded(
            &path,
            crate::contracts::MAX_MESSAGE,
        )?)?;
        if source(store, &spec).is_ok() {
            std::fs::remove_dir_all(&root)?;
            std::fs::File::open(
                root.parent()
                    .ok_or_else(|| invalid("optical orphan parent required"))?,
            )?
            .sync_all()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod geometry_screen_tests {
    use super::screen_native_geometry;
    use crate::cad_triangles::Triangle;

    #[test]
    fn far_translated_original_refuses_rounding_and_area_before_approval() {
        let mut facet = Triangle {
            vertices_m: [[0., 0., 0.], [0.03, 0., 0.], [0., 0., 0.01]],
            normal: [0., -1., 0.],
            area_m2: 0.00015,
            boundary: 2,
        };
        screen_native_geometry(&facet, 1e-8).unwrap();
        facet.vertices_m = facet
            .vertices_m
            .map(|p| [p[0] + 0.1, p[1] - 0.02, p[2] + 0.3]);
        assert!(screen_native_geometry(&facet, 1e-8).is_err());
        assert!(screen_native_geometry(&facet, 2e-8).is_err());
        facet.vertices_m = [[0.1, 0., 0.], [0.13, 0., 0.], [0.1, 0., 0.01]];
        screen_native_geometry(&facet, 1e-8).unwrap();
        assert!(screen_native_geometry(&facet, 1e-10).is_err());
    }

    #[test]
    fn finite_vertex_budget_never_authorizes_collapsed_or_nonfinite_native_area() {
        let mut facet = Triangle {
            vertices_m: [[0.1, 0., 0.], [0.100000001, 0., 0.], [0.1, 0., 0.01]],
            normal: [0., -1., 0.],
            area_m2: 5e-12,
            boundary: 2,
        };
        assert!(screen_native_geometry(&facet, 1e-8).is_err());
        facet.vertices_m[0][0] = f64::NAN;
        assert!(screen_native_geometry(&facet, 1e-8).is_err());
    }
}
