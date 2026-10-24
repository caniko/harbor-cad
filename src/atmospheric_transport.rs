//! Immutable original-source atmospheric CPU transport approvals and receipt gates.
use crate::{
    Result,
    atmosphere::AtmosphericReferenceSpec,
    atmosphere_transfer::{self, AtmosphericTransferRequest},
    contracts::{
        ExecutionPlan, GpuRequirement, ObservationPlan, Stage, StageOperation, digest, invalid,
    },
    qualification::EvidenceRecord,
    storage::{Store, safe_path},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SANDBOX_POLICY: &str = "harbor-cad-atmospheric-spectral-cpu-v1";
pub const STAGE: &str = "atmospheric-transport";
pub const RECEIPT_PATH: &str = "stages/atmospheric-transport/atmospheric-spectral-receipt.json";

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AtmosphericSourceBinding {
    pub job_id: String,
    pub science_id: String,
    pub execution_id: String,
    pub execution_binding_digest: String,
    pub authorization_digest: String,
    pub receipt: EvidenceRecord,
    pub original: EvidenceRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AtmosphericTransportSpec {
    pub schema_version: u32,
    pub request: AtmosphericTransferRequest,
    pub atmosphere: AtmosphericReferenceSpec,
    pub source: AtmosphericSourceBinding,
}

fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl AtmosphericTransportSpec {
    pub fn validate(&self) -> Result<()> {
        self.request.validate()?;
        let source = self.atmosphere.prepare()?;
        let receiver = self.request.receiver.prepare()?;
        let crate::radiation::SpectralSource::Directional {
            propagation_direction,
            ..
        } = self.request.receiver.source
        else {
            return Err(invalid("explicit atmospheric TOA receiver required"));
        };
        if self.schema_version != 1
            || self.source.job_id != self.request.source_job
            || self.source.science_id != digest(&self.atmosphere)?
            || [
                &self.source.science_id,
                &self.source.execution_id,
                &self.source.execution_binding_digest,
                &self.source.authorization_digest,
                &self.source.original.sha256,
                &self.source.receipt.sha256,
            ]
            .iter()
            .any(|v| !hash(v))
            || self.source.original.path != crate::atmosphere::ORIGINAL_PATH
            || !(1..=32 * 1024 * 1024).contains(&self.source.original.bytes)
            || self.source.receipt.path != "stages/atmosphere/atmosphere-receipt.json"
            || !(1..=256 * 1024).contains(&self.source.receipt.bytes)
            || propagation_direction
                .into_iter()
                .zip(source.propagation_direction)
                .any(|(a, b)| (a - b).abs() > 1e-12)
            || receiver.normalized.wavelengths_m.len() != source.wavelengths_nm.len()
            || receiver
                .normalized
                .wavelengths_m
                .iter()
                .zip(&source.wavelengths_nm)
                .any(|(a, b)| (a * 1e9 - b).abs() > 1e-9)
            || receiver
                .normalized
                .source_values_si
                .iter()
                .zip(&source.toa_irradiance_w_m2_nm)
                .any(|(a, b)| (a * 1e-9 - b).abs() > 1e-12 * b.abs())
        {
            return Err(invalid(
                "complete unchanged authorized atmospheric identities, native knots and original TOA receiver required",
            ));
        }
        Ok(())
    }
    pub fn validate_plan(&self, plan: &ExecutionPlan) -> Result<()> {
        self.validate()?;
        if plan.policy == "ci"
            || plan.stages.len() != 2
            || plan.stages[0].id != STAGE
            || plan.stages[0].operation != StageOperation::AtmosphericTransport
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
                "strict independent registered-source CPU atmospheric transport/bundle DAG; seeds are not physical time",
            ));
        }
        Ok(())
    }
    pub fn native_request(&self) -> Result<serde_json::Value> {
        self.validate()?;
        Ok(
            serde_json::json!({"schema_version":1,"atmosphere":self.atmosphere,"receiver":self.request.receiver,"original_path":"/inputs/atmosphere-original.txt","original_sha256":self.source.original.sha256}),
        )
    }
}

impl ExecutionPlan {
    pub fn atmospheric_transport(spec: AtmosphericTransportSpec, policy: String) -> Result<Self> {
        spec.validate()?;
        let mut plan = Self {
            schema_version: 15,
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
            atmospheric_transport: Some(spec),
            cad_variant: None,
            cad_sheet_variant: None,
            cad_transport: None,
            retained_cooling: None,
            cad_solids: None,
            stages: vec![
                Stage {
                    id: STAGE.into(),
                    dependencies: vec![],
                    operation: StageOperation::AtmosphericTransport,
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

fn resolve(store: &Store, request: AtmosphericTransferRequest) -> Result<AtmosphericTransportSpec> {
    let prepared = atmosphere_transfer::prepare(store, &request)?;
    let source_plan = store.plan(&request.source_job)?;
    let authorization = store
        .execution_authorization(&request.source_job)?
        .ok_or_else(|| {
            crate::Error::Unqualified(
                "registered atmospheric source execution authorization required".into(),
            )
        })?;
    authorization.verify(
        &source_plan,
        &store.job_profile(&request.source_job)?,
        &store.execution_binding(&request.source_job)?,
    )?;
    let spec = AtmosphericTransportSpec {
        schema_version: 1,
        atmosphere: source_plan
            .atmosphere
            .ok_or_else(|| invalid("independent registered atmosphere required"))?,
        source: AtmosphericSourceBinding {
            job_id: request.source_job.clone(),
            science_id: prepared.source_science_id,
            execution_id: prepared.source_execution_id,
            execution_binding_digest: prepared.source_execution_binding_digest,
            authorization_digest: digest(&authorization)?,
            receipt: prepared.original_receipt,
            original: prepared.original_field,
        },
        request,
    };
    spec.validate()?;
    Ok(spec)
}
pub fn plan(
    store: &Store,
    request: AtmosphericTransferRequest,
    policy: String,
) -> Result<ExecutionPlan> {
    ExecutionPlan::atmospheric_transport(resolve(store, request)?, policy)
}

/// Reconstruct original native packets and compare their receipt at approved gates.
/// This numerical check does not attest source execution, immutable registration,
/// sandbox execution, sampling convergence or physical validation.
pub fn verify_original_transport(
    spec: &AtmosphericTransportSpec,
    original: &str,
    root: &std::path::Path,
    receipt: &serde_json::Value,
) -> Result<crate::qualification::NumericalEvidence> {
    verify_receipt(spec, original, root, receipt)
}

/// Independently reduce original native packets and compare the numerical receipt.
/// This permits retained standalone diagnostics lacking production execution
/// attestations. Registration, request-byte hash, sandbox and runtime checks are
/// performed separately by the worker's strict `verify_original_transport` gate.
pub fn reconstruct_transport_receipt(
    spec: &AtmosphericTransportSpec,
    original: &str,
    root: &std::path::Path,
    receipt: &serde_json::Value,
) -> Result<crate::qualification::NumericalEvidence> {
    crate::atmospheric_transport_receipt::verify_numerical(spec, original, root, receipt)
}
pub(crate) fn registered_source(store: &Store, spec: &AtmosphericTransportSpec) -> Result<PathBuf> {
    spec.validate()?;
    let original = resolve(store, spec.request.clone())?;
    if digest(&original)? != digest(spec)? {
        return Err(invalid(
            "approved registered atmospheric source/receipt/authorization identity changed",
        ));
    }
    safe_path(
        &store.job_dir(&spec.source.job_id)?,
        &spec.source.original.path,
    )
}

/// Stage distinct, checksummed source inodes under the submission transaction.
pub(crate) fn retain(store: &Store, id: &str, plan: &ExecutionPlan) -> Result<()> {
    let Some(spec) = &plan.atmospheric_transport else {
        return Ok(());
    };
    let source = registered_source(store, spec)?;
    let root = store.job_dir(id)?;
    let intent = crate::storage::commit_artifact(
        &root,
        "atmospheric-source-retention.json",
        &serde_json::to_vec(spec)?,
        "json",
        "durable registered atmospheric source staging intent; preserve orphan inputs if original source is lost",
    )?;
    for (record, original, destination) in [
        (
            &spec.source.original,
            source,
            "source-atmosphere-original.txt",
        ),
        (
            &spec.source.receipt,
            safe_path(
                &store.job_dir(&spec.source.job_id)?,
                &spec.source.receipt.path,
            )?,
            "source-atmosphere-receipt.json",
        ),
    ] {
        let bytes = crate::worker::read_bounded(&original, record.bytes)?;
        let retained = crate::storage::commit_artifact(
            &root,
            destination,
            &bytes,
            if destination.ends_with(".json") {
                "json"
            } else {
                "txt"
            },
            "unchanged distinct-inode authorized atmospheric original; retained before submission acknowledgment",
        )?;
        if retained.sha256 != record.sha256 || retained.bytes != record.bytes {
            return Err(invalid(
                "registered original atmospheric bytes changed during immutable staging",
            ));
        }
        store.add_artifact(id, &retained)?;
    }
    registered_source(store, spec)?;
    let provenance = serde_json::json!({"source":spec.source,"plan":store.recorded_plan(&spec.source.job_id)?,"host_profile":store.job_profile(&spec.source.job_id)?,"execution_binding":store.execution_binding(&spec.source.job_id)?,"execution_authorization":store.execution_authorization(&spec.source.job_id)?});
    store.add_artifact(id,&crate::storage::commit_artifact(&root,"atmospheric-source-execution.json",&serde_json::to_vec_pretty(&provenance)?,"json","original atmospheric source approvals, runtime identity and independent execution authorization; no upgraded qualification")?)?;
    store.add_artifact(id, &intent)?;
    Ok(())
}

pub(crate) fn recover_orphan(store: &Store, id: &str) -> Result<()> {
    let root = safe_path(&store.root, &format!("artifacts/{id}"))?;
    let intent = safe_path(&root, "atmospheric-source-retention.json")?;
    if !intent.exists() {
        return Ok(());
    }
    let spec: AtmosphericTransportSpec = serde_json::from_slice(&crate::worker::read_bounded(
        &intent,
        crate::contracts::MAX_MESSAGE,
    )?)?;
    if registered_source(store, &spec).is_ok() {
        std::fs::remove_dir_all(&root)?;
        std::fs::File::open(
            root.parent()
                .ok_or_else(|| invalid("atmospheric orphan parent"))?,
        )?
        .sync_all()?;
    }
    Ok(())
}

pub(crate) use crate::atmospheric_transport_receipt::{
    annotate_fields, registered, verify_receipt,
};
