use harbor_cad::{contracts::*, wetting::WettingReferenceSpec};

fn reference() -> serde_json::Value {
    serde_json::json!({"schema_version":1,"synthetic":true,"backend":"cpu","formulation":"well_balanced_contact_angle_2d","diameter_m":48e-6,"initial_center_above_wall_m":0.,"resolution":48,"interface_width_m":6e-6,
        "density_liquid_kg_m3":1000.,"density_vapor_kg_m3":1000.,"viscosity_liquid_m2_s":1e-6,"viscosity_vapor_m2_s":1e-6,"surface_tension_n_m":1e-4,"contact_angle_deg":100.,"phase_relaxation_time":1.,"steps":160000,"observation_steps":[0,80000,120000,160000],"mass_tolerance":1e-3,"angle_tolerance_deg":5.,"material_provenance":"synthetic equal-property fluid","boundary_provenance":"uniform planar wall angle; no inlet or gravity"})
}

#[test]
fn explicit_wetting_si_converter_matches_constant_tau_and_retained_observations() {
    let spec: WettingReferenceSpec = serde_json::from_value(reference()).unwrap();
    spec.validate().unwrap();
    assert!((spec.spacing_m() / 1e-6 - 1.).abs() < 1e-12);
    assert!((spec.physical_step_s() / (1e-6 / 6.) - 1.).abs() < 1e-12);
    assert!((spec.surface_tension_lattice() / (1e-4 / 0.036) - 1.).abs() < 1e-12);
    assert!((spec.times_s()[3] / (0.16 / 6.) - 1.).abs() < 1e-12);
    let mut changed = spec.clone();
    changed.boundary_provenance = "different wall input".into();
    assert_ne!(digest(&spec).unwrap(), digest(&changed).unwrap());
}

#[test]
fn wetting_rejects_water_air_ratio_unresolved_interfaces_and_weakened_scientific_gates() {
    for (key, value) in [
        ("synthetic", serde_json::json!(false)),
        ("backend", serde_json::json!("hip")),
        ("density_vapor_kg_m3", serde_json::json!(1.2)),
        ("viscosity_vapor_m2_s", serde_json::json!(1e-5)),
        ("mass_tolerance", serde_json::json!(0.01)),
        ("angle_tolerance_deg", serde_json::json!(6)),
        ("interface_width_m", serde_json::json!(1e-7)),
        ("surface_tension_n_m", serde_json::json!(0.072)),
        ("material_provenance", serde_json::json!(" ")),
        ("observation_steps", serde_json::json!([0, 160000, 160000])),
        ("steps", serde_json::json!(800001)),
        ("resolution", serde_json::json!(true)),
    ] {
        let mut raw = reference();
        raw[key] = value;
        assert!(
            serde_json::from_value::<WettingReferenceSpec>(raw)
                .map_or(true, |v| v.validate().is_err()),
            "{key}"
        );
    }
    let mut raw = reference();
    raw["contact_line_model"] = serde_json::json!("injected");
    assert!(serde_json::from_value::<WettingReferenceSpec>(raw).is_err());
}

#[test]
fn explicit_long_settling_budget_preserves_fields_materials_and_memory_admission() {
    let spec: WettingReferenceSpec = serde_json::from_value(reference()).unwrap();
    let short = ExecutionPlan::wetting_reference(spec.clone(), "research".into()).unwrap();
    let mut long_spec = spec;
    long_spec.steps = 800000;
    long_spec.observation_steps = vec![0, 400000, 600000, 800000];
    let long = ExecutionPlan::wetting_reference(long_spec.clone(), "research".into()).unwrap();
    assert_ne!(short.id().unwrap(), long.id().unwrap());
    assert_eq!(short.stages[0].ram_bytes, long.stages[0].ram_bytes);
    assert_eq!(
        short.observation.max_artifact_bytes,
        long.observation.max_artifact_bytes
    );
    assert_eq!(long.observation.retained_times_s, long_spec.times_s());
    assert!((long_spec.times_s()[3] - 2. / 15.).abs() < 1e-12);
    long_spec.steps = 800001;
    long_spec.observation_steps[3] = 800001;
    assert!(long_spec.validate().is_err());
}

fn hip() -> serde_json::Value {
    let mut raw = reference();
    raw["backend"] = "hip".into();
    raw["selection"] = serde_json::json!({"role":"compute","backend":"hip","pci":"0000:03:00.0","backend_uuid":format!("GPU-{}", "ab".repeat(16))});
    raw
}

#[test]
fn hip_wetting_binds_exact_compute_card_vram_and_no_fallback() {
    let cpu: WettingReferenceSpec = serde_json::from_value(reference()).unwrap();
    // CPU descriptors keep their historical serialization and science identity.
    assert!(
        serde_json::to_value(&cpu)
            .unwrap()
            .get("selection")
            .is_none()
    );
    let cpu_plan = ExecutionPlan::wetting_reference(cpu, "research".into()).unwrap();
    assert_eq!(cpu_plan.stages[0].gpu, GpuRequirement::CpuOnly);
    assert_eq!(cpu_plan.stages[0].vram_bytes, 0);
    let spec: WettingReferenceSpec = serde_json::from_value(hip()).unwrap();
    let plan = ExecutionPlan::wetting_reference(spec.clone(), "research".into()).unwrap();
    let stage = &plan.stages[0];
    assert_eq!(stage.gpu, GpuRequirement::Required);
    assert_eq!(stage.selection.as_ref().unwrap().pci, "0000:03:00.0");
    assert_eq!(stage.vram_bytes, stage.ram_bytes);
    assert_eq!(stage.ram_bytes, cpu_plan.stages[0].ram_bytes);
    assert_eq!(plan.stages[1].gpu, GpuRequirement::CpuOnly);
    for (key, value) in [
        ("role", serde_json::json!("render")),
        ("backend", serde_json::json!("cuda")),
        ("pci", serde_json::json!("0000:03:00")),
        ("backend_uuid", serde_json::Value::Null),
    ] {
        let mut raw = hip();
        raw["selection"][key] = value;
        let spec: WettingReferenceSpec = serde_json::from_value(raw).unwrap();
        assert!(spec.validate().is_err(), "{key}");
    }
    for (backend, selection) in [("hip", false), ("cpu", true), ("cuda", true)] {
        let mut raw = hip();
        raw["backend"] = backend.into();
        if !selection {
            raw.as_object_mut().unwrap().remove("selection");
        }
        let spec: WettingReferenceSpec = serde_json::from_value(raw).unwrap();
        assert!(spec.validate().is_err(), "{backend}");
    }
    for key in ["dropped", "card", "cpu"] {
        let mut changed = plan.clone();
        match key {
            "dropped" => changed.stages[0].selection = None,
            "card" => changed.stages[0].selection.as_mut().unwrap().pci = "0000:04:00.0".into(),
            _ => changed.stages[0].gpu = GpuRequirement::CpuOnly,
        }
        assert!(changed.validate().is_err(), "{key}");
    }
}

#[test]
fn version_nine_wetting_plan_retains_independent_native_times_and_closed_cpu_dag() {
    let spec: WettingReferenceSpec = serde_json::from_value(reference()).unwrap();
    let plan = ExecutionPlan::wetting_reference(spec.clone(), "research".into()).unwrap();
    let raw = serde_json::to_value(&plan).unwrap();
    assert_eq!(raw["schema_version"], 9);
    assert_eq!(raw["stages"][0]["operation"], "wetting_reference");
    assert_eq!(plan.observation.retained_times_s, spec.times_s());
    assert_eq!(plan.science_id().unwrap(), digest(&spec).unwrap());
    assert!(raw.get("case").is_none() && raw.get("source").is_none());
    assert_eq!(
        serde_json::from_value::<ExecutionPlan>(raw)
            .unwrap()
            .id()
            .unwrap(),
        plan.id().unwrap()
    );
    for key in ["checkpoint", "physics", "resource", "gpu"] {
        let mut changed = plan.clone();
        match key {
            "checkpoint" => changed.observation.checkpoint_times_s = vec![0.],
            "physics" => changed.case = Some(CaseSpec::reference()),
            "resource" => changed.stages[0].ram_bytes = 1024,
            _ => changed.stages[0].gpu = GpuRequirement::Required,
        }
        assert!(changed.validate().is_err(), "{key}");
    }
    for injected in [reference(), serde_json::Value::Null] {
        let mut old =
            serde_json::to_value(ExecutionPlan::reference(CaseSpec::reference()).unwrap()).unwrap();
        old["wetting"] = injected;
        assert!(serde_json::from_value::<ExecutionPlan>(old).is_err());
    }
}
