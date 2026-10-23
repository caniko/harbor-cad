//! Explicit SI synthetic CPU FEM recipes, independent of fluid case envelopes.
use crate::{Result, contracts::*};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FemMode {
    ThermalBoundary,
    FreeExpansion,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FemReferenceSpec {
    pub schema_version: u32,
    pub synthetic: bool,
    pub backend: String,
    pub mode: FemMode,
    pub size_m: [f64; 3],
    pub resolution: u32,
    pub geometry_tolerance_m: f64,
    pub temperatures_k: [f64; 2],
    pub numerical_tolerance: f64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_number"
    )]
    #[schemars(with = "f64")]
    pub conductivity_w_m_k: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_number"
    )]
    #[schemars(with = "f64")]
    pub young_modulus_pa: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_number"
    )]
    #[schemars(with = "f64")]
    pub poisson_ratio: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_number"
    )]
    #[schemars(with = "f64")]
    pub expansion_per_k: Option<f64>,
}
fn optional_number<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<f64>, D::Error> {
    f64::deserialize(decoder).map(Some)
}
impl FemReferenceSpec {
    pub fn validate(&self) -> Result<()> {
        let positive = |x: f64| x.is_finite() && x > 0.;
        let minimum = self.size_m.iter().copied().fold(f64::INFINITY, f64::min);
        let maximum = self.size_m.iter().copied().fold(0., f64::max);
        if self.schema_version != 1
            || !self.synthetic
            || self.backend != "cpu"
            || !self.size_m.iter().copied().all(positive)
            || maximum / minimum > 1000.
            || !(2..=32).contains(&self.resolution)
            || !positive(self.numerical_tolerance)
            || self.numerical_tolerance > 1e-6
            || !self.geometry_tolerance_m.is_finite()
            || self.geometry_tolerance_m < 1e-10
            || self.geometry_tolerance_m >= 0.001 * minimum
            || !self.temperatures_k.iter().copied().all(positive)
            || self.temperatures_k[0] == self.temperatures_k[1]
        {
            return Err(invalid(
                "explicit synthetic CPU FEM, finite SI inputs, resolved geometry and unchanged numerical gate required",
            ));
        }
        match self.mode {
            FemMode::ThermalBoundary
                if self.conductivity_w_m_k.is_some_and(positive)
                    && self.young_modulus_pa.is_none()
                    && self.poisson_ratio.is_none()
                    && self.expansion_per_k.is_none() => {}
            FemMode::FreeExpansion
                if self.conductivity_w_m_k.is_none()
                    && self.young_modulus_pa.is_some_and(positive)
                    && self
                        .poisson_ratio
                        .is_some_and(|p| p.is_finite() && p > -1. && p < 0.5)
                    && self.expansion_per_k.is_some_and(|a| {
                        a.is_finite()
                            && a != 0.
                            && (a * (self.temperatures_k[1] - self.temperatures_k[0])).abs() <= 0.01
                    }) => {}
            _ => {
                return Err(invalid(
                    "mode-specific stable material and small-strain applicability required",
                ));
            }
        }
        Ok(())
    }
    pub fn formulation(&self) -> &str {
        match self.mode {
            FemMode::ThermalBoundary => "thermal_boundary",
            FemMode::FreeExpansion => "free_expansion",
        }
    }
}

pub fn verify_receipt(
    plan: &ExecutionPlan,
    value: &serde_json::Value,
) -> Result<crate::qualification::NumericalEvidence> {
    let spec = plan
        .fem
        .as_ref()
        .ok_or_else(|| invalid("approved FEM recipe required"))?;
    verify_recipe_receipt(
        spec,
        value,
        &digest(spec)?,
        spec.formulation(),
        crate::execution::FEM_SANDBOX_POLICY,
        &[],
    )
}

pub(crate) fn verify_recipe_receipt(
    spec: &FemReferenceSpec,
    value: &serde_json::Value,
    request_digest: &str,
    formulation: &str,
    policy: &str,
    extra_checks: &[&str],
) -> Result<crate::qualification::NumericalEvidence> {
    spec.validate()?;
    let n = u64::from(spec.resolution);
    if value["schema_version"] != 1
        || value["adapter"] != "CalculiX"
        || value["backend"] != "cpu"
        || value["factorization"] != "SPOOLES"
        || value["executed"] != true
        || value["software_fallback"] != false
        || value["synthetic"] != true
        || value["precision"] != "float64"
        || value["request_sha256"] != request_digest
        || value["formulation"] != formulation
        || value["calculix_version"] != "2.23"
        || value["gmsh_version"] != "4.15.2"
        || value["gmsh_source_sha256"]
            != "be3f66f225d27ba9fa014f07e83169285da8a051b0e8ab7103d88066b39bdd3e"
        || value["calculix_source_sha256"]
            != "9c88385c10fb04f5dc6c4e98027a51bebdd8aee3920e05190d6c1dd08357d6e7"
        || value["nodes"] != (n + 1).pow(3)
        || value["elements"] != n.pow(3)
        || value["physical_validation"] != "unqualified"
    {
        return Err(invalid(
            "FEM execution/recipe/source identity changed; no numerical promotion",
        ));
    }
    let sandbox = &value["sandbox"];
    let names = [
        "operation_closure_only",
        "no_gpu_nodes",
        "no_sysfs",
        "no_host_home",
        "no_session_bus",
        "no_worker_socket",
        "network_namespace_isolated",
        "descriptor_readonly",
    ];
    if sandbox["policy"] != policy
        || sandbox["checks"]
            .as_object()
            .is_none_or(|checks| checks.len() != names.len() + extra_checks.len())
        || names
            .iter()
            .chain(extra_checks)
            .any(|name| sandbox["checks"][name] != true)
    {
        return Err(invalid(
            "complete operation-specific CPU FEM sandbox evidence required",
        ));
    }
    let fields = match spec.mode {
        FemMode::ThermalBoundary => [
            ("temperature", "K", (n + 1).pow(3)),
            ("heat_flux", "W/m2", 8 * n.pow(3)),
        ],
        FemMode::FreeExpansion => [
            ("displacement", "m", (n + 1).pow(3)),
            ("stress", "Pa", 8 * n.pow(3)),
        ],
    };
    let checks = value["numerical_verification"]
        .as_object()
        .ok_or_else(|| invalid("complete native FEM checks required"))?;
    if checks.len() != fields.len() {
        return Err(invalid("unexpected or missing native FEM numerical check"));
    }
    let mut error: f64 = 0.;
    let mut references = Vec::new();
    for (field, unit, samples) in fields {
        let check = checks
            .get(field)
            .ok_or_else(|| invalid("native FEM field check missing"))?;
        let observed = check["normalized_max_abs_error"]
            .as_f64()
            .filter(|e| e.is_finite() && *e >= 0. && *e <= spec.numerical_tolerance)
            .ok_or_else(|| invalid("native FEM analytical error exceeds approved gate"))?;
        let reference = check["reference"]
            .as_str()
            .filter(|r| !r.is_empty() && r.len() <= 1024)
            .ok_or_else(|| invalid("analytical FEM reference missing"))?;
        if check["passed"] != true
            || check["tolerance"].as_f64() != Some(spec.numerical_tolerance)
            || check["unit"] != unit
            || check["samples"] != samples
        {
            return Err(invalid(
                "native FEM check units, coverage or tolerance changed",
            ));
        }
        references.push(format!("{field}: {reference}"));
        error = error.max(observed);
    }
    Ok(crate::qualification::NumericalEvidence {
        reference: references.join("; "),
        scope: "synthetic static FEM; maximum of separately normalized field errors".into(),
        error_kind: "maximum_normalized_max_abs".into(),
        error,
        tolerance: spec.numerical_tolerance,
    })
}

impl ExecutionPlan {
    pub fn fem_reference(spec: FemReferenceSpec, policy: String) -> Result<Self> {
        spec.validate()?;
        let mut plan = Self {
            schema_version: 5,
            case: None,
            fem: Some(spec),
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
            retained_cooling: None,
            cad_solids: None,
            source: None,
            frames: None,
            filter: None,
            stages: vec![
                Stage {
                    id: "fem".into(),
                    dependencies: vec![],
                    operation: StageOperation::FemReference,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 2 * 1024 * 1024 * 1024,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec!["fem".into()],
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
                max_artifact_bytes: 64 * 1024 * 1024,
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
