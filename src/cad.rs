//! Bounded views of registered CAD region evidence; no document open on queries.
use crate::{
    Result,
    contracts::*,
    qualification::{self, EvidenceRecord, EvidenceState},
    science::Quantity,
    storage::Store,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub name: String,
    pub label: String,
    pub volume_m3: f64,
    pub bounds_m: [f64; 6],
    /// Original FreeCAD placement: matrix translation entries are millimetres.
    pub transform: [f64; 16],
    pub triangles: u64,
    pub source_unit: String,
    pub stl_scale_to_m: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegionSnapshot {
    pub synthetic: bool,
    pub regions: Vec<Region>,
    pub gap_healing: bool,
    pub geometry_tolerance: Quantity,
}

impl RegionSnapshot {
    pub fn verify(&self, case: &CaseSpec, operation: StageOperation) -> Result<()> {
        let mut expected: BTreeSet<_> = case.regions.iter().map(String::as_str).collect();
        if matches!(operation, StageOperation::CadFixture) {
            expected.remove("wall");
        } else if !matches!(
            operation,
            StageOperation::CadInspect | StageOperation::EnclosureFixture
        ) {
            return Err(invalid("CAD region snapshot requires a CAD operation"));
        }
        let names: BTreeSet<_> = self.regions.iter().map(|r| r.name.as_str()).collect();
        if names != expected
            || names.len() != self.regions.len()
            || names.is_empty()
            || names.len() > 256
            || self.synthetic != case.geometry.synthetic
            || self.gap_healing
            || self.geometry_tolerance.si("length")? != case.geometry_tolerance.si("length")?
        {
            return Err(invalid(
                "region identity, synthetic provenance or approved tolerance changed",
            ));
        }
        for region in &self.regions {
            if !token(&region.name)
                || region.label.len() > 1024
                || !region.volume_m3.is_finite()
                || region.volume_m3 <= 0.
                || region
                    .bounds_m
                    .iter()
                    .chain(&region.transform)
                    .any(|v| !v.is_finite())
                || region
                    .bounds_m
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|pair| pair[0] >= pair[1])
                || region.transform[12..] != [0., 0., 0., 1.]
                || region.source_unit != "mm"
                || region.stl_scale_to_m != 0.001
                || region.triangles == 0
            {
                return Err(invalid(
                    "finite positive closed-solid bounds, affine placement and explicit CAD units required",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegionReport {
    pub schema_version: u32,
    pub job_id: String,
    pub science_id: String,
    pub execution_id: String,
    pub execution_binding_digest: String,
    pub evidence: EvidenceRecord,
    pub snapshot: RegionSnapshot,
    pub coordinate_unit: String,
    pub placement_translation_unit: String,
    pub scope: String,
    pub physical_validation: String,
}

pub fn regions(store: &Store, id: &str) -> Result<RegionReport> {
    let plan = store.recorded_plan(id)?;
    let historical = qualification::inspect(store, id)?;
    let capability = historical
        .capabilities
        .iter()
        .find(|c| {
            matches!(
                c.operation,
                StageOperation::CadInspect
                    | StageOperation::CadFixture
                    | StageOperation::EnclosureFixture
            ) && matches!(c.runtime_execution, EvidenceState::Recorded)
        })
        .ok_or_else(|| invalid("succeeded bound native CAD execution required"))?;
    let binding = historical
        .execution_binding
        .ok_or_else(|| invalid("immutable CAD runner binding required"))?;
    let (evidence, value) = qualification::registered_json(store, id, "regions.json")?
        .ok_or_else(|| invalid("registered CAD regions required"))?;
    let snapshot: RegionSnapshot = serde_json::from_value(value)?;
    snapshot.verify(plan.channel_case()?, capability.operation.clone())?;
    if let Some(spec) = &plan.cad_variant {
        crate::cad_variant::verify_snapshot(spec, &snapshot)?;
    }
    if let Some(spec) = &plan.cad_sheet_variant {
        crate::cad_sheet_variant::verify_snapshot(spec, &snapshot)?;
    }
    Ok(RegionReport {
        schema_version: 1,
        job_id: id.into(),
        science_id: plan.science_id()?,
        execution_id: plan.id()?,
        execution_binding_digest: digest(&binding)?,
        evidence,
        snapshot,
        coordinate_unit: "m".into(),
        placement_translation_unit: "mm".into(),
        scope: "historical named solid regions; no face-number identity or FEM mesh qualification"
            .into(),
        physical_validation: "unqualified".into(),
    })
}
