//! Explicit synthetic planar diffuse-interface wetting, independent of airflow.
use crate::{Error, Result, contracts::*};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WettingReferenceSpec {
    pub schema_version: u32,
    pub synthetic: bool,
    pub backend: String,
    pub formulation: String,
    pub diameter_m: f64,
    pub initial_center_above_wall_m: f64,
    pub resolution: u32,
    pub interface_width_m: f64,
    pub density_liquid_kg_m3: f64,
    pub density_vapor_kg_m3: f64,
    pub viscosity_liquid_m2_s: f64,
    pub viscosity_vapor_m2_s: f64,
    pub surface_tension_n_m: f64,
    pub contact_angle_deg: f64,
    pub phase_relaxation_time: f64,
    pub steps: u64,
    pub observation_steps: Vec<u64>,
    pub mass_tolerance: f64,
    pub angle_tolerance_deg: f64,
    pub material_provenance: String,
    pub boundary_provenance: String,
    /// Explicit HIP compute card (PCI + UUID) for `backend = "hip"`; absent for
    /// CPU so historical CPU descriptors and digests are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<GpuSelection>,
}

/// HIP wetting sees exactly its selected render node, `/dev/kfd` and selected
/// KFD/DRM sysfs metadata under the single-KFD-GPU policy; nothing else changes.
pub const HIP_SANDBOX_POLICY: &str = "harbor-cad-wetting-hip-single-kfd-v1";

impl ExecutionPlan {
    pub fn wetting_reference(spec: WettingReferenceSpec, policy: String) -> Result<Self> {
        spec.validate()?;
        let retained_times_s = spec.times_s();
        let (gpu, selection) = match &spec.selection {
            Some(selection) => (GpuRequirement::Required, Some(selection.clone())),
            None => (GpuRequirement::CpuOnly, None),
        };
        let mut plan = Self {
            schema_version: 9,
            case: None,
            fem: None,
            thermal: None,
            cad_source: None,
            imported_fem: None,
            wetting: Some(spec),
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
                    id: "wetting".into(),
                    dependencies: vec![],
                    operation: StageOperation::WettingReference,
                    gpu,
                    selection,
                    ram_bytes: 0,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec!["wetting".into()],
                    operation: StageOperation::Bundle,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 0,
                    vram_bytes: 0,
                },
            ],
            transfers: vec![],
            observation: ObservationPlan {
                metrics: vec![],
                probes: vec![],
                retained_times_s,
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

impl WettingReferenceSpec {
    pub fn spacing_m(&self) -> f64 {
        self.diameter_m / f64::from(self.resolution)
    }
    pub fn physical_step_s(&self) -> f64 {
        0.5 / 3. * self.spacing_m().powi(2) / self.viscosity_liquid_m2_s
    }
    pub fn surface_tension_lattice(&self) -> f64 {
        self.surface_tension_n_m * self.physical_step_s().powi(2)
            / (self.density_liquid_kg_m3 * self.spacing_m().powi(3))
    }
    pub fn times_s(&self) -> Vec<f64> {
        self.observation_steps
            .iter()
            .map(|step| *step as f64 * self.physical_step_s())
            .collect()
    }
    pub fn sandbox_policy(&self) -> &'static str {
        if self.backend == "hip" {
            HIP_SANDBOX_POLICY
        } else {
            crate::execution::WETTING_SANDBOX_POLICY
        }
    }
    pub fn validate(&self) -> Result<()> {
        let positive = |v: f64| v.is_finite() && v > 0.;
        // HIP needs the exact compute card; CPU never carries a device. CUDA and
        // other backends have no wetting driver device identity: no fallback.
        let device = match (self.backend.as_str(), &self.selection) {
            ("cpu", None) => true,
            ("hip", Some(s)) => {
                s.role == Role::Compute
                    && s.backend == "hip"
                    && crate::resources::pci_key(&s.pci).is_ok_and(|key| key == s.pci)
                    && s.backend_uuid.as_deref().is_some_and(token)
            }
            _ => false,
        };
        if self.schema_version != 1
            || !self.synthetic
            || !device
            || self.formulation != "well_balanced_contact_angle_2d"
            || !(24..=96).contains(&self.resolution)
            || !(100..=800000).contains(&self.steps)
            || !positive(self.diameter_m)
            || self.diameter_m > 0.001
            || self.initial_center_above_wall_m != 0.
            || !positive(self.density_liquid_kg_m3)
            || self.density_liquid_kg_m3 > 1e5
            || self.density_liquid_kg_m3 != self.density_vapor_kg_m3
            || !positive(self.viscosity_liquid_m2_s)
            || self.viscosity_liquid_m2_s > 1.
            || self.viscosity_liquid_m2_s != self.viscosity_vapor_m2_s
            || !positive(self.surface_tension_n_m)
            || !positive(self.surface_tension_lattice())
            || self.surface_tension_lattice() > 0.02
            || !positive(self.physical_step_s())
            || !positive(self.interface_width_m)
            || self.interface_width_m > self.diameter_m / 6.
            || !(3. ..=16.).contains(&(self.interface_width_m / self.spacing_m()))
            || !(60. ..=120.).contains(&self.contact_angle_deg)
            || !(0.6..=1.5).contains(&self.phase_relaxation_time)
            || !positive(self.mass_tolerance)
            || self.mass_tolerance > 1e-3
            || !positive(self.angle_tolerance_deg)
            || self.angle_tolerance_deg > 5.
            || !(2..=32).contains(&self.observation_steps.len())
            || self.observation_steps.first() != Some(&0)
            || self.observation_steps.last() != Some(&self.steps)
            || self
                .observation_steps
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || [&self.material_provenance, &self.boundary_provenance]
                .iter()
                .any(|v| v.trim().is_empty() || v.len() > 4096)
        {
            return Err(invalid(
                "explicit bounded synthetic equal-property CPU or exact-card HIP planar wetting with resolved interface, SI inputs, provenance and unchanged mass/contact-angle gates required",
            ));
        }
        Ok(())
    }
}

pub fn verify_receipt(
    plan: &ExecutionPlan,
    root: &std::path::Path,
    value: &serde_json::Value,
) -> Result<crate::qualification::NumericalEvidence> {
    use sha2::{Digest, Sha256};
    let spec = plan
        .wetting
        .as_ref()
        .ok_or_else(|| invalid("independent wetting recipe required"))?;
    spec.validate()?;
    let number = |v: &serde_json::Value| {
        v.as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| invalid("finite wetting receipt component required"))
    };
    let close = |actual: f64, expected: f64, absolute: f64| {
        (actual - expected).abs() <= absolute + expected.abs() * 1e-12
    };
    if value["schema_version"] != 1
        || value["adapter"] != "OpenLB"
        || value["backend"] != spec.backend
        || value["executed"] != true
        || value["software_fallback"] != false
        || value["precision"] != "float64"
        || value["synthetic"] != true
        || value["dimensionality"] != 2
        || value["formulation"] != spec.formulation
        || value["request"] != serde_json::to_value(spec)?
        || value["request_sha256"] != digest(spec)?
        || value["source_revision"] != "145cd54810b468f4b6fd3ed86b10644264841578"
        || value["physical_validation"] != "unqualified"
    {
        return Err(invalid(
            "exact synthetic wetting recipe, immutable source, precision and backend required",
        ));
    }
    for (key, expected) in [
        ("spacing_m", spec.spacing_m()),
        ("physical_step_s", spec.physical_step_s()),
        ("wall_y_m", spec.spacing_m() / 2.),
        (
            "interface_width_lattice",
            spec.interface_width_m / spec.spacing_m(),
        ),
        ("surface_tension_lattice", spec.surface_tension_lattice()),
    ] {
        if !close(number(&value[key])?, expected, 0.) {
            return Err(invalid("wetting SI converter changed"));
        }
    }
    let (gpu_nodes, sysfs) = match &spec.selection {
        Some(selection) => {
            // The worker independently verifies the full PCI/KFD/DRM/compiled
            // runtime identity; this binds the receipt to the approved card.
            if value["pci"] != selection.pci
                || value["backend_uuid"] != serde_json::to_value(&selection.backend_uuid)?
                || value["gpu_kernel_completion_verified"] != true
                || value["gpu_blocks"] != 1
            {
                return Err(Error::Unqualified(
                    "HIP wetting receipt differs from the approved card or lacks kernel completion; no fallback".into(),
                ));
            }
            ("selected_gpu_nodes_only", "selected_sysfs_only")
        }
        None => ("no_gpu_nodes", "no_sysfs"),
    };
    let names = [
        "operation_closure_only",
        gpu_nodes,
        sysfs,
        "no_host_home",
        "no_session_bus",
        "no_worker_socket",
        "network_namespace_isolated",
        "descriptor_readonly",
    ];
    let sandbox = &value["sandbox"];
    if sandbox["policy"] != spec.sandbox_policy()
        || sandbox["checks"]
            .as_object()
            .is_none_or(|v| v.len() != names.len())
        || names.iter().any(|name| sandbox["checks"][name] != true)
    {
        return Err(invalid("complete isolated wetting sandbox required"));
    }
    let native = value["snapshots"]
        .as_array()
        .filter(|v| v.len() == spec.observation_steps.len())
        .ok_or_else(|| invalid("exact complete native wetting observations required"))?;
    let independent = value["independent_fields"]
        .as_array()
        .filter(|v| v.len() == native.len())
        .ok_or_else(|| invalid("complete independent wetting fields required"))?;
    let mut initial = 0.;
    let mut mass_error: f64 = 0.;
    let mut final_angle = 0.;
    for (i, (native, independent)) in native.iter().zip(independent).enumerate() {
        let step = spec.observation_steps[i];
        let path = format!("wetting-{step}.csv");
        let time = step as f64 * spec.physical_step_s();
        if native["step"] != step
            || independent["step"] != step
            || native["path"] != path
            || independent["path"] != path
            || !close(number(&native["time_s"])?, time, 0.)
            || !close(number(&independent["time_s"])?, time, 0.)
        {
            return Err(invalid(
                "exact native wetting step/time/path mapping required",
            ));
        }
        let data = crate::worker::read_bounded(
            &crate::storage::safe_path(root, &path)?,
            16 * 1024 * 1024,
        )?;
        if independent["sha256"] != format!("{:x}", Sha256::digest(&data)) {
            return Err(invalid("original wetting field bytes changed"));
        }
        let check = crate::wetting_fields::assess(spec, &data)?;
        let other = &independent["check"];
        if native["shape"] != serde_json::to_value(check.shape)?
            || other["shape"] != native["shape"]
            || native["nodes"] != check.shape[0] * check.shape[1]
            || native["fluid_nodes"] != check.fluid_nodes
            || other["fluid_nodes"] != check.fluid_nodes
            || other["contour_points"] != check.contour_points
        {
            return Err(invalid(
                "complete native wetting material/contour coverage changed",
            ));
        }
        for actual in [&native["droplet_area_m2"], &other["droplet_area_m2"]] {
            if !close(number(actual)?, check.droplet_area_m2, 0.) {
                return Err(invalid("independent native phase area changed"));
            }
        }
        for (key, expected, absolute) in [
            ("contact_angle_deg", check.contact_angle_deg, 1e-10),
            (
                "circle_radius_m",
                check.circle_radius_m,
                spec.spacing_m() * 1e-10,
            ),
            (
                "relative_radial_residual",
                check.relative_radial_residual,
                1e-10,
            ),
            ("max_speed_lattice", check.max_speed_lattice, 0.),
        ] {
            if !close(number(&other[key])?, expected, absolute) {
                return Err(invalid(
                    "independent raw wetting contour/velocity differs from receipt",
                ));
            }
        }
        for axis in 0..2 {
            if !close(
                number(&other["circle_center_m"][axis])?,
                check.circle_center_m[axis],
                spec.spacing_m() * 1e-10,
            ) {
                return Err(invalid("raw contour center differs from receipt"));
            }
        }
        if i == 0 {
            initial = check.droplet_area_m2;
        }
        mass_error = mass_error.max((check.droplet_area_m2 / initial - 1.).abs());
        final_angle = check.contact_angle_deg;
    }
    let angle_error = (final_angle - spec.contact_angle_deg).abs();
    let checks = &value["numerical_verification"];
    if mass_error > spec.mass_tolerance
        || angle_error > spec.angle_tolerance_deg
        || checks["mass_passed"] != true
        || checks["angle_passed"] != true
        || number(&checks["mass_tolerance"])? != spec.mass_tolerance
        || number(&checks["angle_tolerance_deg"])? != spec.angle_tolerance_deg
        || checks["physical_validation"] != "unqualified"
        || !close(number(&checks["mass_relative_error"])?, mass_error, 1e-12)
        || !close(number(&checks["angle_abs_error_deg"])?, angle_error, 1e-10)
    {
        return Err(invalid(
            "unchanged independent wetting phase-mass/contact-angle gate failed",
        ));
    }
    Ok(crate::qualification::NumericalEvidence{reference:format!("independent original Float64 phase area and whole-contour circle fit at {} native observations",native.len()),scope:"synthetic equal-property planar wetting; one declared mesh; refinement, settling and physical validation assessed separately".into(),error_kind:"maximum_approved_gate_fraction".into(),error:(mass_error/spec.mass_tolerance).max(angle_error/spec.angle_tolerance_deg),tolerance:1.})
}

/// Preserve native lattice velocity explicitly; conversion is defined by the
/// immutable SI descriptor, and is never confused with a physical velocity CSV.
pub(crate) fn annotate_fields(
    plan: &ExecutionPlan,
    artifacts: &mut [ArtifactManifest],
) -> Result<()> {
    let Some(spec) = &plan.wetting else {
        return Ok(());
    };
    spec.validate()?;
    for step in &spec.observation_steps {
        let path = format!("stages/wetting/wetting-{step}.csv");
        let matches = artifacts
            .iter_mut()
            .filter(|a| a.path == path)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(invalid(
                "each approved wetting observation must be registered exactly once",
            ));
        }
        for record in matches {
            if record.format != "csv" || record.bytes == 0 || record.bytes > 16 * 1024 * 1024 {
                return Err(invalid("bounded native wetting CSV manifest required"));
            }
            record.time_s = Some(*step as f64 * spec.physical_step_s());
            record.association = Some("native_lattice_point".into());
            record.units = Some("x_m:m,y_m:m,material:1,phi:1,u_lattice:1,v_lattice:1".into());
            record.provenance = format!(
                "original complete OpenLB Float64 lattice phase/velocity/material; approved native step {step}; physical velocity_m_s = lattice_velocity * spacing_m / physical_step_s from native-wetting-request.json; phase area_m2 = sum(1-phi)*spacing_m^2 over material 1; synthetic planar reference"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    pub(crate) fn fixture() -> (tempfile::TempDir, ExecutionPlan, serde_json::Value, Vec<u8>) {
        let spec:WettingReferenceSpec=serde_json::from_value(serde_json::json!({"schema_version":1,"synthetic":true,"backend":"cpu","formulation":"well_balanced_contact_angle_2d","diameter_m":48e-6,"initial_center_above_wall_m":0.,"resolution":96,"interface_width_m":6e-6,"density_liquid_kg_m3":1000.,"density_vapor_kg_m3":1000.,"viscosity_liquid_m2_s":1e-6,"viscosity_vapor_m2_s":1e-6,"surface_tension_n_m":1e-4,"contact_angle_deg":90.,"phase_relaxation_time":1.,"steps":100,"observation_steps":[0,50,100],"mass_tolerance":1e-3,"angle_tolerance_deg":5.,"material_provenance":"synthetic analytical cap fixture","boundary_provenance":"planar uniform wall"})).unwrap();
        let mut text = "x_m,y_m,material,phi,u_lattice,v_lattice\n".to_string();
        let n = spec.resolution as usize;
        let dx = spec.spacing_m();
        for y in 0..=3 * n / 2 {
            for x in 0..=5 * n / 2 {
                let radius = n as f64 / 2.;
                let distance = (x as f64 - 1.25 * n as f64).hypot(y as f64 - 0.5);
                let phi =
                    (1. + (4. * (distance - radius) / (spec.interface_width_m / dx)).tanh()) / 2.;
                text.push_str(&format!(
                    "{:.17e},{:.17e},{},{:.17e},0,0\n",
                    x as f64 * dx,
                    y as f64 * dx,
                    if y == 0 || y == 3 * n / 2 { 2 } else { 1 },
                    phi
                ));
            }
        }
        let data = text.into_bytes();
        let check = crate::wetting_fields::assess(&spec, &data).unwrap();
        assert!(
            (check.contact_angle_deg - 90.).abs() < 0.12 && check.relative_radial_residual < 0.003,
            "{check:?}"
        );
        let root = tempfile::tempdir().unwrap();
        let mut snapshots = Vec::new();
        let mut fields = Vec::new();
        for step in &spec.observation_steps {
            let name = format!("wetting-{step}.csv");
            std::fs::write(root.path().join(&name), &data).unwrap();
            let time = *step as f64 * spec.physical_step_s();
            snapshots.push(serde_json::json!({"path":name,"step":step,"time_s":time,"nodes":check.shape[0]*check.shape[1],"fluid_nodes":check.fluid_nodes,"shape":check.shape,"droplet_area_m2":check.droplet_area_m2}));
            fields.push(serde_json::json!({"path":name,"step":step,"time_s":time,"sha256":format!("{:x}",Sha256::digest(&data)),"check":check}));
        }
        let sandbox_names = [
            "operation_closure_only",
            "no_gpu_nodes",
            "no_sysfs",
            "no_host_home",
            "no_session_bus",
            "no_worker_socket",
            "network_namespace_isolated",
            "descriptor_readonly",
        ];
        let checks = serde_json::Map::from_iter(
            sandbox_names
                .into_iter()
                .map(|key| (key.to_string(), serde_json::json!(true))),
        );
        let receipt = serde_json::json!({"schema_version":1,"adapter":"OpenLB","backend":"cpu","executed":true,"software_fallback":false,"precision":"float64","source_revision":"145cd54810b468f4b6fd3ed86b10644264841578","formulation":spec.formulation,"dimensionality":2,"synthetic":true,"request":spec,"request_sha256":digest(&spec).unwrap(),"spacing_m":dx,"physical_step_s":spec.physical_step_s(),"wall_y_m":dx/2.,"interface_width_lattice":spec.interface_width_m/dx,"surface_tension_lattice":spec.surface_tension_lattice(),"physical_validation":"unqualified","snapshots":snapshots,"independent_fields":fields,"sandbox":{"policy":crate::execution::WETTING_SANDBOX_POLICY,"checks":checks},"numerical_verification":{"mass_relative_error":0.,"mass_tolerance":spec.mass_tolerance,"mass_passed":true,"angle_abs_error_deg":(check.contact_angle_deg-spec.contact_angle_deg).abs(),"angle_tolerance_deg":spec.angle_tolerance_deg,"angle_passed":true,"physical_validation":"unqualified"}});
        (
            root,
            ExecutionPlan::wetting_reference(spec, "research".into()).unwrap(),
            receipt,
            data,
        )
    }

    #[test]
    fn raw_native_wetting_gate_rejects_times_converter_sandbox_contour_and_byte_drift() {
        let (root, plan, receipt, data) = fixture();
        let evidence = verify_receipt(&plan, root.path(), &receipt).unwrap();
        assert!(evidence.error < 1. && evidence.scope.contains("refinement"));
        for field in [
            "request_sha256",
            "precision",
            "source_revision",
            "spacing_m",
            "sandbox",
            "snapshots",
            "independent_fields",
        ] {
            let mut changed = receipt.clone();
            changed[field] = serde_json::Value::Null;
            assert!(
                verify_receipt(&plan, root.path(), &changed).is_err(),
                "{field}"
            );
        }
        for field in ["time_s", "step", "path"] {
            let mut changed = receipt.clone();
            changed["independent_fields"][1][field] = serde_json::Value::Null;
            assert!(
                verify_receipt(&plan, root.path(), &changed).is_err(),
                "{field}"
            );
        }
        let mut changed = receipt.clone();
        changed["independent_fields"][2]["check"]["contact_angle_deg"] = serde_json::json!(90.);
        assert!(verify_receipt(&plan, root.path(), &changed).is_err());
        let mut changed = receipt.clone();
        changed["numerical_verification"]["mass_tolerance"] = serde_json::json!(0.01);
        assert!(verify_receipt(&plan, root.path(), &changed).is_err());
        let path = root.path().join("wetting-100.csv");
        std::fs::write(&path, &data[..data.len() - 100]).unwrap();
        assert!(verify_receipt(&plan, root.path(), &receipt).is_err());
        changed = receipt.clone();
        changed["independent_fields"][2]["sha256"] = serde_json::json!(format!(
            "{:x}",
            Sha256::digest(std::fs::read(&path).unwrap())
        ));
        assert!(verify_receipt(&plan, root.path(), &changed).is_err());
    }

    #[test]
    fn hip_wetting_receipt_binds_approved_card_sandbox_and_completion() {
        let (root, cpu_plan, cpu_receipt, _) = fixture();
        let mut spec = cpu_plan.wetting.clone().unwrap();
        spec.backend = "hip".into();
        spec.selection = Some(GpuSelection {
            role: Role::Compute,
            backend: "hip".into(),
            pci: "0000:03:00.0".into(),
            backend_uuid: Some(format!("GPU-{}", "ab".repeat(16))),
        });
        let plan = ExecutionPlan::wetting_reference(spec.clone(), "research".into()).unwrap();
        let mut receipt = cpu_receipt.clone();
        receipt["backend"] = "hip".into();
        receipt["request"] = serde_json::to_value(&spec).unwrap();
        receipt["request_sha256"] = digest(&spec).unwrap().into();
        receipt["pci"] = "0000:03:00.0".into();
        receipt["backend_uuid"] = format!("GPU-{}", "ab".repeat(16)).into();
        receipt["gpu_blocks"] = 1.into();
        receipt["gpu_kernel_completion_verified"] = true.into();
        let checks = &mut receipt["sandbox"]["checks"];
        let object = checks.as_object_mut().unwrap();
        object.remove("no_gpu_nodes");
        object.remove("no_sysfs");
        object.insert("selected_gpu_nodes_only".into(), true.into());
        object.insert("selected_sysfs_only".into(), true.into());
        receipt["sandbox"]["policy"] = HIP_SANDBOX_POLICY.into();
        verify_receipt(&plan, root.path(), &receipt).unwrap();
        // A CPU receipt never satisfies a HIP approval, and vice versa.
        assert!(verify_receipt(&plan, root.path(), &cpu_receipt).is_err());
        assert!(verify_receipt(&cpu_plan, root.path(), &receipt).is_err());
        for (key, value) in [
            ("pci", serde_json::json!("0000:04:00.0")),
            (
                "backend_uuid",
                serde_json::json!(format!("GPU-{}", "cd".repeat(16))),
            ),
            ("gpu_kernel_completion_verified", serde_json::json!(false)),
            ("gpu_blocks", serde_json::json!(2)),
        ] {
            let mut changed = receipt.clone();
            changed[key] = value;
            assert!(
                matches!(
                    verify_receipt(&plan, root.path(), &changed),
                    Err(Error::Unqualified(_))
                ),
                "{key}"
            );
        }
        let mut changed = receipt.clone();
        changed["sandbox"]["policy"] = crate::execution::WETTING_SANDBOX_POLICY.into();
        assert!(verify_receipt(&plan, root.path(), &changed).is_err());
    }

    #[test]
    fn wetting_export_metadata_retains_native_time_units_and_point_association() {
        let (root, plan, _, _) = fixture();
        let mut artifacts = plan
            .wetting
            .as_ref()
            .unwrap()
            .observation_steps
            .iter()
            .map(|step| {
                let name = format!("wetting-{step}.csv");
                let mut record = crate::storage::native_manifest(
                    root.path(),
                    &name,
                    16 * 1024 * 1024,
                    "fixture",
                )
                .unwrap();
                record.path = format!("stages/wetting/{name}");
                record
            })
            .collect::<Vec<_>>();
        annotate_fields(&plan, &mut artifacts).unwrap();
        for (record, time) in artifacts
            .iter()
            .zip(plan.wetting.as_ref().unwrap().times_s())
        {
            assert_eq!(record.time_s, Some(time));
            assert_eq!(record.association.as_deref(), Some("native_lattice_point"));
            assert!(record.units.as_ref().unwrap().contains("phi:1"));
            assert!(
                record
                    .provenance
                    .contains("lattice_velocity * spacing_m / physical_step_s")
            );
        }
        let mut duplicate = artifacts.clone();
        duplicate.push(artifacts[0].clone());
        assert!(annotate_fields(&plan, &mut duplicate).is_err());
        artifacts.pop();
        assert!(annotate_fields(&plan, &mut artifacts).is_err());
    }
}
