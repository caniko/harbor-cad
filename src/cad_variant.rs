//! Source-bound native box-copy edits through the existing patched CAD sandbox.
use crate::{
    Result,
    cad_source::CadSource,
    contracts::*,
    science::Quantity,
    storage::{Store, native_manifest, safe_path},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CadVariantRequest {
    pub schema_version: u32,
    pub source_job: String,
    pub region_name: String,
    pub dimensions: [Quantity; 3],
    pub geometry_tolerance: Quantity,
    pub provenance: String,
}

impl CadVariantRequest {
    pub fn normalized(&self) -> Result<([f64; 3], f64)> {
        let dimensions = [
            self.dimensions[0].si("length")?,
            self.dimensions[1].si("length")?,
            self.dimensions[2].si("length")?,
        ];
        let tolerance = self.geometry_tolerance.si("length")?;
        if self.schema_version != 1
            || uuid::Uuid::parse_str(&self.source_job).is_err()
            || !token(&self.region_name)
            || self.provenance.trim().is_empty()
            || self.provenance.len() > 4096
            || dimensions.iter().any(|d| !(1e-5..=10.).contains(d))
            || dimensions.iter().copied().fold(0., f64::max)
                / dimensions.iter().copied().fold(f64::INFINITY, f64::min)
                > 1000.
            || !(1e-10..=1e-4).contains(&tolerance)
            || dimensions.iter().any(|d| tolerance >= 0.001 * d)
        {
            return Err(invalid(
                "explicit bounded native box dimensions, SI tolerance, source identity and provenance required",
            ));
        }
        Ok((dimensions, tolerance))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CadVariantSpec {
    pub schema_version: u32,
    pub request: CadVariantRequest,
    pub source: CadSource,
    pub document: ArtifactManifest,
}
impl CadVariantSpec {
    pub fn validate(&self) -> Result<()> {
        let (_, tolerance) = self.request.normalized()?;
        self.source.validate()?;
        let transform = self.source.geometry.source_transform;
        let expected = [
            1.,
            0.,
            0.,
            transform[3],
            0.,
            1.,
            0.,
            transform[7],
            0.,
            0.,
            1.,
            transform[11],
            0.,
            0.,
            0.,
            1.,
        ];
        if self.schema_version != 1
            || self.request.source_job != self.source.job_id
            || self.request.region_name != self.source.geometry.region_name
            || transform != expected
            || self.source.geometry.geometry_tolerance_m != tolerance
            || !document_ok(&self.document)
        {
            return Err(invalid(
                "single axis-aligned registered CAD source and original document bytes required",
            ));
        }
        for axis in 0..3 {
            if (transform[4 * axis + 3] * 0.001 - self.source.geometry.bounds_m[2 * axis]).abs()
                > tolerance
            {
                return Err(invalid(
                    "original box origin must match its world placement",
                ));
            }
        }
        Ok(())
    }
    pub fn bounds_m(&self) -> Result<[f64; 6]> {
        self.validate()?;
        let (dimensions, _) = self.request.normalized()?;
        let mut bounds = self.source.geometry.bounds_m;
        for axis in 0..3 {
            bounds[2 * axis + 1] = bounds[2 * axis] + dimensions[axis];
        }
        Ok(bounds)
    }
}

/// Exact registered original FCStd record shape shared by every variant recipe.
pub(crate) fn document_ok(document: &ArtifactManifest) -> bool {
    ["source.FCStd", "input.FCStd", "variant.FCStd"].contains(&document.path.as_str())
        && document.schema_version == 1
        && document.format == "FCStd"
        && (1..=64 * 1024 * 1024).contains(&document.bytes)
        && document.sha256.len() == 64
        && document
            .sha256
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        && document.units.is_none()
        && document.time_s.is_none()
        && document.association.is_none()
}

/// The registered original document of a completed CAD job, rechecked on disk.
pub(crate) fn original_document(
    store: &Store,
    id: &str,
    plan: &ExecutionPlan,
) -> Result<ArtifactManifest> {
    let variant = plan.cad_variant.is_some() || plan.cad_sheet_variant.is_some();
    let inspected = plan
        .stages
        .iter()
        .any(|s| s.operation == StageOperation::CadInspect);
    let path = if variant {
        "variant.FCStd"
    } else if inspected {
        "input.FCStd"
    } else {
        "source.FCStd"
    };
    let document = store
        .artifact_record(id, path)?
        .ok_or_else(|| invalid("registered original CAD document required"))?;
    let actual = native_manifest(
        &store.job_dir(id)?,
        path,
        64 * 1024 * 1024,
        "verify original CAD document",
    )?;
    if document.bytes != actual.bytes || document.sha256 != actual.sha256 {
        return Err(invalid("registered CAD document bytes changed"));
    }
    if !variant
        && inspected
        && plan.channel_case()?.geometry.sha256.as_deref() != Some(document.sha256.as_str())
    {
        return Err(invalid(
            "original inspected CAD document differs from its immutable approval",
        ));
    }
    Ok(document)
}

fn resolve(store: &Store, request: CadVariantRequest) -> Result<CadVariantSpec> {
    let (_, tolerance) = request.normalized()?;
    let report = crate::cad::regions(store, &request.source_job)?;
    if report.snapshot.regions.len() != 1 || report.snapshot.regions[0].name != request.region_name
    {
        return Err(invalid(
            "single unambiguous registered native box region required",
        ));
    }
    let (_, source) = crate::cad_source::source(
        store,
        &request.source_job,
        &request.region_name,
        2,
        tolerance,
    )?;
    let plan = store.recorded_plan(&request.source_job)?;
    let document = original_document(store, &request.source_job, &plan)?;
    let spec = CadVariantSpec {
        schema_version: 1,
        request,
        source,
        document,
    };
    spec.validate()?;
    Ok(spec)
}
pub fn plan(store: &Store, request: CadVariantRequest, policy: String) -> Result<ExecutionPlan> {
    let spec = resolve(store, request)?;
    let plan = store.plan(&spec.request.source_job)?;
    ExecutionPlan::cad_variant(plan.channel_case()?.clone(), spec, policy)
}
impl ExecutionPlan {
    pub fn cad_variant(mut case: CaseSpec, spec: CadVariantSpec, policy: String) -> Result<Self> {
        spec.validate()?;
        case.geometry.source = "source-document.FCStd".into();
        case.geometry.sha256 = Some(spec.document.sha256.clone());
        case.geometry_tolerance = Quantity {
            value: spec.request.geometry_tolerance.si("length")?,
            unit: "m".into(),
        };
        // Reuse the original case's named-region/unit context, never solver stages.
        let mut plan = ExecutionPlan::cad_inspection(case, policy, 256 * 1024 * 1024)?;
        plan.schema_version = 16;
        plan.cad_variant = Some(spec);
        crate::estimates::minimum(&plan)?.apply(&mut plan);
        plan.validate()?;
        Ok(plan)
    }
}
pub fn source(store: &Store, spec: &CadVariantSpec) -> Result<PathBuf> {
    if digest(&resolve(store, spec.request.clone())?)? != digest(spec)? {
        return Err(invalid("approved CAD variant source changed"));
    }
    store.job_dir(&spec.source.job_id)
}

/// Original identity shared by box (v1) and spreadsheet (v2) approvals, so both
/// reuse one staging, retention and historical recheck path.
pub(crate) struct Original<'a> {
    pub job_id: &'a str,
    pub science_id: &'a str,
    pub execution_id: &'a str,
    pub binding_digest: &'a str,
    pub authorization_digest: &'a str,
    pub document: &'a ArtifactManifest,
    pub regions: &'a crate::qualification::EvidenceRecord,
    pub spec: Vec<u8>,
}

pub(crate) fn original(plan: &ExecutionPlan) -> Result<Option<Original<'_>>> {
    if let Some(spec) = &plan.cad_variant {
        spec.validate()?;
        let s = &spec.source;
        return Ok(Some(Original {
            job_id: &s.job_id,
            science_id: &s.science_id,
            execution_id: &s.execution_id,
            binding_digest: &s.execution_binding_digest,
            authorization_digest: &s.authorization_digest,
            document: &spec.document,
            regions: &s.region_evidence,
            spec: serde_json::to_vec(spec)?,
        }));
    }
    let Some(spec) = &plan.cad_sheet_variant else {
        return Ok(None);
    };
    spec.validate()?;
    let s = &spec.source;
    Ok(Some(Original {
        job_id: &s.job_id,
        science_id: &s.science_id,
        execution_id: &s.execution_id,
        binding_digest: &s.execution_binding_digest,
        authorization_digest: &s.authorization_digest,
        document: &spec.document,
        regions: &s.region_evidence,
        spec: serde_json::to_vec(spec)?,
    }))
}

/// Re-resolve the approved original from the registered source job.
fn recheck(store: &Store, plan: &ExecutionPlan) -> Result<PathBuf> {
    match (&plan.cad_variant, &plan.cad_sheet_variant) {
        (Some(spec), None) => source(store, spec),
        (None, Some(spec)) => crate::cad_sheet_variant::source(store, spec),
        _ => Err(invalid("exactly one approved CAD variant required")),
    }
}

pub(crate) fn retain(store: &Store, id: &str, plan: &ExecutionPlan) -> Result<()> {
    let Some(spec) = original(plan)? else {
        return Ok(());
    };
    let original = recheck(store, plan)?;
    let root = store.job_dir(id)?;
    let intent = crate::storage::commit_artifact(
        &root,
        "cad-variant-retention.json",
        &spec.spec,
        "json",
        "durable original CAD-copy staging intent; preserve unverifiable orphan originals",
    )?;
    let staging = root.join(format!(".variant-source-{}", uuid::Uuid::new_v4()));
    crate::storage::private_dir(&staging)?;
    crate::storage::copy_verified(&original, &staging, spec.document)?;
    crate::storage::copy_verified(
        &original,
        &staging,
        &store
            .artifact_record(spec.job_id, &spec.regions.path)?
            .ok_or_else(|| invalid("original variant region evidence required"))?,
    )?;
    std::fs::rename(
        safe_path(&staging, &spec.document.path)?,
        staging.join("source-document.FCStd"),
    )?;
    std::fs::rename(
        safe_path(&staging, &spec.regions.path)?,
        staging.join("variant-source-regions.json"),
    )?;
    crate::storage::sync_directories(&staging)?;
    // Each file is published atomically; the durable writer transaction only
    // acknowledges when all inputs and original execution provenance exist.
    for path in ["source-document.FCStd", "variant-source-regions.json"] {
        let destination = safe_path(&root, path)?;
        if destination.exists() {
            return Err(invalid("immutable variant destination already exists"));
        }
        std::fs::rename(staging.join(path), &destination)?;
        store.add_artifact(
            id,
            &native_manifest(
                &root,
                path,
                64 * 1024 * 1024,
                "verified distinct-inode source for controlled CAD-copy variant",
            )?,
        )?;
    }
    std::fs::remove_dir(&staging)?;
    let source: serde_json::Value = serde_json::from_slice(&spec.spec)?;
    let provenance = serde_json::json!({"source":source,"plan":store.recorded_plan(spec.job_id)?,"host_profile":store.job_profile(spec.job_id)?,"execution_binding":store.execution_binding(spec.job_id)?,"execution_authorization":store.execution_authorization(spec.job_id)?});
    store.add_artifact(
        id,
        &crate::storage::commit_artifact(
            &root,
            "cad-variant-source-execution.json",
            &serde_json::to_vec_pretty(&provenance)?,
            "json",
            "original CAD source execution and approval identities; not upgraded qualification",
        )?,
    )?;
    store.add_artifact(id, &intent)?;
    std::fs::File::open(&root)?.sync_all()?;
    registered(store, id, plan)?;
    Ok(())
}
pub fn registered(store: &Store, id: &str, plan: &ExecutionPlan) -> Result<PathBuf> {
    let spec = original(plan)?.ok_or_else(|| invalid("approved CAD variant required"))?;
    let approved: serde_json::Value = serde_json::from_slice(&spec.spec)?;
    let root = store.job_dir(id)?;
    let (_, intent) =
        crate::qualification::registered_json(store, id, "cad-variant-retention.json")?
            .ok_or_else(|| invalid("registered original CAD variant staging intent required"))?;
    if intent != approved {
        return Err(invalid("CAD variant source-retention intent changed"));
    }
    let (_, execution) =
        crate::qualification::registered_json(store, id, "cad-variant-source-execution.json")?
            .ok_or_else(|| invalid("retained original CAD execution provenance required"))?;
    let original: ExecutionPlan = serde_json::from_value(execution["plan"].clone())?;
    let binding: crate::execution::ExecutionBinding =
        serde_json::from_value(execution["execution_binding"].clone())?;
    let profile: HostExecutionProfile = serde_json::from_value(execution["host_profile"].clone())?;
    let authorization: crate::authority::ExecutionAuthorization =
        serde_json::from_value(execution["execution_authorization"].clone())?;
    authorization.verify(&original, &profile, &binding)?;
    if execution["source"] != approved
        || original.id()? != spec.execution_id
        || original.science_id()? != spec.science_id
        || digest(&binding)? != spec.binding_digest
        || digest(&authorization)? != spec.authorization_digest
    {
        return Err(invalid(
            "retained CAD source science/execution/binding/authorization changed",
        ));
    }
    for (path, hash, bytes) in [
        (
            "source-document.FCStd",
            &spec.document.sha256,
            spec.document.bytes,
        ),
        (
            "variant-source-regions.json",
            &spec.regions.sha256,
            spec.regions.bytes,
        ),
    ] {
        let record = store
            .artifact_record(id, path)?
            .ok_or_else(|| invalid("registered variant source copy required"))?;
        let observed =
            native_manifest(&root, path, 64 * 1024 * 1024, "recheck variant source copy")?;
        if record.sha256 != *hash
            || record.bytes != bytes
            || observed.sha256 != *hash
            || observed.bytes != bytes
        {
            return Err(invalid("original retained CAD variant bytes changed"));
        }
    }
    safe_path(&root, "source-document.FCStd")
}
pub(crate) fn recover_orphan(store: &Store, id: &str) -> Result<()> {
    let root = safe_path(&store.root, &format!("artifacts/{id}"))?;
    let intent = root.join("cad-variant-retention.json");
    if !intent.exists() {
        return Ok(());
    }
    let intent: serde_json::Value =
        serde_json::from_slice(&crate::worker::read_bounded(&intent, MAX_MESSAGE)?)?;
    let verifiable = if intent["schema_version"] == 2 {
        crate::cad_sheet_variant::source(store, &serde_json::from_value(intent)?).is_ok()
    } else {
        source(store, &serde_json::from_value(intent)?).is_ok()
    };
    if verifiable {
        std::fs::remove_dir_all(&root)?;
        std::fs::File::open(
            root.parent()
                .ok_or_else(|| invalid("variant orphan parent required"))?,
        )?
        .sync_all()?;
    }
    Ok(())
}

/// Verify geometry from the approved original context, independent of FreeCAD.
pub fn verify_snapshot(spec: &CadVariantSpec, snapshot: &crate::cad::RegionSnapshot) -> Result<()> {
    let (dimensions, tolerance) = spec.request.normalized()?;
    spec.validate()?;
    if snapshot.regions.len() != 1
        || snapshot.regions[0].name != spec.request.region_name
        || snapshot.synthetic != spec.source.geometry.synthetic
        || snapshot.gap_healing
        || snapshot.geometry_tolerance.si("length")? != tolerance
    {
        return Err(invalid(
            "recomputed variant must preserve unique original named geometry and provenance",
        ));
    }
    let region = &snapshot.regions[0];
    let bounds = spec.bounds_m()?;
    let volume: f64 = dimensions.iter().product();
    if region.transform != spec.source.geometry.source_transform
        || region.source_unit != "mm"
        || region.stl_scale_to_m != 0.001
        || region.triangles == 0
        || !region.volume_m3.is_finite()
        || (region.volume_m3 - volume).abs() > 1e-10 * volume
        || region
            .bounds_m
            .iter()
            .zip(bounds)
            .any(|(v, expected)| !v.is_finite() || (v - expected).abs() > tolerance)
    {
        return Err(invalid(
            "recomputed variant volume/bounds/placement differ from explicit box edit",
        ));
    }
    Ok(())
}

pub(crate) fn importer_receipt(receipt: &serde_json::Value) -> Result<()> {
    if receipt["adapter"] != "FreeCAD"
        || receipt["executed"] != true
        || receipt["backend"] != "cpu"
        || receipt["security_minimum"] != "1.1.4"
        || receipt["sandbox_required"] != true
        || receipt["import_policy"] != crate::sandbox::IMPORT_POLICY
    {
        return Err(invalid(
            "patched operation-specific CAD execution receipt required",
        ));
    }
    Ok(())
}

pub(crate) fn verify_outputs(
    plan: &ExecutionPlan,
    root: &std::path::Path,
    receipt: &serde_json::Value,
) -> Result<()> {
    let spec = plan
        .cad_variant
        .as_ref()
        .ok_or_else(|| invalid("approved controlled CAD variant required"))?;
    spec.validate()?;
    importer_receipt(receipt)?;
    let report: serde_json::Value = serde_json::from_slice(&crate::worker::read_bounded(
        &safe_path(root, "cad-variant-recompute.json")?,
        MAX_MESSAGE,
    )?)?;
    if report["schema_version"] != 1
        || report["approved_variant"] != serde_json::to_value(spec)?
        || report["source_preserved"] != true
        || report["object_type"] != "Part::Box"
        || report["gap_healing"] != false
        || report["expressions"] != false
    {
        return Err(invalid(
            "complete approved native primitive recompute evidence required",
        ));
    }
    let (_, tolerance) = spec.request.normalized()?;
    let dimensions = spec.request.normalized()?.0;
    let before = spec.source.geometry.lengths_m();
    for (key, bounds, dims, volume) in [
        (
            "before",
            spec.source.geometry.bounds_m,
            before,
            spec.source.geometry.volume_m3,
        ),
        (
            "after",
            spec.bounds_m()?,
            dimensions,
            dimensions.iter().product(),
        ),
    ] {
        let observed_bounds: [f64; 6] = serde_json::from_value(report[key]["bounds_m"].clone())?;
        let observed_dims: [f64; 3] = serde_json::from_value(report[key]["dimensions_m"].clone())?;
        let transform: [f64; 16] = serde_json::from_value(report[key]["transform"].clone())?;
        let observed_volume = report[key]["volume_m3"]
            .as_f64()
            .ok_or_else(|| invalid("finite original recompute volume required"))?;
        if transform != spec.source.geometry.source_transform
            || !observed_volume.is_finite()
            || (observed_volume - volume).abs() > 1e-10 * volume
            || observed_bounds
                .into_iter()
                .zip(bounds)
                .chain(observed_dims.into_iter().zip(dims))
                .any(|(a, b)| !a.is_finite() || (a - b).abs() > tolerance)
        {
            return Err(invalid(
                "native original/recomputed dimensions, world bounds, volume or placement changed",
            ));
        }
    }
    let snapshot: crate::cad::RegionSnapshot = serde_json::from_slice(
        &crate::worker::read_bounded(&safe_path(root, "regions.json")?, 256 * 1024)?,
    )?;
    verify_snapshot(spec, &snapshot)?;
    crate::cad_source::verify_brep_outputs(root, &snapshot)?;
    let mesh = native_manifest(
        root,
        &format!("{}.stl", spec.request.region_name),
        64 * 1024 * 1024,
        "verify closed controlled variant STL",
    )?;
    if mesh.bytes == 0 {
        return Err(invalid("nonempty controlled variant STL required"));
    }
    let document = native_manifest(
        root,
        "variant.FCStd",
        64 * 1024 * 1024,
        "verify closed native CAD variant",
    )?;
    if report["document"]["path"] != "variant.FCStd"
        || report["document"]["bytes"] != document.bytes
        || report["document"]["sha256"] != document.sha256
        || document.bytes == 0
    {
        return Err(invalid(
            "closed native variant document differs from recompute receipt",
        ));
    }
    Ok(())
}
/// Recheck complete registered original geometry and recompute evidence. This
/// verifies data integrity only; execution qualification still requires the
/// exact native runtime binding and successful owned-service evidence.
pub fn verify_registered_outputs(
    store: &Store,
    id: &str,
    plan: &ExecutionPlan,
    receipt: &serde_json::Value,
) -> Result<()> {
    registered(store, id, plan)?;
    let root = store.job_dir(id)?;
    let name = &plan
        .cad_variant
        .as_ref()
        .ok_or_else(|| invalid("controlled CAD variant required"))?
        .request
        .region_name;
    for path in [
        "regions.json".into(),
        "cad-variant-recompute.json".into(),
        "variant.FCStd".into(),
        "brep-manifest.json".into(),
        format!("{name}.brep"),
        format!("{name}.stl"),
    ] {
        let expected = store
            .artifact_record(id, &path)?
            .ok_or_else(|| invalid("complete registered original CAD variant outputs required"))?;
        let observed = native_manifest(
            &root,
            &path,
            64 * 1024 * 1024,
            "recheck historical native variant original",
        )?;
        if expected.bytes == 0
            || expected.bytes != observed.bytes
            || expected.sha256 != observed.sha256
        {
            return Err(invalid(
                "registered native CAD variant output bytes changed",
            ));
        }
    }
    verify_outputs(plan, &root, receipt)
}
