//! Explicit SI synthetic two-state planar penalty-contact reference contract.
use crate::{Result, contracts::invalid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SANDBOX_POLICY: &str = "harbor-cad-contact-cpu-v1";

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContactReferenceSpec {
    pub schema_version: u32,
    pub synthetic: bool,
    pub backend: String,
    pub formulation: String,
    pub size_m: [f64; 3],
    pub resolution: u32,
    pub geometry_tolerance_m: f64,
    pub initial_gap_m: f64,
    pub preload_compression_m: f64,
    pub final_compression_m: f64,
    pub young_modulus_pa: [f64; 2],
    pub expansion_per_k: [f64; 2],
    pub reference_temperature_k: f64,
    pub final_temperatures_k: [f64; 2],
    pub contact_stiffness_pa_m: f64,
    pub numerical_tolerance: f64,
    pub material_provenance: String,
    pub contact_provenance: String,
    pub boundary_provenance: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ContactReferenceState {
    pub solver_step_parameter: u32,
    pub physical_time_s: Option<f64>,
    pub compression_m: f64,
    pub thermal_strains: [f64; 2],
    pub pressure_pa: f64,
    pub gap_m: f64,
    pub model: &'static str,
}

impl ContactReferenceSpec {
    pub fn validate(&self) -> Result<()> {
        let h = self.size_m[2];
        let minimum = self.size_m.iter().copied().fold(f64::INFINITY, f64::min);
        let positive = |v: f64| v.is_finite() && v > 0.;
        if self.schema_version != 1
            || !self.synthetic
            || self.backend != "cpu"
            || self.formulation != "planar_linear_penalty_contact"
            || !(2..=16).contains(&self.resolution)
            || self.size_m.iter().any(|v| !(1e-6..=0.1).contains(v))
            || !self.geometry_tolerance_m.is_finite()
            || self.geometry_tolerance_m < 1e-10
            || self.geometry_tolerance_m >= 1e-3 * minimum
            || !(0. ..=0.01 * h).contains(&self.initial_gap_m)
            || [self.preload_compression_m, self.final_compression_m]
                .iter()
                .any(|v| !(0. ..=0.001 * h).contains(v))
            || self
                .young_modulus_pa
                .iter()
                .any(|v| !(1e4..=1e12).contains(v))
            || self
                .expansion_per_k
                .iter()
                .any(|v| !(0. ..=1e-4).contains(v))
            || !(100. ..=1000.).contains(&self.reference_temperature_k)
            || self
                .final_temperatures_k
                .iter()
                .any(|v| !(100. ..=1000.).contains(v))
            || !positive(self.contact_stiffness_pa_m)
            || self.contact_stiffness_pa_m > 1e16
            || !positive(self.compliance())
            || !positive(self.numerical_tolerance)
            || self.numerical_tolerance > 0.002
            || self
                .expansion_per_k
                .iter()
                .zip(self.final_temperatures_k)
                .any(|(a, t)| (a * (t - self.reference_temperature_k)).abs() > 0.001)
            || [
                &self.material_provenance,
                &self.contact_provenance,
                &self.boundary_provenance,
            ]
            .iter()
            .any(|v| v.trim().is_empty() || v.len() > 4096)
        {
            return Err(invalid(
                "explicit bounded synthetic zero-Poisson CPU planar contact, small strain, SI material/interface/boundary provenance and unchanged 0.002 gate required",
            ));
        }
        Ok(())
    }

    fn compliance(&self) -> f64 {
        self.young_modulus_pa
            .iter()
            .map(|e| self.size_m[2] / e)
            .sum::<f64>()
            + 1. / self.contact_stiffness_pa_m
    }

    pub fn reference(&self, state: u32) -> Result<ContactReferenceState> {
        self.validate()?;
        if !(1..=2).contains(&state) {
            return Err(invalid(
                "explicit preload or final static contact state required",
            ));
        }
        let compression_m = if state == 1 {
            self.preload_compression_m
        } else {
            self.final_compression_m
        };
        let thermal_strains = if state == 1 {
            [0.; 2]
        } else {
            std::array::from_fn(|i| {
                self.expansion_per_k[i]
                    * (self.final_temperatures_k[i] - self.reference_temperature_k)
            })
        };
        let closure = compression_m + self.size_m[2] * thermal_strains.iter().sum::<f64>()
            - self.initial_gap_m;
        let pressure_pa = closure.max(0.) / self.compliance();
        let gap_m = if pressure_pa > 0. {
            -pressure_pa / self.contact_stiffness_pa_m
        } else {
            -closure
        };
        Ok(ContactReferenceState {
            solver_step_parameter: state,
            physical_time_s: None,
            compression_m,
            thermal_strains,
            pressure_pa,
            gap_m,
            model: "p=max(0,compression+sum(alpha*dT*h)-gap)/(sum(h/E)+1/K); Poisson ratio zero",
        })
    }
}

impl crate::contracts::ExecutionPlan {
    pub fn contact_reference(spec: ContactReferenceSpec, policy: String) -> Result<Self> {
        use crate::contracts::*;
        spec.validate()?;
        let mut plan = Self {
            schema_version: 10,
            case: None,
            fem: None,
            thermal: None,
            cad_source: None,
            imported_fem: None,
            wetting: None,
            contact: Some(spec),
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
                    id: "contact".into(),
                    dependencies: vec![],
                    operation: StageOperation::ContactReference,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 2 * 1024 * 1024 * 1024,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec!["contact".into()],
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
                retained_times_s: vec![],
                checkpoint_times_s: vec![],
                preview_times_s: vec![],
                max_artifact_bytes: 128 * 1024 * 1024,
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

pub fn verify_native_outputs(
    spec: &ContactReferenceSpec,
    root: &std::path::Path,
    value: &serde_json::Value,
) -> Result<crate::qualification::NumericalEvidence> {
    use crate::contracts::{digest, invalid};
    use sha2::{Digest, Sha256};
    spec.validate()?;
    if value["schema_version"] != 1
        || value["adapter"] != "CalculiX"
        || value["backend"] != "cpu"
        || value["factorization"] != "SPOOLES"
        || value["precision"] != "float64"
        || value["executed"] != true
        || value["software_fallback"] != false
        || value["synthetic"] != true
        || value["formulation"] != spec.formulation
        || value["request"] != serde_json::to_value(spec)?
        || value["request_sha256"] != digest(spec)?
        || value["calculix_version"] != "2.23"
        || value["gmsh_version"] != "4.15.2"
        || value["calculix_source_sha256"]
            != "9c88385c10fb04f5dc6c4e98027a51bebdd8aee3920e05190d6c1dd08357d6e7"
        || value["physical_validation"] != "unqualified"
        || value["input_serialization"]
            != serde_json::json!({"native_numeric_field_characters":20,"maximum_relative_error":5e-13,"source":"CalculiX 2.23 expansions.f/boundarys.f f20.0","original_approved_si_request_preserved":true})
    {
        return Err(invalid(
            "exact original CPU contact request, solver identity and Float64 provenance required",
        ));
    }
    let checks = [
        "operation_closure_only",
        "no_gpu_nodes",
        "no_sysfs",
        "no_host_home",
        "no_session_bus",
        "no_worker_socket",
        "network_namespace_isolated",
        "descriptor_readonly",
    ];
    if value["sandbox"]["policy"] != SANDBOX_POLICY
        || value["sandbox"]["checks"]
            .as_object()
            .is_none_or(|v| v.len() != checks.len())
        || checks.iter().any(|k| value["sandbox"]["checks"][k] != true)
    {
        return Err(invalid("complete isolated CPU contact sandbox required"));
    }
    let names = [
        "mesh.json",
        "fields.json",
        "reference.inp",
        "reference.dat",
        "lower-reference.msh",
    ];
    let mut outputs = std::collections::BTreeMap::new();
    if value["outputs"]
        .as_object()
        .is_none_or(|v| v.len() != names.len())
    {
        return Err(invalid(
            "exact complete native contact output hashes required",
        ));
    }
    for name in names {
        let data =
            crate::worker::read_bounded(&crate::storage::safe_path(root, name)?, 32 * 1024 * 1024)?;
        if value["outputs"][name] != format!("{:x}", Sha256::digest(&data)) {
            return Err(invalid("original contact native bytes changed"));
        }
        outputs.insert(name, data);
    }
    let mesh = serde_json::from_slice(&outputs["mesh.json"])?;
    let fields: serde_json::Value = serde_json::from_slice(&outputs["fields.json"])?;
    let original = crate::contact_fields::parse_dat(
        std::str::from_utf8(&outputs["reference.dat"])
            .map_err(|_| invalid("native DAT text required"))?,
    )?;
    if original != fields {
        return Err(invalid(
            "contact fields differ from authoritative original DAT",
        ));
    }
    let independent = crate::contact_fields::assess(spec, &mesh, &fields)?;
    let report = value["numerical_verification"]
        .as_array()
        .filter(|v| v.len() == 2)
        .ok_or_else(|| invalid("complete two-state contact checks required"))?;
    let mut maximum: f64 = 0.;
    for (actual, expected) in report.iter().zip(
        independent
            .as_array()
            .ok_or_else(|| invalid("native checks"))?,
    ) {
        if actual["passed"] != true
            || actual["tolerance"].as_f64() != Some(spec.numerical_tolerance)
            || actual["reference"] != expected["reference"]
            || actual["normalized_errors"]
                .as_object()
                .is_none_or(|v| v.len() != 4)
        {
            return Err(invalid("contact static state, units or tolerance changed"));
        }
        for key in ["displacement", "stress", "reaction_force_balance", "gap"] {
            let exact = expected["normalized_errors"][key]
                .as_f64()
                .ok_or_else(|| invalid("native contact error"))?;
            let claimed = actual["normalized_errors"][key]
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| invalid("finite contact error required"))?;
            if (exact - claimed).abs() > 1e-12 || claimed < 0. || claimed > spec.numerical_tolerance
            {
                return Err(invalid(
                    "contact receipt differs from independent original field check",
                ));
            }
            maximum = maximum.max(exact);
        }
    }
    Ok(crate::qualification::NumericalEvidence {reference:"complete original C3D8 DAT displacement/stress/reaction-force balance and geometric opening vs explicit planar series compliance".into(),scope:"synthetic constant-property two-state zero-Poisson planar penalty contact; convergence and physical validation separate".into(),error_kind:"maximum_normalized_max_abs".into(),error:maximum,tolerance:spec.numerical_tolerance})
}

pub(crate) fn annotate_fields(
    plan: &crate::contracts::ExecutionPlan,
    artifacts: &mut [crate::contracts::ArtifactManifest],
) {
    if plan.contact.is_none() && plan.thermal_contact.is_none() {
        return;
    }
    for artifact in artifacts {
        if ["stages/contact/fields.json", "stages/contact/reference.dat"]
            .contains(&artifact.path.as_str())
        {
            artifact.units =
                Some("coordinates m; displacement m; reaction_force N; stress Pa".into());
            artifact.association = Some("native_node_and_integration_point".into());
            artifact.time_s = None;
            artifact.provenance.push_str("; original native C3D8 fields at preload/final solver parameters 1/2; static states have no inferred physical time");
        } else if artifact.path == "stages/contact/mesh.json" {
            artifact.units = Some("m".into());
            artifact.association = Some("native_node_and_c3d8_element".into());
        }
    }
}
