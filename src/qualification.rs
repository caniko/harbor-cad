//! Read-only, source/execution/device-scoped historical evidence, never promotion.
use crate::{
    Error, Result,
    contracts::*,
    execution::ExecutionBinding,
    storage::{Store, safe_path},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceState {
    NotObserved,
    NotAssessed,
    Recorded,
    ReportedPass,
    ReportedFail,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NumericalEvidence {
    pub reference: String,
    pub scope: String,
    pub error_kind: String,
    pub error: f64,
    pub tolerance: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityEvidence {
    pub stage_id: String,
    pub operation: StageOperation,
    pub formulation: String,
    pub dimensions: u8,
    pub precision: Option<String>,
    pub refinement: u32,
    pub backend: String,
    pub requested_device: Option<GpuSelection>,
    pub observed_device: BTreeMap<String, String>,
    pub declared_source_revisions: BTreeMap<String, String>,
    pub evidence: Option<EvidenceRecord>,
    pub runtime_execution: EvidenceState,
    pub numerical_verification: EvidenceState,
    pub numerical_evidence: Option<NumericalEvidence>,
    pub convergence: String,
    pub physical_validation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobEvidenceReport {
    pub schema_version: u32,
    pub scope: String,
    pub job_id: String,
    pub job_state: String,
    pub science_id: String,
    pub execution_id: String,
    pub execution_binding: Option<ExecutionBinding>,
    pub authorization_digest: Option<String>,
    pub capabilities: Vec<CapabilityEvidence>,
    pub current_runtime_qualification: EvidenceState,
    pub physical_validation: String,
}

fn optional_text(value: &Value, key: &str) -> Result<Option<String>> {
    if value.get(key).is_none_or(Value::is_null) {
        return Ok(None);
    }
    let text = value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 1024)
        .ok_or_else(|| invalid("bounded evidence text required"))?;
    Ok(Some(text.into()))
}

pub(crate) fn registered_json(
    store: &Store,
    id: &str,
    path: &str,
) -> Result<Option<(EvidenceRecord, Value)>> {
    let Some(record) = store.artifact_record(id, path)? else {
        return Ok(None);
    };
    if record.format != "json"
        || record.path != path
        || record.bytes == 0
        || record.bytes > 256 * 1024
    {
        return Err(invalid("bounded registered JSON evidence required"));
    }
    let data = crate::worker::read_bounded(&safe_path(&store.job_dir(id)?, path)?, 256 * 1024)?;
    if data.len() as u64 != record.bytes || format!("{:x}", Sha256::digest(&data)) != record.sha256
    {
        return Err(invalid("historical evidence differs from registered bytes"));
    }
    let value: Value = serde_json::from_slice(&data)?;
    if !value.is_object() {
        return Err(invalid("evidence object required"));
    }
    Ok(Some((
        EvidenceRecord {
            path: path.into(),
            sha256: record.sha256,
            bytes: record.bytes,
        },
        value,
    )))
}

fn numerical(
    value: &Value,
    plan: &ExecutionPlan,
    operation: &StageOperation,
) -> Result<(EvidenceState, Option<NumericalEvidence>)> {
    let (body, error_key, tolerance, reference, scope, error_kind, passed) = match operation {
        StageOperation::ChannelReference => (
            value,
            "numerical_error",
            plan.channel_case()?.applicability.numerical_tolerance,
            "analytical channel residual".into(),
            "synthetic reference only".into(),
            "relative_residual",
            None,
        ),
        StageOperation::Openlb if value["numerical_verification"].is_object() => {
            let body = &value["numerical_verification"];
            let tolerance = body["tolerance"]
                .as_f64()
                .ok_or_else(|| invalid("numerical tolerance missing"))?;
            if tolerance != plan.channel_case()?.applicability.numerical_tolerance {
                return Err(invalid(
                    "receipt weakened or changed the approved numerical tolerance",
                ));
            }
            (
                body,
                "relative_l2_error",
                tolerance,
                optional_text(body, "reference")?
                    .ok_or_else(|| invalid("numerical reference missing"))?,
                optional_text(body, "scope")?.ok_or_else(|| invalid("numerical scope missing"))?,
                "relative_l2",
                Some(
                    body["passed"]
                        .as_bool()
                        .ok_or_else(|| invalid("numerical status missing"))?,
                ),
            )
        }
        StageOperation::FemReference => {
            return Ok((
                EvidenceState::ReportedPass,
                Some(crate::fem::verify_receipt(plan, value)?),
            ));
        }
        StageOperation::ThermalReference => {
            return Ok((
                EvidenceState::ReportedPass,
                Some(crate::thermal::verify_receipt(plan, value)?),
            ));
        }
        _ => return Ok((EvidenceState::NotAssessed, None)),
    };
    let error = body[error_key]
        .as_f64()
        .filter(|x| x.is_finite() && *x >= 0.)
        .ok_or_else(|| invalid("finite nonnegative numerical error required"))?;
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err(invalid("positive finite numerical tolerance required"));
    }
    let pass = passed.unwrap_or(error <= tolerance);
    if pass && error > tolerance {
        return Err(invalid("numerical pass exceeds approved tolerance"));
    }
    Ok((
        if pass {
            EvidenceState::ReportedPass
        } else {
            EvidenceState::ReportedFail
        },
        Some(NumericalEvidence {
            reference,
            scope,
            error_kind: error_kind.into(),
            error,
            tolerance,
        }),
    ))
}

pub fn inspect(store: &Store, id: &str) -> Result<JobEvidenceReport> {
    let job = store.job(id)?;
    // Current validation rules may reject an old formulation; inspecting its
    // immutable recorded evidence must neither rewrite nor relaunch that job.
    let plan = store.recorded_plan(id)?;
    let binding = match store.execution_binding(id) {
        Ok(binding) => {
            if binding.plan_digest != job.plan_digest {
                return Err(invalid("historical binding/plan mismatch"));
            }
            Some(binding)
        }
        Err(Error::Unqualified(_)) => None,
        Err(error) => return Err(error),
    };
    let authorization_digest = store
        .execution_authorization(id)?
        .as_ref()
        .map(digest)
        .transpose()?;
    let mut capabilities = Vec::new();
    for stage in &plan.stages {
        let thermal_path = format!("stages/{}/thermal-receipt.json", stage.id);
        let (path, adapter) = match stage.operation {
            StageOperation::ChannelReference => ("validation.json", None),
            StageOperation::CadFixture => ("cad_fixture-receipt.json", Some("FreeCAD")),
            StageOperation::CadInspect => ("cad_inspect-receipt.json", Some("FreeCAD")),
            StageOperation::EnclosureFixture => ("enclosure_fixture-receipt.json", Some("FreeCAD")),
            StageOperation::CadMesh => ("stages/mesh/cad-mesh-receipt.json", Some("Gmsh")),
            StageOperation::FemImported => (
                "stages/fem-imported/fem-imported-receipt.json",
                Some("CalculiX"),
            ),
            StageOperation::Openlb => ("openlb-receipt.json", Some("OpenLB")),
            StageOperation::NumericalFilter => (
                "stages/filter/numerical_filter-receipt.json",
                Some("Viskores"),
            ),
            StageOperation::Render => ("render-receipt.json", Some("ParaView")),
            StageOperation::Video => ("video-receipt.json", Some("FFmpeg")),
            StageOperation::FemReference => {
                ("stages/fem/fem-reference-receipt.json", Some("CalculiX"))
            }
            StageOperation::ThermalReference => (
                if plan.thermal_contact.is_some() {
                    thermal_path.as_str()
                } else {
                    "stages/thermal/thermal-receipt.json"
                },
                Some("CalculiX"),
            ),
            StageOperation::WettingReference => (
                "stages/wetting/verified-wetting-receipt.json",
                Some("OpenLB"),
            ),
            StageOperation::FreezingReference if plan.retained_cooling.is_some() => {
                (crate::cooling_execution::RECEIPT_PATH, Some("OpenLB"))
            }
            StageOperation::FreezingReference => {
                ("stages/freezing/freezing-receipt.json", Some("OpenLB"))
            }
            StageOperation::SpectralReference if plan.cad_transport.is_some() => {
                (crate::cad_transport::RECEIPT_PATH, Some("Mitsuba"))
            }
            StageOperation::SpectralReference => {
                ("stages/spectral/spectral-receipt.json", Some("Mitsuba"))
            }
            StageOperation::AtmosphericReference => (
                "stages/atmosphere/atmosphere-receipt.json",
                Some("libRadtran"),
            ),
            StageOperation::AtmosphericTransport => {
                (crate::atmospheric_transport::RECEIPT_PATH, Some("Mitsuba"))
            }
            StageOperation::ContactReference => {
                ("stages/contact/contact-receipt.json", Some("CalculiX"))
            }
            StageOperation::ThermalProjection => {
                ("stages/projection/projection-receipt.json", None)
            }
            StageOperation::Bundle => continue,
        };
        let loaded = registered_json(store, id, path)?;
        let operation_key = serde_json::to_string(&stage.operation)?;
        let native_bound = binding.as_ref().is_some_and(|b| {
            adapter.is_none()
                || (b.native_runtime.is_some() && b.native_files.contains_key(&operation_key))
        });
        let mut capability = CapabilityEvidence {
            stage_id: stage.id.clone(),
            operation: stage.operation.clone(),
            formulation: if plan.retained_cooling.is_some() {
                "stationary_equal_property_retained_phase_conduction".into()
            } else if let Some(spec) = &plan.cad_transport {
                spec.request.formulation.clone()
            } else if plan.atmospheric_transport.is_some() {
                "registered_full_sphere_original_midpoint_planar_uv_transport".into()
            } else if let Some(spec) = &plan.atmosphere {
                spec.model.clone()
            } else if plan.spectral.is_some() {
                "directional_planar_uv_irradiance_and_prescribed_dose".into()
            } else if let Some(spec) = &plan.thermal_contact {
                match stage.operation {
                    StageOperation::ThermalReference => {
                        spec.thermal_stage(&stage.id)?.formulation.clone()
                    }
                    StageOperation::ContactReference => "planar_linear_penalty_contact".into(),
                    StageOperation::ThermalProjection => {
                        "congruent_constant_capacitance_box_projection".into()
                    }
                    _ => return Err(invalid("approved coupling stage required")),
                }
            } else if let Some(spec) = &plan.contact {
                spec.formulation.clone()
            } else if let Some(spec) = &plan.wetting {
                spec.formulation.clone()
            } else if let Some(spec) = &plan.freezing {
                spec.formulation.clone()
            } else if let Some(spec) = &plan.imported_fem {
                spec.formulation()
            } else if let Some(source) = &plan.cad_source {
                source.geometry.formulation.clone()
            } else {
                plan.thermal.as_ref().map_or_else(
                    || {
                        plan.fem.as_ref().map_or_else(
                            || {
                                plan.channel_case()
                                    .map(|c| c.applicability.formulation.clone())
                            },
                            |f| Ok(f.formulation().into()),
                        )
                    },
                    |t| Ok(t.formulation.clone()),
                )?
            },
            dimensions: if plan.atmosphere.is_some() {
                1
            } else if plan.wetting.is_some()
                || plan.freezing.is_some()
                || plan.retained_cooling.is_some()
            {
                2
            } else {
                plan.case
                    .as_ref()
                    .map_or(3, |c| c.applicability.dimensionality)
            },
            precision: None,
            refinement: if let Some(spec) = &plan.retained_cooling {
                spec.request.spatial_refinement
            } else if let Some(spec) = &plan.atmospheric_transport {
                spec.request.receiver.samples
            } else if let Some(spec) = &plan.atmosphere {
                spec.streams
            } else if let Some(spec) = &plan.cad_transport {
                spec.request.samples_per_triangle
            } else if let Some(spec) = &plan.spectral {
                spec.samples
            } else if let Some(spec) = &plan.thermal_contact {
                if stage.operation == StageOperation::ThermalReference {
                    spec.thermal_stage(&stage.id)?.resolution
                } else {
                    spec.mechanical.resolution
                }
            } else if let Some(spec) = &plan.contact {
                spec.resolution
            } else if let Some(spec) = &plan.wetting {
                spec.resolution
            } else if let Some(spec) = &plan.freezing {
                spec.resolution
            } else if let Some(source) = &plan.cad_source {
                source.geometry.resolution
            } else {
                plan.thermal.as_ref().map_or_else(
                    || {
                        plan.fem.as_ref().map_or_else(
                            || plan.channel_case().map(|c| c.resolution),
                            |f| Ok(f.resolution),
                        )
                    },
                    |t| Ok(t.resolution),
                )?
            },
            backend: stage
                .selection
                .as_ref()
                .map_or("cpu".into(), |s| s.backend.clone()),
            requested_device: stage.selection.clone(),
            observed_device: BTreeMap::new(),
            declared_source_revisions: BTreeMap::new(),
            evidence: None,
            runtime_execution: EvidenceState::NotObserved,
            numerical_verification: EvidenceState::NotAssessed,
            numerical_evidence: None,
            convergence: "not_assessed".into(),
            physical_validation: "unqualified".into(),
        };
        if let Some((record, value)) = loaded {
            capability.evidence = Some(record);
            capability.precision = if adapter.is_none() {
                Some("float64".into())
            } else {
                optional_text(&value, "precision")?
            };
            capability.convergence =
                optional_text(&value, "convergence")?.unwrap_or_else(|| "not_assessed".into());
            for key in [
                "source_revision",
                "viskores_revision",
                "kokkos_revision",
                "gmsh_source_sha256",
                "calculix_source_sha256",
                "mitsuba_version",
                "drjit_version",
                "source_sha256",
                "profile_sha256",
            ] {
                if let Some(text) = optional_text(&value, key)? {
                    capability
                        .declared_source_revisions
                        .insert(key.into(), text);
                }
            }
            for key in [
                "pci",
                "backend_uuid",
                "architecture",
                "compiled_architecture",
            ] {
                if let Some(text) = optional_text(&value, key)? {
                    capability.observed_device.insert(key.into(), text);
                }
            }
            let executed = if let Some(adapter) = adapter {
                value["adapter"] == adapter
                    && value["backend"] == capability.backend
                    && value["executed"] == true
                    && value["software_fallback"] == false
            } else {
                value["process"] == "succeeded"
            };
            if executed && native_bound && job.state == "succeeded" && job.exit_code == Some(0) {
                if let Some(selection) = &stage.selection
                    && (value["pci"] != selection.pci
                        || selection
                            .backend_uuid
                            .as_ref()
                            .is_some_and(|uuid| value["backend_uuid"] != *uuid))
                {
                    return Err(invalid(
                        "historical receipt differs from the approved device",
                    ));
                }
                capability.runtime_execution = EvidenceState::Recorded;
                if plan.cad_variant.is_some() && stage.operation == StageOperation::CadInspect {
                    crate::cad_variant::verify_registered_outputs(store, id, &plan, &value)?;
                    capability.formulation = "source_bound_controlled_box_variant".into();
                }
                if plan.cad_sheet_variant.is_some() && stage.operation == StageOperation::CadInspect
                {
                    crate::cad_sheet_variant::verify_registered_outputs(store, id, &plan, &value)?;
                    capability.formulation = "source_bound_spreadsheet_variant".into();
                }
                if matches!(stage.operation, StageOperation::FemImported) {
                    for path in [
                        "stages/fem-imported/mesh.json",
                        "stages/fem-imported/reference.dat",
                    ] {
                        let record = store
                            .artifact_record(id, path)?
                            .ok_or_else(|| invalid("registered imported FEM output missing"))?;
                        let observed = crate::storage::native_manifest(
                            &store.job_dir(id)?,
                            path,
                            32 * 1024 * 1024,
                            "verify historical imported FEM output",
                        )?;
                        if record.sha256 != observed.sha256 || record.bytes != observed.bytes {
                            return Err(invalid("historical imported FEM output bytes changed"));
                        }
                    }
                    capability.numerical_evidence = Some(crate::fem_imported::verify_receipt(
                        &plan,
                        &store.job_dir(id)?.join("stages/fem-imported"),
                        &value,
                    )?);
                    capability.numerical_verification = EvidenceState::ReportedPass;
                } else if matches!(stage.operation, StageOperation::CadMesh) {
                    let record = store
                        .artifact_record(id, "stages/mesh/mesh.json")?
                        .ok_or_else(|| invalid("registered imported mesh evidence missing"))?;
                    let observed = crate::storage::native_manifest(
                        &store.job_dir(id)?,
                        &record.path,
                        32 * 1024 * 1024,
                        "historical bounded imported mesh byte verification",
                    )?;
                    if record.format != "json"
                        || record.sha256 != observed.sha256
                        || record.bytes != observed.bytes
                    {
                        return Err(invalid(
                            "historical imported mesh differs from its registered bytes",
                        ));
                    }
                    capability.numerical_evidence = Some(crate::cad_mesh::verify_receipt(
                        &plan,
                        &store.job_dir(id)?.join("stages/mesh"),
                        &value,
                    )?);
                    capability.numerical_verification = EvidenceState::ReportedPass;
                    capability.convergence =
                        "geometric correspondence; no field solve or convergence claim".into();
                } else if matches!(stage.operation, StageOperation::ContactReference) {
                    let root = store.job_dir(id)?.join("stages/contact");
                    for name in [
                        "mesh.json",
                        "fields.json",
                        "reference.inp",
                        "reference.dat",
                        "lower-reference.msh",
                    ] {
                        let path = format!("stages/contact/{name}");
                        let record = store
                            .artifact_record(id, &path)?
                            .ok_or_else(|| invalid("registered contact output absent"))?;
                        let observed = crate::storage::native_manifest(
                            &store.job_dir(id)?,
                            &path,
                            32 * 1024 * 1024,
                            "verify historical original contact output",
                        )?;
                        if record.bytes == 0
                            || record.bytes > 32 * 1024 * 1024
                            || record.sha256 != observed.sha256
                            || record.bytes != observed.bytes
                        {
                            return Err(invalid("registered original contact bytes changed"));
                        }
                    }
                    let coupled = if plan.thermal_contact.is_some() {
                        Some(crate::thermal_contact::verify_registered(store, id, &plan)?.0)
                    } else {
                        None
                    };
                    capability.numerical_evidence = Some(crate::contact::verify_native_outputs(
                        coupled
                            .as_ref()
                            .or(plan.contact.as_ref())
                            .ok_or_else(|| invalid("native contact recipe required"))?,
                        &root,
                        &value,
                    )?);
                    capability.numerical_verification = EvidenceState::ReportedPass;
                    capability.convergence="one declared mesh; native planar spatial-reference campaign assessed separately".into();
                } else if stage.operation == StageOperation::ThermalProjection {
                    capability.numerical_evidence =
                        Some(crate::thermal_contact::verify_registered(store, id, &plan)?.1);
                    capability.numerical_verification = EvidenceState::ReportedPass;
                } else if stage.operation == StageOperation::ThermalReference {
                    let spec = if let Some(coupling) = &plan.thermal_contact {
                        coupling.thermal_stage(&stage.id)?
                    } else {
                        plan.thermal
                            .as_ref()
                            .ok_or_else(|| invalid("native thermal recipe required"))?
                    };
                    capability.numerical_evidence =
                        Some(crate::thermal_results::verify_registered_originals(
                            store,
                            id,
                            spec,
                            &format!("stages/{}", stage.id),
                            &value,
                        )?);
                    capability.numerical_verification = EvidenceState::ReportedPass;
                } else if matches!(stage.operation, StageOperation::WettingReference) {
                    let fields = value["independent_fields"]
                        .as_array()
                        .ok_or_else(|| invalid("registered native wetting fields required"))?;
                    let prefix = "stages/wetting";
                    let spec = plan
                        .wetting
                        .as_ref()
                        .ok_or_else(|| invalid("wetting recipe required"))?;
                    for (field, step) in fields.iter().zip(&spec.observation_steps) {
                        let name = field["path"]
                            .as_str()
                            .ok_or_else(|| invalid("native wetting path required"))?;
                        let path = format!("{prefix}/{name}");
                        let record = store
                            .artifact_record(id, &path)?
                            .ok_or_else(|| invalid("registered wetting field absent"))?;
                        if record.path != path
                            || record.bytes == 0
                            || record.bytes > 16 * 1024 * 1024
                            || record.format != "csv"
                            || record.time_s != Some(*step as f64 * spec.physical_step_s())
                            || record.association.as_deref() != Some("native_lattice_point")
                            || record.units.as_deref()
                                != Some("x_m:m,y_m:m,material:1,phi:1,u_lattice:1,v_lattice:1")
                        {
                            return Err(invalid("registered bounded wetting CSV required"));
                        }
                        let observed = crate::storage::native_manifest(
                            &store.job_dir(id)?,
                            &path,
                            16 * 1024 * 1024,
                            "verify historical original wetting field",
                        )?;
                        if observed.bytes != record.bytes
                            || observed.sha256 != record.sha256
                            || field["sha256"] != record.sha256
                        {
                            return Err(invalid("registered native wetting identity changed"));
                        }
                    }
                    let root = safe_path(&store.job_dir(id)?, prefix)?;
                    capability.numerical_evidence =
                        Some(crate::wetting::verify_receipt(&plan, &root, &value)?);
                    capability.numerical_verification = EvidenceState::ReportedPass;
                } else if stage.operation == StageOperation::FreezingReference
                    && plan.retained_cooling.is_some()
                {
                    capability.numerical_evidence =
                        Some(crate::cooling_fields::registered(store, id, &plan, &value)?);
                    capability.numerical_verification = EvidenceState::ReportedPass;
                    capability.convergence="one independently approved original-parent grid and temporal substeps; uniform analytic and complete-history spatial/temporal gates assessed separately".into();
                } else if stage.operation == StageOperation::FreezingReference {
                    capability.numerical_evidence = Some(crate::freezing::verify_registered(
                        store, id, &plan, &value,
                    )?);
                    capability.numerical_verification = EvidenceState::ReportedPass;
                    capability.convergence = "one declared grid; equal-physical-time native spatial refinement assessed separately".into();
                } else if stage.operation == StageOperation::SpectralReference
                    && plan.cad_transport.is_some()
                {
                    capability.numerical_evidence = Some(
                        crate::cad_transport_fields::verify_registered(store, id, &plan, &value)?,
                    );
                    capability.numerical_verification = EvidenceState::ReportedPass;
                    capability.convergence = "three declared seeds; original-point verification; sampling convergence not_assessed".into();
                } else if stage.operation == StageOperation::SpectralReference {
                    capability.numerical_evidence = Some(crate::radiation::verify_registered(
                        store, id, &plan, &value,
                    )?);
                    capability.numerical_verification = EvidenceState::ReportedPass;
                    capability.convergence="three retained directional seeds; exact original-knot quadrature; hemispherical sampling refinement assessed separately".into();
                } else if stage.operation == StageOperation::AtmosphericReference {
                    capability.numerical_evidence =
                        Some(crate::atmosphere::registered(store, id, &plan, &value)?);
                    capability.numerical_verification = EvidenceState::ReportedPass;
                    capability.convergence="one declared DISORT/angle/wavelength grid; separate stream/angular-shape/spectral refinement not_assessed".into();
                } else if stage.operation == StageOperation::AtmosphericTransport {
                    capability.numerical_evidence = Some(crate::atmospheric_transport::registered(
                        store, id, &plan, &value,
                    )?);
                    capability.numerical_verification = EvidenceState::ReportedPass;
                    capability.convergence="three declared native transport seeds; original source refinement and sampling convergence assessed separately".into();
                } else {
                    (
                        capability.numerical_verification,
                        capability.numerical_evidence,
                    ) = numerical(&value, &plan, &stage.operation)?;
                }
            }
        }
        capabilities.push(capability);
    }
    Ok(JobEvidenceReport {
        schema_version: 1,
        scope: "historical_job_evidence".into(),
        job_id: job.id,
        job_state: job.state,
        science_id: plan.science_id()?,
        execution_id: job.plan_digest,
        execution_binding: binding,
        authorization_digest,
        capabilities,
        current_runtime_qualification: EvidenceState::NotAssessed,
        physical_validation: "unqualified".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scientific_pass_cannot_hide_exceeded_changed_missing_or_nonfinite_gates() {
        let mut case = CaseSpec::reference();
        case.length.value = 0.02;
        case.resolution = 8;
        case.acceleration.value = 0.001;
        case.max_time_s = 20.;
        case.applicability.formulation = "periodic_forced_channel".into();
        let plan = ExecutionPlan::openlb_reference(case, "research".into()).unwrap();
        let tolerance = plan
            .channel_case()
            .unwrap()
            .applicability
            .numerical_tolerance;
        let good = serde_json::json!({"numerical_verification":{
            "passed":true,"relative_l2_error":tolerance/2.,"tolerance":tolerance,
            "reference":"analytical parallel plate velocity","scope":"velocity only; pressure unqualified"}});
        let (state, evidence) = numerical(&good, &plan, &StageOperation::Openlb).unwrap();
        assert!(matches!(state, EvidenceState::ReportedPass));
        assert!(evidence.unwrap().scope.contains("pressure unqualified"));
        for (key, value) in [
            ("relative_l2_error", serde_json::json!(tolerance * 2.)),
            ("relative_l2_error", serde_json::json!(-1.)),
            ("relative_l2_error", Value::Null),
            ("tolerance", serde_json::json!(tolerance * 2.)),
            ("reference", Value::Null),
            ("scope", Value::Null),
        ] {
            let mut changed = good.clone();
            changed["numerical_verification"][key] = value;
            assert!(numerical(&changed, &plan, &StageOperation::Openlb).is_err());
        }
        let mut failed = good;
        failed["numerical_verification"]["relative_l2_error"] = serde_json::json!(tolerance * 2.);
        failed["numerical_verification"]["passed"] = serde_json::json!(false);
        assert!(matches!(
            numerical(&failed, &plan, &StageOperation::Openlb)
                .unwrap()
                .0,
            EvidenceState::ReportedFail
        ));
        assert!(matches!(
            numerical(&failed, &plan, &StageOperation::NumericalFilter)
                .unwrap()
                .0,
            EvidenceState::NotAssessed
        ));
    }
}
