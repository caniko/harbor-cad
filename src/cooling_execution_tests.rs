//! Fabricated archival identities test storage/contracts, never qualify native execution.
use crate::{
    contracts::{ExecutionPlan, HostExecutionProfile, StageOperation, digest},
    cooling_execution::{self, CoolingExecutionRequest, CoolingExecutionSpec},
    execution::{ExecutionBinding, FileIdentity},
    storage::{Store, commit_artifact, ingest_native_tree},
};
use std::{collections::BTreeMap, fs, os::unix::fs::MetadataExt};

fn archive(store: &Store) -> (String, ExecutionPlan) {
    let (original, plan, receipt, _) = crate::wetting::tests::fixture();
    let runtime = "/nix/store/00000000000000000000000000000000-archived/runtime.json";
    let profile = HostExecutionProfile {
        schema_version: 1,
        policy: "research".into(),
        allowed_input_root: store.root.to_string_lossy().into(),
        max_ram_bytes: 4 * 1024 * 1024 * 1024,
        max_disk_bytes: 1024 * 1024 * 1024,
        threads: 2,
        timeout_seconds: 60,
        native_runtime: Some(runtime.into()),
        service_mode: "systemd".into(),
    };
    let job = store
        .submit_with_profile(&plan, "archived-source", &profile)
        .unwrap();
    let fake = FileIdentity {
        path: runtime.into(),
        sha256: "a".repeat(64),
        bytes: 1,
    };
    let binding = ExecutionBinding {
        schema_version: 1,
        runner_protocol: 1,
        sandbox_policy: crate::execution::WETTING_SANDBOX_POLICY.into(),
        plan_digest: plan.id().unwrap(),
        host_profile_digest: digest(&profile).unwrap(),
        runner: fake.clone(),
        native_runtime: Some(fake.clone()),
        native_files: BTreeMap::from([(
            serde_json::to_string(&StageOperation::WettingReference).unwrap(),
            fake,
        )]),
    };
    let authority = crate::authority::HostAuthority {
        schema_version: 1,
        fleetix_revision: crate::contracts::FLEETIX_REV.into(),
        fleetix_contract_digest: crate::contracts::fleetix_digest(),
        max_ram_bytes: profile.max_ram_bytes,
        ram_headroom_bytes: 0,
        filesystems: vec![],
        cards: vec![],
        routes: vec![],
        allowed_devices: vec![],
        overrides: vec![],
        native_runtimes: vec![runtime.into()],
        allowed_input_roots: vec![profile.allowed_input_root.clone()],
        kfd_exclusion: None,
    };
    let authorization =
        crate::authority::ExecutionAuthorization::capture(&plan, &profile, &binding, &authority)
            .unwrap();
    for (table, hash, value) in [
        (
            "job_executions",
            digest(&binding).unwrap(),
            serde_json::to_value(binding).unwrap(),
        ),
        (
            "job_authorizations",
            digest(&authorization).unwrap(),
            serde_json::to_value(authorization).unwrap(),
        ),
    ] {
        store
            .connection
            .execute(
                &format!("INSERT INTO {table} VALUES(?1,?2,?3)"),
                rusqlite::params![job.id, hash, serde_json::to_string(&value).unwrap()],
            )
            .unwrap();
    }
    for name in ["wetting-receipt.json", "verified-wetting-receipt.json"] {
        fs::write(
            original.path().join(name),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
    }
    let root = store.job_dir(&job.id).unwrap();
    let dest = root.join("stages/wetting");
    fs::create_dir_all(&dest).unwrap();
    let mut records =
        ingest_native_tree(original.path(), &dest, plan.observation.max_artifact_bytes).unwrap();
    for record in &mut records {
        record.path = format!("stages/wetting/{}", record.path);
    }
    crate::wetting::annotate_fields(&plan, &mut records).unwrap();
    store.add_artifacts(&job.id, &records).unwrap();
    store
        .add_artifact(
            &job.id,
            &commit_artifact(
                &root,
                "native-wetting-request.json",
                &serde_json::to_vec(plan.wetting.as_ref().unwrap()).unwrap(),
                "json",
                "fabricated archived wetting request",
            )
            .unwrap(),
        )
        .unwrap();
    store
        .transition(&job.id, "queued", "starting", None)
        .unwrap();
    store
        .transition(&job.id, "starting", "running", None)
        .unwrap();
    store.finish(&job.id, 0, None).unwrap();
    (job.id, plan)
}

fn request(id: &str, plan: &ExecutionPlan) -> CoolingExecutionRequest {
    let mut initialization: crate::retained_cooling::RetainedCoolingRequest =
        serde_json::from_str(include_str!("../examples/retained-cooling.json")).unwrap();
    initialization.retained.source_job = id.into();
    initialization.retained.physical_time_s = 0.;
    initialization.thermal.density.value = plan.wetting.as_ref().unwrap().density_liquid_kg_m3;
    CoolingExecutionRequest {
        schema_version: 1,
        initialization,
        spatial_refinement: 2,
        integration_substeps: 1,
        base_steps: 16,
        observation_base_steps: vec![0, 4, 16],
    }
}

pub(crate) fn fixture() -> (tempfile::TempDir, CoolingExecutionSpec, Vec<u8>) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let (id, plan) = archive(&store);
    let spec = cooling_execution::resolve(&store, request(&id, &plan)).unwrap();
    let raw = fs::read(
        store
            .job_dir(&id)
            .unwrap()
            .join(&spec.prepared.retained.original_field.path),
    )
    .unwrap();
    (temp, spec, raw)
}

#[test]
fn source_bound_plan_rejects_old_schema_null_capabilities_underestimates_and_changed_originals() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let (id, source) = archive(&store);
    let spec = cooling_execution::resolve(&store, request(&id, &source)).unwrap();
    let plan = ExecutionPlan::retained_cooling(spec, "research".into()).unwrap();
    assert_eq!(plan.schema_version, 18);
    assert_eq!(plan.stages[0].id, "retained-cooling");
    assert!(plan.peak_ram() >= 512 * 1024 * 1024);
    assert!(plan.observation.max_artifact_bytes > 16 * 1024 * 1024);
    let raw = serde_json::to_value(&plan).unwrap();
    let parsed: ExecutionPlan = serde_json::from_value(raw.clone()).unwrap();
    parsed.validate().unwrap();
    assert_eq!(parsed.id().unwrap(), plan.id().unwrap());
    for version in 1..=17 {
        let mut changed = raw.clone();
        changed["schema_version"] = version.into();
        assert!(serde_json::from_value::<ExecutionPlan>(changed).is_err());
    }
    for (key, value) in [
        ("freezing", serde_json::Value::Null),
        ("source", serde_json::Value::Null),
        ("retained_cooling", serde_json::Value::Null),
    ] {
        let mut changed = raw.clone();
        changed[key] = value;
        assert!(serde_json::from_value::<ExecutionPlan>(changed).is_err());
    }
    let mut changed = plan.clone();
    changed.stages[0].ram_bytes = 1;
    assert!(changed.validate().is_err());
    changed = plan.clone();
    changed.observation.max_artifact_bytes = 1;
    assert!(changed.validate().is_err());
    assert!(store.submit(&plan, "missing-auth").is_err());
    let original = store
        .job_dir(&id)
        .unwrap()
        .join("stages/wetting/wetting-0.csv");
    let bytes = fs::read(&original).unwrap();
    fs::write(original, &bytes[..bytes.len() - 10]).unwrap();
    assert!(cooling_execution::source(&store, plan.retained_cooling.as_ref().unwrap()).is_err());
}

#[test]
fn distinct_original_history_retention_survives_source_loss_and_orphans_remain_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let (id, source) = archive(&store);
    let spec = cooling_execution::resolve(&store, request(&id, &source)).unwrap();
    let plan = ExecutionPlan::retained_cooling(spec.clone(), "research".into()).unwrap();
    let destination = store.submit(&source, "storage-only").unwrap();
    cooling_execution::retain(&store, &destination.id, &plan).unwrap();
    let root = store.job_dir(&destination.id).unwrap();
    let original = store.job_dir(&id).unwrap();
    for record in &spec.originals {
        let copy = root.join("source-wetting").join(&record.path);
        let source = original.join(&record.path);
        assert_eq!(fs::read(&copy).unwrap(), fs::read(&source).unwrap());
        assert_ne!(
            fs::metadata(copy).unwrap().ino(),
            fs::metadata(source).unwrap().ino()
        );
    }
    cooling_execution::registered(&store, &destination.id, &plan).unwrap();
    let altered_orphan = uuid::Uuid::new_v4().to_string();
    let altered_root = store.root.join("artifacts").join(&altered_orphan);
    crate::storage::private_dir(&altered_root).unwrap();
    commit_artifact(
        &altered_root,
        "cooling-source-retention.json",
        &serde_json::to_vec(&spec).unwrap(),
        "json",
        "fabricated orphan",
    )
    .unwrap();
    let mut provenance: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("cooling-source-execution.json")).unwrap())
            .unwrap();
    provenance["execution_binding"]["runner"]["sha256"] = "b".repeat(64).into();
    commit_artifact(
        &altered_root,
        "cooling-source-execution.json",
        &serde_json::to_vec(&provenance).unwrap(),
        "json",
        "fabricated changed orphan provenance",
    )
    .unwrap();
    assert!(cooling_execution::recover_orphan(&store, &altered_orphan).is_err());
    assert!(altered_root.join("cooling-source-retention.json").is_file());
    assert!(altered_root.join("cooling-source-execution.json").is_file());
    let orphan = uuid::Uuid::new_v4().to_string();
    let oroot = store.root.join("artifacts").join(&orphan);
    crate::storage::private_dir(&oroot).unwrap();
    commit_artifact(
        &oroot,
        "cooling-source-retention.json",
        &serde_json::to_vec(&spec).unwrap(),
        "json",
        "fabricated orphan",
    )
    .unwrap();
    fs::rename(&original, temp.path().join("source-loss")).unwrap();
    assert!(cooling_execution::source(&store, &spec).is_err());
    cooling_execution::registered(&store, &destination.id, &plan).unwrap();
    assert!(cooling_execution::recover_orphan(&store, &orphan).is_err());
    assert!(oroot.join("cooling-source-retention.json").is_file());
    let path = root
        .join("source-wetting")
        .join(&spec.prepared.retained.original_field.path);
    let bytes = fs::read(&path).unwrap();
    fs::write(path, &bytes[..bytes.len() - 10]).unwrap();
    assert!(cooling_execution::registered(&store, &destination.id, &plan).is_err());
}
