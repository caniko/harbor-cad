use harbor_cad::contracts::{CaseSpec, ExecutionPlan, StageOperation};
use harbor_cad::freezing::FreezingReferenceSpec;

fn fixture() -> FreezingReferenceSpec {
    serde_json::from_value(serde_json::json!({
        "schema_version":1,"synthetic":true,"backend":"cpu",
        "formulation":"conduction_stefan_solidification_2d",
        "size_m":[0.001,0.000125,0.001],"resolution":32,
        "density_kg_m3":1000.,"specific_heat_j_kg_k":1000.,
        "conductivity_w_m_k":1.,"latent_heat_j_kg":100000.,
        "melting_temperature_k":273.15,"initial_temperature_k":273.15,
        "cold_wall_temperature_k":263.15,"material_temperature_domain_k":[253.15,283.15],
        "steps":1024,"observation_steps":[0,512,1024],
        "front_tolerance":0.02,"temperature_tolerance":0.02,
        "mass_tolerance":1e-10,"energy_tolerance":1e-10,
        "material_provenance":"synthetic equal-phase constant properties; no expansion",
        "boundary_provenance":"prescribed cold xmin; insulated xmax; periodic y; zero velocity",
        "geometry_provenance":"synthetic fixed-volume planar reference",
        "moisture_risk":{"assessment":"missing","reason":"humidity unavailable"}
    }))
    .unwrap()
}

#[test]
fn explicit_solidification_scale_preserves_si_energy_and_diffusive_time_refinement() {
    let spec = fixture();
    let scale = spec.scale().unwrap();
    assert_eq!(scale.stefan_number, 0.1);
    assert_eq!(scale.spacing_m, 0.001 / 32.);
    assert!((scale.physical_step_s - 0.00016276041666666666).abs() < 1e-18);
    assert_eq!(scale.initial_specific_enthalpy_j_kg, 110000.);
    assert_eq!(scale.shape, [33, 4]);
    assert!((scale.active_volume_m3 - 1.2109375e-10).abs() < 1e-25);
    // Independent rational volume/mass reference; multiplication order can
    // differ by one Float64 rounding step across the SI converters.
    assert!((scale.cell_mass_kg / 9.765625e-10 - 1.).abs() < 1e-15);
    assert!((scale.duration_s - 1. / 6.).abs() < 1e-15);
    let mut finer = spec.clone();
    finer.resolution = 64;
    finer.steps *= 4;
    finer.observation_steps = finer.observation_steps.iter().map(|n| n * 4).collect();
    let refined = finer.scale().unwrap();
    assert_eq!(refined.duration_s, scale.duration_s);
    assert_eq!(refined.physical_step_s * 4., scale.physical_step_s);
    assert_eq!(refined.spacing_m * 2., scale.spacing_m);
}

#[test]
fn freezing_contract_rejects_missing_physics_nonconduction_and_weakened_acceptance() {
    let original = serde_json::to_value(fixture()).unwrap();
    for (name, value) in [
        ("backend", serde_json::json!("hip")),
        ("synthetic", serde_json::json!(false)),
        ("latent_heat_j_kg", serde_json::json!(0.)),
        ("cold_wall_temperature_k", serde_json::json!(283.15)),
        ("initial_temperature_k", serde_json::json!(274.15)),
        ("resolution", serde_json::json!(16)),
        ("observation_steps", serde_json::json!([0, 512, 512, 1024])),
        ("size_m", serde_json::json!([0.001, 0.00025, 0.001])),
        ("energy_tolerance", serde_json::json!(0.02)),
        ("mass_tolerance", serde_json::json!(0.001)),
        ("front_tolerance", serde_json::json!(0.03)),
        (
            "material_temperature_domain_k",
            serde_json::json!([270., 283.15]),
        ),
        ("geometry_provenance", serde_json::json!("")),
    ] {
        let mut changed = original.clone();
        changed[name] = value;
        assert!(
            serde_json::from_value::<FreezingReferenceSpec>(changed)
                .unwrap()
                .scale()
                .is_err(),
            "{name}"
        );
    }
    for key in [
        "pressure_pa",
        "solid_density_kg_m3",
        "retained_water_job_id",
        "inlet_velocity_m_s",
    ] {
        let mut changed = original.clone();
        changed[key] = serde_json::json!(1.);
        assert!(
            serde_json::from_value::<FreezingReferenceSpec>(changed).is_err(),
            "{key}"
        );
    }
    let mut changed = fixture();
    changed.conductivity_w_m_k = f64::INFINITY;
    assert!(changed.scale().is_err());
    changed = fixture();
    changed.density_kg_m3 = f64::MIN_POSITIVE / 1000.;
    assert!(changed.scale().is_err());
}

#[test]
fn gpu_solidification_is_a_typed_cpu_only_refusal_not_a_fallback() {
    for backend in ["hip", "cuda"] {
        let mut spec = fixture();
        spec.backend = backend.into();
        assert!(
            matches!(spec.scale(), Err(harbor_cad::Error::Unqualified(ref m)) if m == harbor_cad::freezing::GPU_REFUSAL),
            "{backend}"
        );
        assert!(ExecutionPlan::freezing_reference(spec, "research".into()).is_err());
    }
}

#[test]
fn version_twelve_freezing_plan_binds_native_times_geometry_and_conservative_allocated_resources() {
    let spec = fixture();
    let plan = ExecutionPlan::freezing_reference(spec.clone(), "research".into()).unwrap();
    assert_eq!(plan.schema_version, 12);
    assert_eq!(plan.stages[0].operation, StageOperation::FreezingReference);
    assert_eq!(plan.observation.retained_times_s, spec.times_s().unwrap());
    assert!(plan.stages[0].ram_bytes >= 512 * 1024 * 1024);
    assert_eq!(plan.stages[0].vram_bytes, 0);
    let wire = serde_json::to_value(&plan).unwrap();
    assert!(wire.get("case").is_none());
    assert_eq!(
        serde_json::from_value::<ExecutionPlan>(wire.clone())
            .unwrap()
            .id()
            .unwrap(),
        plan.id().unwrap()
    );
    for key in [
        "case",
        "thermal",
        "wetting",
        "contact",
        "thermal_contact",
        "source",
        "frames",
        "cad_source",
        "imported_fem",
        "filter",
        "fem",
    ] {
        let mut changed = wire.clone();
        changed[key] = serde_json::Value::Null;
        assert!(
            serde_json::from_value::<ExecutionPlan>(changed).is_err(),
            "{key}"
        );
    }
    let mut larger = spec.clone();
    larger.resolution *= 2;
    larger.steps *= 4;
    larger.observation_steps = larger.observation_steps.iter().map(|n| n * 4).collect();
    let mut changed = plan.clone();
    changed.freezing = Some(larger.clone());
    assert!(
        changed.validate().is_err(),
        "original geometry estimates must not authorize refinement"
    );
    let finer = ExecutionPlan::freezing_reference(larger, "research".into()).unwrap();
    assert_eq!(
        finer.observation.retained_times_s,
        plan.observation.retained_times_s
    );
    assert!(finer.observation.max_artifact_bytes > plan.observation.max_artifact_bytes);
    assert!(finer.stages[0].ram_bytes > plan.stages[0].ram_bytes);
    assert_ne!(finer.science_id().unwrap(), plan.science_id().unwrap());
    for mutate in 0..4 {
        let mut changed = plan.clone();
        match mutate {
            0 => changed.observation.retained_times_s[1] *= 0.5,
            1 => changed.stages[0].ram_bytes = 1,
            2 => changed.stages[1].dependencies.clear(),
            _ => changed.observation.checkpoint_times_s = vec![0.],
        }
        assert!(changed.validate().is_err());
    }
    let mut old = ExecutionPlan::reference(CaseSpec::reference()).unwrap();
    let original_id = old.id().unwrap();
    let mut injected = serde_json::to_value(&old).unwrap();
    injected["freezing"] = serde_json::to_value(&spec).unwrap();
    assert!(serde_json::from_value::<ExecutionPlan>(injected).is_err());
    old.freezing = Some(spec);
    assert!(old.validate().is_err());
    assert!(old.science_id().is_err());
    old.freezing = None;
    assert_eq!(old.id().unwrap(), original_id);
}
