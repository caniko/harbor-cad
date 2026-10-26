//! Version-2 spreadsheet-alias CAD-copy variants and the synthetic enclosure seed.
//!
//! Expressions are the edit mechanism: only existing alias cells of the one
//! registered `Spreadsheet::Sheet` change. Original identity, staging and
//! historical rechecks reuse the version-1 path in `cad_variant`.
use crate::{
    Error, Result,
    cad::{Region, RegionSnapshot},
    cad_source::hash,
    contracts::*,
    qualification::{self, EvidenceRecord},
    science::Quantity,
    storage::{Store, native_manifest, safe_path},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

pub const ENCLOSURE_REGIONS: [&str; 6] = ["housing", "lid", "gasket", "window", "pcb", "mount"];
pub const MAX_PARAMETERS: usize = 32;
/// Material the enclosure fixture must keep beside its rim groove: two 0.75 mm
/// lips. The groove (`groove_width`) is centred in the `wall`, so each lip is
/// (wall - groove_width)/2; thinner lips are sub-millimetre slivers that fail
/// the mesh quality gate and the process minimum wall.
pub const ENCLOSURE_RIM_LIPS_M: f64 = 1.5e-3;

/// Refuse an enclosure-fixture variant whose rim lips would be thinner than
/// `ENCLOSURE_RIM_LIPS_M` in total (SI lengths; unknown values refuse).
pub fn enclosure_rim_precondition(wall_m: Option<f64>, groove_width_m: Option<f64>) -> Result<()> {
    match (wall_m, groove_width_m) {
        // 1 nm slack absorbs binary noise in exact grid values (3.5 = 2 + 1.5 mm).
        (Some(wall), Some(groove)) if wall + 1e-9 >= groove + ENCLOSURE_RIM_LIPS_M => Ok(()),
        (wall, groove) => Err(Error::GeometryPrecondition(format!(
            "enclosure fixture wall {wall:?} m must be at least groove_width {groove:?} m + {ENCLOSURE_RIM_LIPS_M} m (two 0.75 mm rim lips beside the centred gasket groove)"
        ))),
    }
}

/// Either variant request; `schema_version` selects the strict decoder.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum AnyCadVariantRequest {
    Box(crate::cad_variant::CadVariantRequest),
    Sheet(CadSheetVariantRequest),
}
impl<'de> Deserialize<'de> for AnyCadVariantRequest {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(decoder)?;
        if value["schema_version"] == 2 {
            serde_json::from_value(value).map(Self::Sheet)
        } else {
            serde_json::from_value(value).map(Self::Box)
        }
        .map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CadSheetVariantRequest {
    pub schema_version: u32,
    pub source_job: String,
    pub sheet_name: String,
    /// Alias → value with explicit unit: `m`/`mm`/`nm` lengths or `1` (dimensionless).
    pub parameters: BTreeMap<String, Quantity>,
    pub regions: Vec<String>,
    pub geometry_tolerance: Quantity,
    pub provenance: String,
}

/// Validated SI value of one alias.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SheetValue {
    Length(f64),
    Dimensionless(f64),
}

/// FreeCAD object/alias identifier: ASCII letter, then letters, digits or `_`.
fn identifier(name: &str) -> bool {
    name.len() <= 64
        && name.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl CadSheetVariantRequest {
    pub fn normalized(&self) -> Result<(BTreeMap<String, SheetValue>, f64)> {
        let tolerance = self.geometry_tolerance.si("length")?;
        if self.schema_version != 2
            || uuid::Uuid::parse_str(&self.source_job).is_err()
            || !identifier(&self.sheet_name)
            || self.parameters.len() > MAX_PARAMETERS
            || self.regions.is_empty()
            || self.regions.len() > 64
            || self.regions.iter().any(|r| !token(r))
            || self
                .regions
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.regions.len()
            || !(1e-6..=1e-4).contains(&tolerance)
            || self.provenance.trim().is_empty()
            || self.provenance.len() > 4096
        {
            return Err(invalid(
                "explicit sheet, at most 32 aliases, unique named regions, SI tolerance, source identity and provenance required",
            ));
        }
        let mut values = BTreeMap::new();
        for (alias, quantity) in &self.parameters {
            let value = if quantity.unit == "1" {
                if !quantity.value.is_finite() || !(0. ..=1e6).contains(&quantity.value) {
                    return Err(invalid("bounded dimensionless sheet value required"));
                }
                SheetValue::Dimensionless(quantity.value)
            } else {
                let si = quantity.si("length")?;
                if !(1e-5..=10.).contains(&si) {
                    return Err(invalid("sheet lengths must lie within 1e-5..10 m"));
                }
                SheetValue::Length(si)
            };
            if !identifier(alias) {
                return Err(invalid("FreeCAD spreadsheet alias identifier required"));
            }
            values.insert(alias.clone(), value);
        }
        Ok((values, tolerance))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CadSheetSource {
    pub schema_version: u32,
    pub job_id: String,
    pub science_id: String,
    pub execution_id: String,
    pub execution_binding_digest: String,
    pub authorization_digest: String,
    pub region_evidence: EvidenceRecord,
    pub sheet_evidence: EvidenceRecord,
    pub synthetic: bool,
    /// Registered original geometry of exactly the requested regions, in order.
    pub regions: Vec<Region>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CadSheetVariantSpec {
    pub schema_version: u32,
    pub request: CadSheetVariantRequest,
    pub source: CadSheetSource,
    pub document: ArtifactManifest,
}

impl CadSheetVariantSpec {
    pub fn validate(&self) -> Result<()> {
        self.request.normalized()?;
        let s = &self.source;
        if self.schema_version != 2
            || s.schema_version != 1
            || s.job_id != self.request.source_job
            || [
                &s.science_id,
                &s.execution_id,
                &s.execution_binding_digest,
                &s.authorization_digest,
                &s.region_evidence.sha256,
                &s.sheet_evidence.sha256,
            ]
            .iter()
            .any(|h| !hash(h))
            || s.region_evidence.path != "regions.json"
            || !(1..=256 * 1024).contains(&s.region_evidence.bytes)
            || s.sheet_evidence.path != "sheets.json"
            || !(1..=256 * 1024).contains(&s.sheet_evidence.bytes)
            || s.regions
                .iter()
                .map(|r| &r.name)
                .ne(self.request.regions.iter())
            || s.regions
                .iter()
                .any(|r| !finite_solid(&r.bounds_m, r.volume_m3, &r.transform))
            || !crate::cad_variant::document_ok(&self.document)
        {
            return Err(invalid(
                "registered spreadsheet CAD source, requested regions and original document bytes required",
            ));
        }
        Ok(())
    }
}

fn finite_solid(bounds: &[f64; 6], volume: f64, transform: &[f64; 16]) -> bool {
    bounds.iter().chain(transform).all(|v| v.is_finite())
        && bounds.as_chunks::<2>().0.iter().all(|p| p[0] < p[1])
        && transform[12..] == [0., 0., 0., 1.]
        && volume.is_finite()
        && volume > 0.
        && volume
            <= (bounds[1] - bounds[0])
                * (bounds[3] - bounds[2])
                * (bounds[5] - bounds[4])
                * (1. + 1e-9)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SheetEvidence {
    schema_version: u32,
    sheets: Vec<SheetRecord>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SheetRecord {
    name: String,
    #[allow(dead_code)]
    label: String,
    aliases: BTreeMap<String, AliasRecord>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AliasRecord {
    #[allow(dead_code)]
    cell: String,
    dimension: String,
    value_si: Option<f64>,
}

fn resolve(store: &Store, request: CadSheetVariantRequest) -> Result<CadSheetVariantSpec> {
    let (values, _) = request.normalized()?;
    let id = request.source_job.clone();
    let (plan, report, binding, authorization) = crate::cad_source::authorized(store, &id)?;
    let regions = request
        .regions
        .iter()
        .map(|name| {
            report
                .snapshot
                .regions
                .iter()
                .find(|r| &r.name == name)
                .cloned()
                .ok_or_else(|| invalid("requested region is not registered in the source CAD job"))
        })
        .collect::<Result<Vec<_>>>()?;
    let (sheet_evidence, value) = qualification::registered_json(store, &id, "sheets.json")?
        .ok_or_else(|| invalid("registered spreadsheet alias evidence required"))?;
    let evidence: SheetEvidence = serde_json::from_value(value)?;
    if evidence.schema_version != 1
        || evidence.sheets.len() != 1
        || evidence.sheets[0].name != request.sheet_name
    {
        return Err(invalid(
            "source document must contain exactly one Spreadsheet::Sheet with the requested name",
        ));
    }
    let aliases = &evidence.sheets[0].aliases;
    for (alias, value) in &values {
        let record = aliases
            .get(alias)
            .ok_or_else(|| invalid("only existing spreadsheet aliases may be set"))?;
        if !matches!(
            (record.dimension.as_str(), value),
            ("length", SheetValue::Length(_)) | ("dimensionless", SheetValue::Dimensionless(_))
        ) {
            return Err(invalid("sheet value must keep the alias dimension"));
        }
    }
    if plan.channel_case()?.applicability.formulation == "enclosure_fixture" {
        // Effective value: the requested edit, else the source sheet's value.
        let length = |alias: &str| match values.get(alias) {
            Some(SheetValue::Length(value)) => Some(*value),
            Some(SheetValue::Dimensionless(_)) => None,
            None => aliases
                .get(alias)
                .filter(|r| r.dimension == "length")
                .and_then(|r| r.value_si),
        };
        enclosure_rim_precondition(length("wall"), length("groove_width"))?;
    }
    let document = crate::cad_variant::original_document(store, &id, &plan)?;
    let spec = CadSheetVariantSpec {
        schema_version: 2,
        source: CadSheetSource {
            schema_version: 1,
            job_id: id,
            science_id: plan.science_id()?,
            execution_id: plan.id()?,
            execution_binding_digest: digest(&binding)?,
            authorization_digest: digest(&authorization)?,
            region_evidence: report.evidence,
            sheet_evidence,
            synthetic: report.snapshot.synthetic,
            regions,
        },
        request,
        document,
    };
    spec.validate()?;
    Ok(spec)
}

pub fn plan(
    store: &Store,
    request: CadSheetVariantRequest,
    policy: String,
) -> Result<ExecutionPlan> {
    let spec = resolve(store, request)?;
    let plan = store.plan(&spec.request.source_job)?;
    ExecutionPlan::cad_sheet_variant(plan.channel_case()?.clone(), spec, policy)
}

/// Recheck the approved original against its registered source job.
pub fn source(store: &Store, spec: &CadSheetVariantSpec) -> Result<PathBuf> {
    if digest(&resolve(store, spec.request.clone())?)? != digest(spec)? {
        return Err(invalid("approved spreadsheet CAD variant source changed"));
    }
    store.job_dir(&spec.source.job_id)
}

impl ExecutionPlan {
    pub fn cad_sheet_variant(
        mut case: CaseSpec,
        spec: CadSheetVariantSpec,
        policy: String,
    ) -> Result<Self> {
        spec.validate()?;
        case.geometry.source = "source-document.FCStd".into();
        case.geometry.sha256 = Some(spec.document.sha256.clone());
        case.regions = spec.request.regions.clone();
        case.geometry_tolerance = Quantity {
            value: spec.request.geometry_tolerance.si("length")?,
            unit: "m".into(),
        };
        let mut plan = ExecutionPlan::cad_inspection(case, policy, 256 * 1024 * 1024)?;
        plan.schema_version = 16;
        plan.cad_sheet_variant = Some(spec);
        crate::estimates::minimum(&plan)?.apply(&mut plan);
        plan.validate()?;
        Ok(plan)
    }

    /// Fixed deterministic synthetic enclosure seed; its `Params` sheet is the
    /// only intended edit surface for later version-2 variants.
    pub fn enclosure_fixture(policy: String) -> Result<Self> {
        let mut case = CaseSpec::reference();
        case.name = "synthetic-enclosure".into();
        case.geometry.source = "synthetic outdoor enclosure seed fixture".into();
        case.regions = ENCLOSURE_REGIONS.map(String::from).to_vec();
        case.applicability.formulation = "enclosure_fixture".into();
        case.claims = vec![];
        let stage = |id: &str, dependencies: &[&str], operation, ram_bytes| Stage {
            id: id.into(),
            dependencies: dependencies.iter().map(|d| d.to_string()).collect(),
            operation,
            gpu: GpuRequirement::CpuOnly,
            selection: None,
            ram_bytes,
            vram_bytes: 0,
        };
        let mut plan = Self {
            case: Some(case),
            stages: vec![
                stage(
                    "cad",
                    &[],
                    StageOperation::EnclosureFixture,
                    1024 * 1024 * 1024,
                ),
                stage("bundle", &["cad"], StageOperation::Bundle, 16 * 1024 * 1024),
            ],
            observation: ObservationPlan {
                metrics: vec!["geometry_regions".into()],
                probes: vec![],
                retained_times_s: vec![],
                checkpoint_times_s: vec![],
                preview_times_s: vec![],
                max_artifact_bytes: 256 * 1024 * 1024,
                scientific_congestion: "fail".into(),
                preview_may_drop: false,
            },
            policy,
            // Every optional recipe stays absent, exactly as in the reference plan.
            ..Self::reference(CaseSpec::reference())?
        };
        crate::estimates::minimum(&plan)?.apply(&mut plan);
        plan.validate()?;
        Ok(plan)
    }
}

/// Recomputed named geometry must be exactly the requested original regions.
pub fn verify_snapshot(spec: &CadSheetVariantSpec, snapshot: &RegionSnapshot) -> Result<()> {
    let (_, tolerance) = spec.request.normalized()?;
    spec.validate()?;
    // Native export order follows the document, not the request.
    let names: std::collections::BTreeSet<_> = snapshot.regions.iter().map(|r| &r.name).collect();
    if names.len() != snapshot.regions.len()
        || names != spec.request.regions.iter().collect()
        || snapshot.synthetic != spec.source.synthetic
        || snapshot.gap_healing
        || snapshot.geometry_tolerance.si("length")? != tolerance
    {
        return Err(invalid(
            "recomputed spreadsheet variant must report exactly the requested named regions",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    schema_version: u32,
    approved_variant: serde_json::Value,
    source_preserved: bool,
    object_type: String,
    sheet_name: String,
    parameters: BTreeMap<String, CellChange>,
    only_sheet_cells_changed: bool,
    before: BTreeMap<String, Geometry>,
    after: BTreeMap<String, Metrics>,
    min_wall_method: String,
    gap_healing: bool,
    expressions: bool,
    document: DocumentRecord,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CellChange {
    cell: String,
    before: String,
    after: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Geometry {
    bounds_m: [f64; 6],
    volume_m3: f64,
    transform: [f64; 16],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metrics {
    bounds_m: [f64; 6],
    volume_m3: f64,
    transform: [f64; 16],
    surface_area_m2: f64,
    min_wall_m: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentRecord {
    path: String,
    sha256: String,
    bytes: u64,
}

fn same(bounds: &[f64; 6], volume: f64, region: &Region, tolerance: f64) -> bool {
    bounds
        .iter()
        .zip(&region.bounds_m)
        .all(|(a, b)| (a - b).abs() <= tolerance)
        && (volume - region.volume_m3).abs() <= 1e-10 * region.volume_m3
}

/// Check the native recompute receipt and every re-exported region before
/// publication. Geometry metrics are checked for internal consistency only.
pub(crate) fn verify_outputs(
    plan: &ExecutionPlan,
    root: &std::path::Path,
    receipt: &serde_json::Value,
) -> Result<()> {
    let spec = plan
        .cad_sheet_variant
        .as_ref()
        .ok_or_else(|| invalid("approved spreadsheet CAD variant required"))?;
    let (_, tolerance) = spec.request.normalized()?;
    spec.validate()?;
    crate::cad_variant::importer_receipt(receipt)?;
    let report: Report = serde_json::from_slice(&crate::worker::read_bounded(
        &safe_path(root, "cad-variant-recompute.json")?,
        MAX_MESSAGE,
    )?)?;
    if report.schema_version != 2
        || report.approved_variant != serde_json::to_value(spec)?
        || !report.source_preserved
        || report.object_type != "Spreadsheet::Sheet"
        || report.sheet_name != spec.request.sheet_name
        || report.parameters.keys().ne(spec.request.parameters.keys())
        || report
            .parameters
            .values()
            .any(|c| c.cell.is_empty() || c.before.is_empty() || c.after.is_empty())
        || !report.only_sheet_cells_changed
        || report.min_wall_method.trim().is_empty()
        || report.gap_healing
        || !report.expressions
        || report.before.len() != spec.source.regions.len()
        || report.after.len() != spec.request.regions.len()
    {
        return Err(invalid(
            "complete approved spreadsheet-only recompute evidence required",
        ));
    }
    for region in &spec.source.regions {
        let before = report
            .before
            .get(&region.name)
            .ok_or_else(|| invalid("native original region missing"))?;
        if before.transform != region.transform
            || !same(&before.bounds_m, before.volume_m3, region, tolerance)
        {
            return Err(invalid(
                "native original differs from registered CAD geometry",
            ));
        }
    }
    let snapshot: RegionSnapshot = serde_json::from_slice(&crate::worker::read_bounded(
        &safe_path(root, "regions.json")?,
        256 * 1024,
    )?)?;
    snapshot.verify(plan.channel_case()?, StageOperation::CadInspect)?;
    verify_snapshot(spec, &snapshot)?;
    for region in &snapshot.regions {
        let m = report
            .after
            .get(&region.name)
            .ok_or_else(|| invalid("recomputed region metrics missing"))?;
        let extent = m
            .bounds_m
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| p[1] - p[0])
            .fold(f64::INFINITY, f64::min);
        // Any closed solid satisfies the isoperimetric inequality A³ ≥ 36πV².
        if !finite_solid(&m.bounds_m, m.volume_m3, &m.transform)
            || m.transform != region.transform
            || !same(&m.bounds_m, m.volume_m3, region, tolerance)
            || !m.surface_area_m2.is_finite()
            || m.surface_area_m2.powi(3)
                < 36. * std::f64::consts::PI * m.volume_m3.powi(2) * (1. - 1e-9)
            || !m.min_wall_m.is_finite()
            || m.min_wall_m <= 0.
            || m.min_wall_m > extent + tolerance
        {
            return Err(invalid(
                "recomputed region volume, bounds, area or wall estimate inconsistent",
            ));
        }
        let mesh = native_manifest(
            root,
            &format!("{}.stl", region.name),
            64 * 1024 * 1024,
            "verify closed spreadsheet variant STL",
        )?;
        if mesh.bytes == 0 {
            return Err(invalid("nonempty spreadsheet variant STL required"));
        }
    }
    crate::cad_source::verify_brep_outputs(root, &snapshot)?;
    let document = native_manifest(
        root,
        "variant.FCStd",
        64 * 1024 * 1024,
        "verify closed spreadsheet CAD variant",
    )?;
    if report.document.path != "variant.FCStd"
        || report.document.bytes != document.bytes
        || report.document.sha256 != document.sha256
        || document.bytes == 0
    {
        return Err(invalid(
            "closed native variant document differs from recompute receipt",
        ));
    }
    Ok(())
}

/// Recheck registered original and output bytes; data integrity only.
pub fn verify_registered_outputs(
    store: &Store,
    id: &str,
    plan: &ExecutionPlan,
    receipt: &serde_json::Value,
) -> Result<()> {
    crate::cad_variant::registered(store, id, plan)?;
    let root = store.job_dir(id)?;
    let spec = plan
        .cad_sheet_variant
        .as_ref()
        .ok_or_else(|| invalid("spreadsheet CAD variant required"))?;
    let mut paths = vec![
        "regions.json".to_string(),
        "cad-variant-recompute.json".into(),
        "variant.FCStd".into(),
        "brep-manifest.json".into(),
    ];
    for name in &spec.request.regions {
        paths.push(format!("{name}.brep"));
        paths.push(format!("{name}.stl"));
    }
    for path in paths {
        let expected = store
            .artifact_record(id, &path)?
            .ok_or_else(|| invalid("complete registered spreadsheet variant outputs required"))?;
        let observed = native_manifest(
            &root,
            &path,
            64 * 1024 * 1024,
            "recheck historical spreadsheet variant",
        )?;
        if expected.bytes == 0
            || expected.bytes != observed.bytes
            || expected.sha256 != observed.sha256
        {
            return Err(invalid(
                "registered spreadsheet CAD variant output bytes changed",
            ));
        }
    }
    verify_outputs(plan, &root, receipt)
}
