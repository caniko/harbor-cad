use harbor_cad::{
    authority::{CardBudget, HostAuthority, RouteOverride},
    contracts::*,
    worker::load_profile,
};

fn authority() -> HostAuthority {
    HostAuthority {
        schema_version: 1,
        fleetix_revision: FLEETIX_REV.into(),
        fleetix_contract_digest: fleetix_digest(),
        max_ram_bytes: 4 * 1024 * 1024 * 1024,
        ram_headroom_bytes: 1024 * 1024 * 1024,
        filesystems: vec![],
        cards: vec![CardBudget {
            pci: "0000:03:00.0".into(),
            max_vram_bytes: 24 * 1024 * 1024 * 1024,
            headroom_bytes: 2 * 1024 * 1024 * 1024,
        }],
        routes: vec![],
        allowed_devices: vec![GpuSelection {
            role: Role::Compute,
            backend: "hip".into(),
            pci: "0000:03:00.0".into(),
            backend_uuid: Some("GPU-exact-uuid".into()),
        }],
        overrides: vec![],
        native_runtimes: vec![],
        allowed_input_roots: vec!["/nonexistent".into()],
        kfd_exclusion: None,
    }
}

#[test]
fn absent_routes_require_an_exact_recorded_override_and_shared_card_budget() {
    let mut host = authority();
    host.validate().unwrap();
    let selected = host.allowed_devices[0].clone();
    assert!(host.authorize_selection(&selected, 1024).is_err());
    host.overrides.push(RouteOverride {
        selection: selected.clone(),
        reason: "explicit synthetic HIP qualification".into(),
    });
    host.authorize_selection(&selected, 1024).unwrap();
    let mut stale = selected.clone();
    stale.backend_uuid = Some("GPU-stale".into());
    assert!(host.authorize_selection(&stale, 1024).is_err());
    assert!(
        host.authorize_selection(&selected, host.cards[0].max_vram_bytes)
            .is_err()
    );
    host.cards.clear();
    assert!(host.validate().is_err());
}

#[test]
fn authority_rejects_drift_ambiguous_routes_and_unapproved_runtime_without_changing_v1() {
    let mut host = authority();
    let profile = load_profile(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/profiles/ci.json"
    )))
    .unwrap();
    host.allowed_input_roots = vec![profile.allowed_input_root.clone()];
    let plan = ExecutionPlan::reference(CaseSpec::reference()).unwrap();
    let original = (plan.id().unwrap(), digest(&profile).unwrap());
    host.authorize(&plan, &profile).unwrap();
    let mut native = profile.clone();
    native.native_runtime = Some("/nix/store/00000000000000000000000000000000-runtime".into());
    assert!(host.authorize(&plan, &native).is_err());
    host.routes = vec![host.allowed_devices[0].clone(); 2];
    assert!(host.validate().is_err());
    host.routes.clear();
    host.fleetix_contract_digest = "drift".into();
    assert!(host.validate().is_err());
    assert_eq!(original, (plan.id().unwrap(), digest(&profile).unwrap()));
    host = authority();
    host.cards[0].pci = "0000:03:20.0".into();
    assert!(host.validate().is_err());
}

#[test]
fn durable_authorization_binds_exact_execution_and_idempotency_retains_the_first_policy() {
    use harbor_cad::{
        authority::ExecutionAuthorization, execution::ExecutionBinding, storage::Store,
    };
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let runner = temp.path().join("runner");
    std::fs::write(&runner, b"fixed runner").unwrap();
    let profile = load_profile(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/profiles/ci.json"
    )))
    .unwrap();
    let plan = ExecutionPlan::reference(CaseSpec::reference()).unwrap();
    let binding = ExecutionBinding::capture(&plan, &profile, &runner, Default::default()).unwrap();
    let mut host = authority();
    host.allowed_input_roots = vec![profile.allowed_input_root.clone()];
    let authorization = ExecutionAuthorization::capture(&plan, &profile, &binding, &host).unwrap();
    let job = store
        .submit_authorized(&plan, "first", &profile, &binding, &authorization)
        .unwrap();
    let saved = digest(&store.execution_authorization(&job.id).unwrap().unwrap()).unwrap();
    host.ram_headroom_bytes += 1;
    let changed = ExecutionAuthorization::capture(&plan, &profile, &binding, &host).unwrap();
    assert_eq!(
        store
            .submit_authorized(&plan, "first", &profile, &binding, &changed)
            .unwrap()
            .id,
        job.id
    );
    assert_eq!(
        saved,
        digest(&store.execution_authorization(&job.id).unwrap().unwrap()).unwrap()
    );
    let mut wrong = authorization.clone();
    wrong.execution_binding_digest = "different executable".into();
    assert!(
        store
            .submit_authorized(&plan, "wrong", &profile, &binding, &wrong)
            .is_err()
    );
    store
        .transition(&job.id, "queued", "starting", None)
        .unwrap();
    store.finish(&job.id, 1, Some("retained evidence")).unwrap();
    let exported = temp.path().join("bundle");
    store.export(&job.id, &exported).unwrap();
    let data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(exported.join("execution.json")).unwrap()).unwrap();
    assert_eq!(
        data["execution_authorization"]["execution_binding_digest"],
        digest(&binding).unwrap()
    );
    assert_eq!(data["job"]["plan_digest"], plan.id().unwrap());
}
