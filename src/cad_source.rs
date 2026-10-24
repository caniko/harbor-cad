//! Closed named BREP evidence for imported recipes; no native document opens.
use crate::{
    Result,
    contracts::*,
    qualification::{self, EvidenceRecord},
    storage::{
        Store, commit_artifact, copy_verified, native_manifest, private_dir, publish_directory,
        safe_path, sync_directories,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CadMeshRequest {
    pub source_job: String,
    pub region_name: String,
    pub resolution: u32,
    pub geometry_tolerance_m: f64,
}

impl ExecutionPlan {
    pub fn cad_mesh(source: CadSource, policy: String) -> Result<Self> {
        let mut plan = Self {
            schema_version: 7,
            case: None,
            fem: None,
            thermal: None,
            cad_source: Some(source),
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
                    id: "mesh".into(),
                    dependencies: vec![],
                    operation: StageOperation::CadMesh,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 1,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec!["mesh".into()],
                    operation: StageOperation::Bundle,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 16 * 1024 * 1024,
                    vram_bytes: 0,
                },
            ],
            transfers: vec![],
            observation: ObservationPlan {
                metrics: vec!["geometry_correspondence".into()],
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

pub fn plan(store: &Store, request: CadMeshRequest, policy: String) -> Result<ExecutionPlan> {
    let (_, bound) = source(
        store,
        &request.source_job,
        &request.region_name,
        request.resolution,
        request.geometry_tolerance_m,
    )?;
    ExecutionPlan::cad_mesh(bound, policy)
}

/// Exact standalone Gmsh descriptor, with source CAD and placement units retained.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CadMeshDescriptor {
    pub schema_version: u32,
    pub synthetic: bool,
    pub backend: String,
    pub formulation: String,
    pub geometry_provenance: String,
    pub brep_file: String,
    pub brep_sha256: String,
    pub brep_bytes: u64,
    pub region_name: String,
    pub bounds_m: [f64; 6],
    pub volume_m3: f64,
    pub source_unit: String,
    pub scale_to_m: f64,
    pub placement_translation_unit: String,
    pub source_transform: [f64; 16],
    pub resolution: u32,
    pub geometry_tolerance_m: f64,
    pub volume_relative_tolerance: f64,
}

pub(crate) fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl CadMeshDescriptor {
    pub fn lengths_m(&self) -> [f64; 3] {
        [
            self.bounds_m[1] - self.bounds_m[0],
            self.bounds_m[3] - self.bounds_m[2],
            self.bounds_m[5] - self.bounds_m[4],
        ]
    }
    pub fn validate(&self) -> Result<()> {
        let sizes = self.lengths_m();
        let min = sizes.into_iter().fold(f64::INFINITY, f64::min);
        let max = sizes.into_iter().fold(0., f64::max);
        let volume = sizes.iter().product::<f64>();
        if self.schema_version != 1
            || self.backend != "cpu"
            || self.formulation != "imported_axis_aligned_box"
            || self.brep_file != "solid.brep"
            || !hash(&self.brep_sha256)
            || !(1..=64 * 1024 * 1024).contains(&self.brep_bytes)
            || !token(&self.region_name)
            || self.geometry_provenance.trim().is_empty()
            || self.geometry_provenance.len() > 4096
            || self.source_unit != "mm"
            || self.placement_translation_unit != "mm"
            || self.scale_to_m != 0.001
            || !self
                .bounds_m
                .iter()
                .chain(&self.source_transform)
                .all(|v| v.is_finite())
            || min <= 0.
            || max / min > 1000.
            || !volume.is_finite()
            || !self.volume_m3.is_finite()
            || self.volume_m3 <= 0.
            || !self.volume_relative_tolerance.is_finite()
            || self.volume_relative_tolerance <= 0.
            || self.volume_relative_tolerance > 1e-10
            || (self.volume_m3 - volume).abs()
                > self.volume_relative_tolerance * volume.abs().max(self.volume_m3.abs())
            || !(2..=32).contains(&self.resolution)
            || !self.geometry_tolerance_m.is_finite()
            || self.geometry_tolerance_m < 1e-10
            || self.geometry_tolerance_m >= 0.001 * min
            || self
                .bounds_m
                .iter()
                .any(|v| 10. * (v.abs().next_up() - v.abs()) > self.geometry_tolerance_m)
            || self.source_transform[12..] != [0., 0., 0., 1.]
        {
            return Err(invalid(
                "bounded imported box, exact BREP, rigid CAD placement and unchanged SI correspondence gates required",
            ));
        }
        let r = &self.source_transform;
        for i in 0..3 {
            for j in 0..3 {
                let dot = (0..3).map(|k| r[4 * i + k] * r[4 * j + k]).sum::<f64>();
                if (dot - f64::from(i == j)).abs() > 1e-12 {
                    return Err(invalid("rigid original FreeCAD placement required"));
                }
            }
        }
        let det = r[0] * (r[5] * r[10] - r[6] * r[9]) - r[1] * (r[4] * r[10] - r[6] * r[8])
            + r[2] * (r[4] * r[9] - r[5] * r[8]);
        if (det - 1.).abs() > 1e-12 {
            return Err(invalid("proper original FreeCAD placement required"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CadSource {
    pub schema_version: u32,
    pub job_id: String,
    pub science_id: String,
    pub execution_id: String,
    pub execution_binding_digest: String,
    pub authorization_digest: String,
    pub manifest: EvidenceRecord,
    pub region_evidence: EvidenceRecord,
    pub brep: ArtifactManifest,
    pub geometry: CadMeshDescriptor,
}

impl CadSource {
    pub fn validate(&self) -> Result<()> {
        self.geometry.validate()?;
        if self.schema_version != 1
            || uuid::Uuid::parse_str(&self.job_id).is_err()
            || [
                &self.science_id,
                &self.execution_id,
                &self.execution_binding_digest,
                &self.authorization_digest,
                &self.manifest.sha256,
                &self.region_evidence.sha256,
            ]
            .iter()
            .any(|h| !hash(h))
            || self.manifest.path != "brep-manifest.json"
            || !(1..=256 * 1024).contains(&self.manifest.bytes)
            || self.region_evidence.path != "regions.json"
            || !(1..=256 * 1024).contains(&self.region_evidence.bytes)
            || self.brep.schema_version != 1
            || self.brep.path != format!("{}.brep", self.geometry.region_name)
            || self.brep.format != "brep"
            || self.brep.sha256 != self.geometry.brep_sha256
            || self.brep.bytes != self.geometry.brep_bytes
            || self.brep.units.is_some()
            || self.brep.time_s.is_some()
            || self.brep.association.is_some()
        {
            return Err(invalid(
                "immutable authorized named CAD source, descriptors and exact BREP bytes required",
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrepManifest {
    schema_version: u32,
    synthetic: bool,
    regions: Vec<BrepRegion>,
    gap_healing: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrepRegion {
    pub region_name: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub source_unit: String,
    pub scale_to_m: f64,
    pub bounds_m: [f64; 6],
    pub volume_m3: f64,
    pub source_transform: [f64; 16],
    pub placement_translation_unit: String,
}

/// Check every native named BREP against independently verified world geometry
/// before a controlled variant is published. Native execution alone cannot
/// authorize a changed manifest or a missing/empty geometry export.
pub(crate) fn verify_brep_outputs(
    root: &std::path::Path,
    snapshot: &crate::cad::RegionSnapshot,
) -> Result<()> {
    let manifest: BrepManifest = serde_json::from_slice(&crate::worker::read_bounded(
        &safe_path(root, "brep-manifest.json")?,
        256 * 1024,
    )?)?;
    if manifest.schema_version != 1
        || manifest.synthetic != snapshot.synthetic
        || manifest.gap_healing
        || manifest.regions.len() != snapshot.regions.len()
        || manifest.regions.is_empty()
        || manifest.regions.len() > 256
    {
        return Err(invalid(
            "complete original-unit named BREP manifest required",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for record in manifest.regions {
        let region = snapshot
            .regions
            .iter()
            .find(|r| r.name == record.region_name)
            .ok_or_else(|| invalid("BREP export refers to an unknown recomputed region"))?;
        if !seen.insert(record.region_name)
            || record.path != format!("{}.brep", region.name)
            || record.bounds_m != region.bounds_m
            || record.volume_m3 != region.volume_m3
            || record.source_transform != region.transform
            || record.source_unit != "mm"
            || record.scale_to_m != 0.001
            || record.placement_translation_unit != "mm"
            || !(1..=64 * 1024 * 1024).contains(&record.bytes)
        {
            return Err(invalid(
                "recomputed BREP export metadata differs from original named geometry",
            ));
        }
        let original = native_manifest(
            root,
            &record.path,
            64 * 1024 * 1024,
            "verify recomputed native BREP export",
        )?;
        if original.sha256 != record.sha256 || original.bytes != record.bytes {
            return Err(invalid(
                "recomputed native BREP original differs from its closed manifest",
            ));
        }
    }
    Ok(())
}

/// A completed, originally authorized patched-importer CAD job and its verified
/// registered regions. Shared by every recipe that binds an original CAD job.
pub(crate) fn authorized(
    store: &Store,
    id: &str,
) -> Result<(
    ExecutionPlan,
    crate::cad::RegionReport,
    crate::execution::ExecutionBinding,
    crate::authority::ExecutionAuthorization,
)> {
    let job = store.job(id)?;
    if job.state != "succeeded" || job.exit_code != Some(0) {
        return Err(invalid("completed committed named CAD source required"));
    }
    let plan = store.recorded_plan(id)?;
    let report = crate::cad::regions(store, id)?;
    let binding = store.execution_binding(id)?;
    let authorization = store
        .execution_authorization(id)?
        .ok_or_else(|| invalid("original CAD execution authorization required"))?;
    authorization.verify(&plan, &store.job_profile(id)?, &binding)?;
    let receipt_path = if plan
        .stages
        .iter()
        .any(|s| s.operation == StageOperation::CadInspect)
    {
        "cad_inspect-receipt.json"
    } else if plan
        .stages
        .iter()
        .any(|s| s.operation == StageOperation::EnclosureFixture)
    {
        "enclosure_fixture-receipt.json"
    } else {
        "cad_fixture-receipt.json"
    };
    let (_, receipt) = qualification::registered_json(store, id, receipt_path)?
        .ok_or_else(|| invalid("registered patched CAD importer receipt required"))?;
    if receipt["security_minimum"] != "1.1.4"
        || receipt["sandbox_required"] != true
        || receipt["import_policy"] != crate::sandbox::IMPORT_POLICY
        || !binding.native_files.contains_key("cad_closure")
    {
        return Err(invalid(
            "original patched importer and closure-specific CAD policy required",
        ));
    }
    Ok((plan, report, binding, authorization))
}

/// A succeeded, originally authorized CAD job with its closed BREP manifest
/// rechecked against the registered region evidence. Shared by every recipe
/// that binds named BREPs; bounds and placement never come from callers.
pub(crate) struct VerifiedManifest {
    pub plan: ExecutionPlan,
    pub report: crate::cad::RegionReport,
    pub binding: crate::execution::ExecutionBinding,
    pub authorization: crate::authority::ExecutionAuthorization,
    pub manifest: EvidenceRecord,
    pub synthetic: bool,
    pub regions: Vec<BrepRegion>,
}

pub(crate) fn verified_manifest(store: &Store, id: &str) -> Result<VerifiedManifest> {
    let (plan, report, binding, authorization) = authorized(store, id)?;
    let (manifest, value) = qualification::registered_json(store, id, "brep-manifest.json")?
        .ok_or_else(|| invalid("registered closed BREP manifest required"))?;
    let decoded: BrepManifest = serde_json::from_value(value)?;
    if decoded.schema_version != 1
        || decoded.gap_healing
        || decoded.synthetic != report.snapshot.synthetic
        || decoded.regions.len() != report.snapshot.regions.len()
        || decoded.regions.len() > 256
    {
        return Err(invalid("original complete named CAD BREP manifest changed"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for record in &decoded.regions {
        let original = report
            .snapshot
            .regions
            .iter()
            .find(|r| r.name == record.region_name)
            .ok_or_else(|| invalid("BREP region differs from registered CAD"))?;
        if !seen.insert(&record.region_name)
            || record.path != format!("{}.brep", original.name)
            || record.bounds_m != original.bounds_m
            || record.volume_m3 != original.volume_m3
            || record.source_transform != original.transform
            || record.source_unit != original.source_unit
            || record.scale_to_m != original.stl_scale_to_m
            || record.placement_translation_unit != "mm"
        {
            return Err(invalid(
                "BREP manifest changed region, original placement, bounds, volume or units",
            ));
        }
    }
    Ok(VerifiedManifest {
        plan,
        report,
        binding,
        authorization,
        manifest,
        synthetic: decoded.synthetic,
        regions: decoded.regions,
    })
}

/// The registered BREP artifact of one manifest region, byte-checked on disk.
pub(crate) fn verified_brep(
    store: &Store,
    id: &str,
    selected: &BrepRegion,
) -> Result<ArtifactManifest> {
    let brep = store
        .artifact_record(id, &selected.path)?
        .ok_or_else(|| invalid("closed registered BREP artifact required"))?;
    let observed = native_manifest(
        &store.job_dir(id)?,
        &selected.path,
        64 * 1024 * 1024,
        "CAD source byte verification",
    )?;
    if observed.sha256 != brep.sha256
        || observed.bytes != brep.bytes
        || selected.sha256 != brep.sha256
        || selected.bytes != brep.bytes
    {
        return Err(invalid(
            "imported CAD BREP bytes differ from original registered evidence",
        ));
    }
    Ok(brep)
}

/// Resolve only a completed, originally authorized CAD job and registered solid.
/// Bounds and placement come from verified importer evidence, never caller overrides.
pub fn source(
    store: &Store,
    id: &str,
    region: &str,
    resolution: u32,
    geometry_tolerance_m: f64,
) -> Result<(PathBuf, CadSource)> {
    if !token(region) {
        return Err(invalid("completed committed named CAD source required"));
    }
    let VerifiedManifest {
        plan,
        report,
        binding,
        authorization,
        manifest,
        synthetic,
        regions,
    } = verified_manifest(store, id)?;
    let selected = regions
        .into_iter()
        .find(|r| r.region_name == region)
        .ok_or_else(|| invalid("approved named BREP region missing"))?;
    let root = store.job_dir(id)?;
    let brep = verified_brep(store, id, &selected)?;
    let geometry = CadMeshDescriptor {
        schema_version: 1,
        synthetic,
        backend: "cpu".into(),
        formulation: "imported_axis_aligned_box".into(),
        geometry_provenance: format!(
            "authorized CAD job {id}, named region {region}; {}",
            plan.channel_case()?.geometry.source
        ),
        brep_file: "solid.brep".into(),
        brep_sha256: brep.sha256.clone(),
        brep_bytes: brep.bytes,
        region_name: region.into(),
        bounds_m: selected.bounds_m,
        volume_m3: selected.volume_m3,
        source_unit: selected.source_unit,
        scale_to_m: selected.scale_to_m,
        placement_translation_unit: selected.placement_translation_unit,
        source_transform: selected.source_transform,
        resolution,
        geometry_tolerance_m,
        volume_relative_tolerance: 1e-10,
    };
    let bound = CadSource {
        schema_version: 1,
        job_id: id.into(),
        science_id: plan.science_id()?,
        execution_id: plan.id()?,
        execution_binding_digest: digest(&binding)?,
        authorization_digest: digest(&authorization)?,
        manifest,
        region_evidence: report.evidence,
        brep,
        geometry,
    };
    bound.validate()?;
    Ok((root, bound))
}

/// Run inside the submission writer transaction before acknowledgment. The
/// verified source is copied to distinct inodes, not exposed through the worker.
pub(crate) fn retain(store: &Store, id: &str, plan: &ExecutionPlan) -> Result<()> {
    let Some(expected) = &plan.cad_source else {
        return Ok(());
    };
    let (root, observed) = source(
        store,
        &expected.job_id,
        &expected.geometry.region_name,
        expected.geometry.resolution,
        expected.geometry.geometry_tolerance_m,
    )?;
    if digest(&observed)? != digest(expected)? {
        return Err(invalid("approved imported CAD source changed"));
    }
    let job_dir = store.job_dir(id)?;
    let destination = job_dir.join("retained-cad");
    if destination.exists() {
        return Err(invalid("immutable CAD source destination already exists"));
    }
    let intent = commit_artifact(
        &job_dir,
        "cad-source-retention.json",
        &serde_json::to_vec(expected)?,
        "json",
        "durable imported CAD staging intent; recovery verifies original source before deleting unpublished copies",
    )?;
    let staging = job_dir.join(format!(".retained-cad-{}", uuid::Uuid::new_v4()));
    private_dir(&staging)?;
    for path in [
        &expected.brep.path,
        &expected.manifest.path,
        &expected.region_evidence.path,
    ] {
        let record = store
            .artifact_record(&expected.job_id, path)?
            .ok_or_else(|| invalid("original CAD source record missing"))?;
        copy_verified(&root, &staging, &record)?;
    }
    if expected.brep.path != "solid.brep" {
        fs::rename(
            safe_path(&staging, &expected.brep.path)?,
            staging.join("solid.brep"),
        )?;
    }
    verify_copies(&staging, expected)?;
    sync_directories(&staging)?;
    publish_directory(&staging, &destination)?;
    fs::File::open(&job_dir)?.sync_all()?;
    for path in ["solid.brep", "brep-manifest.json", "regions.json"] {
        let record = native_manifest(
            &job_dir,
            &format!("retained-cad/{path}"),
            64 * 1024 * 1024,
            "verified distinct-inode immutable CAD source; original mm placement retained beside SI world bounds",
        )?;
        store.add_artifact(id, &record)?;
    }
    let provenance = serde_json::json!({"source":expected,"plan":store.recorded_plan(&expected.job_id)?,"host_profile":store.job_profile(&expected.job_id)?,
        "execution_binding":store.execution_binding(&expected.job_id)?,"execution_authorization":store.execution_authorization(&expected.job_id)?});
    store.add_artifact(
        id,
        &commit_artifact(
            &job_dir,
            "cad-source-execution.json",
            &serde_json::to_vec_pretty(&provenance)?,
            "json",
            "original authorized CAD source execution; no upgraded qualification",
        )?,
    )?;
    store.add_artifact(id, &intent)?;
    Ok(())
}

fn verify_copies(root: &std::path::Path, source: &CadSource) -> Result<()> {
    source.validate()?;
    for (path, hash, bytes) in [
        ("solid.brep", &source.brep.sha256, source.brep.bytes),
        (
            "brep-manifest.json",
            &source.manifest.sha256,
            source.manifest.bytes,
        ),
        (
            "regions.json",
            &source.region_evidence.sha256,
            source.region_evidence.bytes,
        ),
    ] {
        let observed =
            native_manifest(root, path, 64 * 1024 * 1024, "recheck immutable CAD source")?;
        if &observed.sha256 != hash || observed.bytes != bytes {
            return Err(invalid(
                "retained imported CAD bytes differ from approved source",
            ));
        }
    }
    let names = fs::read_dir(root)?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<std::io::Result<std::collections::BTreeSet<_>>>()?;
    if names
        != ["solid.brep", "brep-manifest.json", "regions.json"]
            .into_iter()
            .map(std::ffi::OsString::from)
            .collect()
    {
        return Err(invalid("only exact retained CAD source evidence permitted"));
    }
    Ok(())
}

pub fn registered(store: &Store, id: &str, plan: &ExecutionPlan) -> Result<PathBuf> {
    let expected = plan
        .cad_source
        .as_ref()
        .ok_or_else(|| invalid("source-bound CAD plan required"))?;
    let root = store.job_dir(id)?;
    let intent =
        crate::worker::read_bounded(&safe_path(&root, "cad-source-retention.json")?, MAX_MESSAGE)?;
    let source: CadSource = serde_json::from_slice(&intent)?;
    if digest(&source)? != digest(expected)? {
        return Err(invalid("retained CAD intent differs from approved plan"));
    }
    let copies = safe_path(&root, "retained-cad")?;
    verify_copies(&copies, expected)?;
    for path in ["solid.brep", "brep-manifest.json", "regions.json"] {
        let record = store
            .artifact_record(id, &format!("retained-cad/{path}"))?
            .ok_or_else(|| invalid("retained CAD artifact registration missing"))?;
        let observed = native_manifest(
            &root,
            &record.path,
            64 * 1024 * 1024,
            "recheck retained CAD registration",
        )?;
        if record.sha256 != observed.sha256 || record.bytes != observed.bytes {
            return Err(invalid("retained CAD artifact registry changed"));
        }
    }
    Ok(copies)
}

pub(crate) fn recover_orphan(store: &Store, id: &str) -> Result<()> {
    let root = safe_path(&store.root, &format!("artifacts/{id}"))?;
    let intent = safe_path(&root, "cad-source-retention.json")?;
    if !intent.exists() {
        return Ok(());
    }
    let expected: CadSource =
        serde_json::from_slice(&crate::worker::read_bounded(&intent, MAX_MESSAGE)?)?;
    if let Ok((_, original)) = source(
        store,
        &expected.job_id,
        &expected.geometry.region_name,
        expected.geometry.resolution,
        expected.geometry.geometry_tolerance_m,
    ) && digest(&original)? == digest(&expected)?
    {
        fs::remove_dir_all(&root)?;
        fs::File::open(root.parent().ok_or_else(|| invalid("CAD orphan parent"))?)?.sync_all()?;
    }
    Ok(())
}
