//! Archived-record storage tests; these fabricated process identities are not native evidence.
use crate::{
    atmospheric_transport::{self, AtmosphericTransportSpec},
    authority::{ExecutionAuthorization, HostAuthority},
    contracts::{ExecutionPlan, HostExecutionProfile, StageOperation, digest},
    execution::{ExecutionBinding, FileIdentity},
    storage::{Store, commit_artifact, ingest_native_tree},
};
use std::{collections::BTreeMap, fs, os::unix::fs::MetadataExt};

fn archived_source(store: &Store) -> (String, ExecutionPlan) {
    let (original, atmosphere, receipt) = crate::atmosphere_fields::tests::fixture();
    let source_plan = ExecutionPlan::atmospheric_reference(atmosphere, "research".into()).unwrap();
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
        .submit_with_profile(&source_plan, "archived-source", &profile)
        .unwrap();
    let fake = FileIdentity {
        path: runtime.into(),
        sha256: "a".repeat(64),
        bytes: 1,
    };
    let binding = ExecutionBinding {
        schema_version: 1,
        runner_protocol: 1,
        sandbox_policy: crate::atmosphere::SANDBOX_POLICY.into(),
        plan_digest: source_plan.id().unwrap(),
        host_profile_digest: digest(&profile).unwrap(),
        runner: fake.clone(),
        native_runtime: Some(fake.clone()),
        native_files: BTreeMap::from([(
            serde_json::to_string(&StageOperation::AtmosphericReference).unwrap(),
            fake,
        )]),
    };
    let authority = HostAuthority {
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
        ExecutionAuthorization::capture(&source_plan, &profile, &binding, &authority).unwrap();
    for (table, hash, value) in [
        (
            "job_executions",
            digest(&binding).unwrap(),
            serde_json::to_value(&binding).unwrap(),
        ),
        (
            "job_authorizations",
            digest(&authorization).unwrap(),
            serde_json::to_value(&authorization).unwrap(),
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
    fs::write(
        original.path().join("atmosphere-receipt.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let dest = store.job_dir(&job.id).unwrap().join("stages/atmosphere");
    fs::create_dir_all(&dest).unwrap();
    let mut records = ingest_native_tree(
        original.path(),
        &dest,
        source_plan.observation.max_artifact_bytes,
    )
    .unwrap();
    for record in &mut records {
        record.path = format!("stages/atmosphere/{}", record.path);
    }
    crate::atmosphere::annotate_fields(&source_plan, &mut records).unwrap();
    store.add_artifacts(&job.id, &records).unwrap();
    store
        .transition(&job.id, "queued", "starting", None)
        .unwrap();
    store
        .transition(&job.id, "starting", "running", None)
        .unwrap();
    store.finish(&job.id, 0, None).unwrap();
    (job.id, source_plan)
}

#[test]
fn atmospheric_inputs_are_retained_before_acknowledgment_and_source_mutation_rejects() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let (id, source_plan) = archived_source(&store);
    let mut request: crate::atmosphere_transfer::AtmosphericTransferRequest =
        serde_json::from_str(include_str!("../examples/atmosphere-transfer.json")).unwrap();
    request.source_job = id.clone();
    let plan = atmospheric_transport::plan(&store, request, "research".into()).unwrap();
    assert!(store.submit(&plan, "missing-auth").is_err());
    let destination = store
        .submit(&source_plan, "storage-destination-only")
        .unwrap();
    atmospheric_transport::retain(&store, &destination.id, &plan).unwrap();
    let spec = plan.atmospheric_transport.as_ref().unwrap();
    let original = atmospheric_transport::registered_source(&store, spec).unwrap();
    let retained = store
        .job_dir(&destination.id)
        .unwrap()
        .join("source-atmosphere-original.txt");
    assert_eq!(fs::read(&original).unwrap(), fs::read(&retained).unwrap());
    assert_ne!(
        fs::metadata(&original).unwrap().ino(),
        fs::metadata(&retained).unwrap().ino()
    );
    assert_eq!(
        store
            .artifact_record(&destination.id, "source-atmosphere-original.txt")
            .unwrap()
            .unwrap()
            .sha256,
        spec.source.original.sha256
    );
    assert!(
        store
            .artifact_record(&destination.id, "atmospheric-source-execution.json")
            .unwrap()
            .is_some()
    );
    let before = fs::read(&retained).unwrap();
    fs::write(&original, b"changed original\n").unwrap();
    assert!(atmospheric_transport::registered_source(&store, spec).is_err());
    assert!(atmospheric_transport::retain(&store, &destination.id, &plan).is_err());
    assert_eq!(fs::read(&retained).unwrap(), before);
}

#[test]
fn orphan_atmospheric_inputs_preserve_the_only_source_copy_and_recover_when_proven() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let (id, _) = archived_source(&store);
    let mut request: crate::atmosphere_transfer::AtmosphericTransferRequest =
        serde_json::from_str(include_str!("../examples/atmosphere-transfer.json")).unwrap();
    request.source_job = id;
    let plan = atmospheric_transport::plan(&store, request, "research".into()).unwrap();
    let spec: AtmosphericTransportSpec = plan.atmospheric_transport.unwrap();
    let source = atmospheric_transport::registered_source(&store, &spec).unwrap();
    let saved = fs::read(&source).unwrap();
    let orphan = uuid::Uuid::new_v4().to_string();
    let root = store.root.join(format!("artifacts/{orphan}"));
    fs::create_dir_all(&root).unwrap();
    commit_artifact(
        &root,
        "atmospheric-source-retention.json",
        &serde_json::to_vec(&spec).unwrap(),
        "json",
        "manufactured failed-submission intent",
    )
    .unwrap();
    fs::write(root.join("source-atmosphere-original.txt"), &saved).unwrap();
    fs::remove_file(&source).unwrap();
    atmospheric_transport::recover_orphan(&store, &orphan).unwrap();
    assert_eq!(
        fs::read(root.join("source-atmosphere-original.txt")).unwrap(),
        saved
    );
    fs::write(&source, &saved).unwrap();
    atmospheric_transport::recover_orphan(&store, &orphan).unwrap();
    assert!(!root.exists());
}
