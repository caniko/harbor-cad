use harbor_cad::cad_source::CadMeshDescriptor;
use harbor_cad::{
    authority::{ExecutionAuthorization, HostAuthority},
    contracts::*,
    execution::{ExecutionBinding, FileIdentity},
    storage::{Store, commit_artifact},
};

fn descriptor() -> serde_json::Value {
    serde_json::json!({"schema_version":1,"synthetic":true,"backend":"cpu","formulation":"imported_axis_aligned_box",
        "geometry_provenance":"controlled synthetic imported Part solid","brep_file":"solid.brep","brep_sha256":"a".repeat(64),"brep_bytes":1000,
        "region_name":"solid","bounds_m":[0.1,0.12,-0.02,-0.01,0.3,0.31],"volume_m3":2e-6,
        "source_unit":"mm","scale_to_m":0.001,"placement_translation_unit":"mm",
        "source_transform":[1.,0.,0.,100.,0.,1.,0.,-20.,0.,0.,1.,300.,0.,0.,0.,1.],"resolution":4,
        "geometry_tolerance_m":1e-6,"volume_relative_tolerance":1e-10})
}

#[test]
fn explicit_imported_descriptor_retains_world_bounds_source_units_and_bounded_mesh() {
    let geometry: CadMeshDescriptor = serde_json::from_value(descriptor()).unwrap();
    geometry.validate().unwrap();
    assert!((geometry.lengths_m()[0] - 0.02).abs() < 1e-15);
    assert_eq!(geometry.source_transform[3], 100.);
    assert_eq!(geometry.source_unit, "mm");
    assert_eq!(geometry.brep_file, "solid.brep");
}

#[test]
fn imported_geometry_rejects_source_scope_units_nonrigid_placements_and_weak_gates() {
    for (key, value) in [
        ("backend", serde_json::json!("hip")),
        ("brep_file", serde_json::json!("../solid.brep")),
        ("brep_sha256", serde_json::json!("a".repeat(63))),
        ("brep_bytes", serde_json::json!(0)),
        ("source_unit", serde_json::json!("m")),
        ("scale_to_m", serde_json::json!(1.)),
        ("volume_m3", serde_json::json!(4e-6)),
        ("volume_relative_tolerance", serde_json::json!(1e-4)),
        ("geometry_tolerance_m", serde_json::json!(0.01)),
        ("resolution", serde_json::json!(33)),
        ("geometry_provenance", serde_json::json!("")),
        (
            "source_transform",
            serde_json::json!([
                2., 0., 0., 100., 0., 1., 0., -20., 0., 0., 1., 300., 0., 0., 0., 1.
            ]),
        ),
        (
            "source_transform",
            serde_json::json!([
                -1., 0., 0., 100., 0., 1., 0., -20., 0., 0., 1., 300., 0., 0., 0., 1.
            ]),
        ),
    ] {
        let mut value_record = descriptor();
        value_record[key] = value;
        let decoded = serde_json::from_value::<CadMeshDescriptor>(value_record);
        assert!(
            decoded.is_err() || decoded.unwrap().validate().is_err(),
            "{key}"
        );
    }
    let mut foreign = descriptor();
    foreign["contact"] = serde_json::json!(true);
    assert!(serde_json::from_value::<CadMeshDescriptor>(foreign).is_err());
}

fn archived_source(store: &Store) -> String {
    archived_source_with_geometry(store, descriptor())
}

fn archived_source_with_geometry(store: &Store, geometry: serde_json::Value) -> String {
    // Persisted-record fixture only; these opaque bytes establish no native
    // CAD execution or hardware qualification.
    let mut case = CaseSpec::reference();
    case.regions = vec!["solid".into()];
    case.geometry.source = "controlled.FCStd".into();
    let document = b"opaque archived original CAD document";
    use sha2::{Digest, Sha256};
    case.geometry.sha256 = Some(format!("{:x}", Sha256::digest(document)));
    let plan = ExecutionPlan::cad_inspection(case, "research".into(), 64 * 1024 * 1024).unwrap();
    let profile = HostExecutionProfile {
        schema_version: 1,
        policy: "research".into(),
        allowed_input_root: store.root.to_string_lossy().into(),
        max_ram_bytes: 2 * 1024 * 1024 * 1024,
        max_disk_bytes: 1024 * 1024 * 1024,
        threads: 1,
        timeout_seconds: 60,
        native_runtime: None,
        service_mode: "systemd".into(),
    };
    let job = store
        .submit_with_profile(&plan, "archived-cad", &profile)
        .unwrap();
    let identity = FileIdentity {
        path: "/nix/store/archived-cad-fixture/file".into(),
        sha256: "a".repeat(64),
        bytes: 1,
    };
    let binding = ExecutionBinding {
        schema_version: 1,
        runner_protocol: 1,
        sandbox_policy: harbor_cad::execution::SANDBOX_POLICY.into(),
        plan_digest: plan.id().unwrap(),
        host_profile_digest: digest(&profile).unwrap(),
        runner: identity.clone(),
        native_runtime: Some(identity.clone()),
        native_files: std::collections::BTreeMap::from([
            (
                serde_json::to_string(&StageOperation::CadInspect).unwrap(),
                identity.clone(),
            ),
            ("cad_closure".into(), identity),
        ]),
    };
    let authority = HostAuthority {
        schema_version: 1,
        fleetix_revision: FLEETIX_REV.into(),
        fleetix_contract_digest: fleetix_digest(),
        max_ram_bytes: profile.max_ram_bytes,
        ram_headroom_bytes: 0,
        filesystems: vec![],
        cards: vec![],
        routes: vec![],
        allowed_devices: vec![],
        overrides: vec![],
        native_runtimes: vec![],
        allowed_input_roots: vec![profile.allowed_input_root.clone()],
        kfd_exclusion: None,
    };
    let authorization =
        ExecutionAuthorization::capture(&plan, &profile, &binding, &authority).unwrap();
    for (table, record, record_digest) in [
        (
            "job_executions",
            serde_json::to_value(&binding).unwrap(),
            digest(&binding).unwrap(),
        ),
        (
            "job_authorizations",
            serde_json::to_value(&authorization).unwrap(),
            digest(&authorization).unwrap(),
        ),
    ] {
        store
            .connection
            .execute(
                &format!("INSERT INTO {table} VALUES(?1,?2,?3)"),
                rusqlite::params![
                    job.id,
                    record_digest,
                    serde_json::to_string(&record).unwrap()
                ],
            )
            .unwrap();
    }
    let root = store.job_dir(&job.id).unwrap();
    store
        .add_artifact(
            &job.id,
            &commit_artifact(
                &root,
                "input.FCStd",
                document,
                "FCStd",
                "persisted-record fixture; not native CAD qualification",
            )
            .unwrap(),
        )
        .unwrap();
    let brep = commit_artifact(
        &root,
        "solid.brep",
        b"opaque archived BREP bytes",
        "brep",
        "persisted-record fixture",
    )
    .unwrap();
    store.add_artifact(&job.id, &brep).unwrap();
    let region = serde_json::json!({"name":"solid","label":"solid","volume_m3":geometry["volume_m3"],"bounds_m":geometry["bounds_m"],"transform":geometry["source_transform"],"triangles":12,"source_unit":"mm","stl_scale_to_m":0.001});
    let manifest_region = serde_json::json!({"region_name":"solid","path":"solid.brep","bytes":brep.bytes,"sha256":brep.sha256,"source_unit":"mm","scale_to_m":0.001,
        "bounds_m":geometry["bounds_m"],"volume_m3":geometry["volume_m3"],"source_transform":geometry["source_transform"],"placement_translation_unit":"mm"});
    for (path, value) in [
        (
            "cad_inspect-receipt.json",
            serde_json::json!({"adapter":"FreeCAD","executed":true,"backend":"cpu","software_fallback":false,"security_minimum":"1.1.4","sandbox_required":true,"import_policy":harbor_cad::sandbox::IMPORT_POLICY}),
        ),
        (
            "regions.json",
            serde_json::json!({"synthetic":true,"regions":[region],"gap_healing":false,"geometry_tolerance":{"value":1e-5,"unit":"m"}}),
        ),
        (
            "brep-manifest.json",
            serde_json::json!({"schema_version":1,"synthetic":true,"regions":[manifest_region],"gap_healing":false}),
        ),
    ] {
        store
            .add_artifact(
                &job.id,
                &commit_artifact(
                    &root,
                    path,
                    &serde_json::to_vec(&value).unwrap(),
                    "json",
                    "persisted-record fixture",
                )
                .unwrap(),
            )
            .unwrap();
    }
    store
        .transition(&job.id, "queued", "starting", None)
        .unwrap();
    store
        .transition(&job.id, "starting", "running", None)
        .unwrap();
    store.finish(&job.id, 0, None).unwrap();
    job.id
}

#[test]
fn retained_cad_source_binds_original_approvals_and_rejects_byte_or_metadata_substitution() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = archived_source(&store);
    let (root, source) = harbor_cad::cad_source::source(&store, &id, "solid", 4, 1e-6).unwrap();
    source.validate().unwrap();
    assert_eq!(
        source.geometry.bounds_m,
        [0.1, 0.12, -0.02, -0.01, 0.3, 0.31]
    );
    assert_eq!(source.geometry.brep_sha256, source.brep.sha256);
    assert_eq!(source.geometry.region_name, "solid");
    assert!(harbor_cad::cad_source::source(&store, &id, "missing", 4, 1e-6).is_err());
    let original = std::fs::read(root.join("solid.brep")).unwrap();
    std::fs::write(root.join("solid.brep"), b"substituted BREP bytes").unwrap();
    assert!(harbor_cad::cad_source::source(&store, &id, "solid", 4, 1e-6).is_err());
    std::fs::write(root.join("solid.brep"), original).unwrap();
    let original = std::fs::read(root.join("brep-manifest.json")).unwrap();
    let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
    changed["regions"][0]["source_transform"][3] = serde_json::json!(200.);
    std::fs::write(
        root.join("brep-manifest.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    assert!(harbor_cad::cad_source::source(&store, &id, "solid", 4, 1e-6).is_err());
    std::fs::write(root.join("brep-manifest.json"), original).unwrap();
    store
        .connection
        .execute("DELETE FROM job_authorizations WHERE job=?1", [&id])
        .unwrap();
    assert!(harbor_cad::cad_source::source(&store, &id, "solid", 4, 1e-6).is_err());
}

#[test]
fn imported_mesh_plan_is_versioned_independent_and_cannot_be_injected_into_older_plans() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = archived_source(&store);
    let (_, source) = harbor_cad::cad_source::source(&store, &id, "solid", 4, 1e-6).unwrap();
    let plan = ExecutionPlan::cad_mesh(source.clone(), "research".into()).unwrap();
    let raw = serde_json::to_value(&plan).unwrap();
    assert_eq!(raw["schema_version"], 7);
    assert!(raw.get("case").is_none());
    assert!(raw.get("fem").is_none());
    assert!(raw.get("thermal").is_none());
    assert_eq!(raw["cad_source"], serde_json::to_value(&source).unwrap());
    assert_eq!(raw["stages"][0]["operation"], "cad_mesh");
    assert!(plan.observation.retained_times_s.is_empty());
    assert_eq!(
        serde_json::from_value::<ExecutionPlan>(raw.clone())
            .unwrap()
            .id()
            .unwrap(),
        plan.id().unwrap()
    );
    for version in 1..=6 {
        let mut old = raw.clone();
        old["schema_version"] = serde_json::json!(version);
        assert!(serde_json::from_value::<ExecutionPlan>(old).is_err());
    }
    let mut old =
        serde_json::to_value(ExecutionPlan::reference(CaseSpec::reference()).unwrap()).unwrap();
    old["cad_source"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<ExecutionPlan>(old).is_err());
    let mut weak = plan.clone();
    weak.stages[0].ram_bytes = 1;
    assert!(weak.validate().is_err());
    let mut injected = plan.clone();
    injected.stages[0].operation = StageOperation::FemReference;
    assert!(injected.validate().is_err());
    assert!(ExecutionPlan::cad_mesh(source, "ci".into()).is_err());
    assert!(store.submit(&plan, "unauthorized-mesh").is_err());
}

#[test]
fn controlled_variant_binds_original_document_units_and_preserved_box_placement() {
    use harbor_cad::cad_variant::{CadVariantRequest, verify_snapshot};
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = archived_source(&store);
    let request:CadVariantRequest=serde_json::from_value(serde_json::json!({"schema_version":1,"source_job":id,"region_name":"solid",
        "dimensions":[{"value":30.,"unit":"mm"},{"value":20.,"unit":"mm"},{"value":10.,"unit":"mm"}],
        "geometry_tolerance":{"value":1e-6,"unit":"m"},"provenance":"explicit controlled primitive copy; no physical claim"})).unwrap();
    let plan = harbor_cad::cad_variant::plan(&store, request.clone(), "research".into()).unwrap();
    let spec = plan.cad_variant.as_ref().unwrap();
    assert_eq!(plan.schema_version, 16);
    assert_eq!(spec.request.dimensions[0].unit, "mm");
    assert_eq!(spec.source.geometry.source_transform[3], 100.);
    assert_eq!(
        spec.source.geometry.bounds_m,
        [0.1, 0.12, -0.02, -0.01, 0.3, 0.31]
    );
    assert_eq!(spec.bounds_m().unwrap(), [0.1, 0.13, -0.02, 0., 0.3, 0.31]);
    assert_eq!(
        serde_json::from_value::<ExecutionPlan>(serde_json::to_value(&plan).unwrap())
            .unwrap()
            .id()
            .unwrap(),
        plan.id().unwrap()
    );
    let (_, record) = harbor_cad::cad_source::source(&store, &id, "solid", 2, 1e-6).unwrap();
    let mut region: harbor_cad::cad::RegionSnapshot = serde_json::from_slice(
        &std::fs::read(store.job_dir(&id).unwrap().join("regions.json")).unwrap(),
    )
    .unwrap();
    assert!(verify_snapshot(spec, &region).is_err());
    region.geometry_tolerance = request.geometry_tolerance.clone();
    region.regions[0].bounds_m = spec.bounds_m().unwrap();
    region.regions[0].volume_m3 = 6e-6;
    verify_snapshot(spec, &region).unwrap();
    assert_eq!(record.geometry.volume_m3, 2e-6);
    let mut changed = plan.clone();
    changed.cad_variant.as_mut().unwrap().request.dimensions[0].value = 31.;
    changed.validate().unwrap();
    assert_ne!(changed.science_id().unwrap(), plan.science_id().unwrap());
    assert_ne!(changed.id().unwrap(), plan.id().unwrap());
    region.regions[0].transform[3] = 101.;
    assert!(verify_snapshot(spec, &region).is_err());
    assert!(store.submit(&plan, "unbound-variant").is_err());
    std::fs::write(
        store.job_dir(&id).unwrap().join("input.FCStd"),
        b"substituted original source",
    )
    .unwrap();
    assert!(harbor_cad::cad_variant::plan(&store, request, "research".into()).is_err());
}

#[test]
fn controlled_variant_rejects_old_envelopes_foreign_physics_and_understated_resources() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = archived_source(&store);
    let request=serde_json::from_value(serde_json::json!({"schema_version":1,"source_job":id,"region_name":"solid","dimensions":[{"value":0.03,"unit":"m"},{"value":0.02,"unit":"m"},{"value":0.01,"unit":"m"}],"geometry_tolerance":{"value":1e-6,"unit":"m"},"provenance":"explicit synthetic variant"})).unwrap();
    let plan = harbor_cad::cad_variant::plan(&store, request, "research".into()).unwrap();
    for version in 1..=15 {
        let mut value = serde_json::to_value(&plan).unwrap();
        value["schema_version"] = serde_json::json!(version);
        assert!(serde_json::from_value::<ExecutionPlan>(value).is_err());
    }
    let baseline = ExecutionPlan::reference(CaseSpec::reference()).unwrap();
    for injected in [
        serde_json::Value::Null,
        serde_json::to_value(&plan.cad_variant).unwrap(),
    ] {
        let mut old = serde_json::to_value(&baseline).unwrap();
        old["cad_variant"] = injected;
        assert!(serde_json::from_value::<ExecutionPlan>(old).is_err());
    }
    let mut changed = plan.clone();
    changed.stages[0].operation = StageOperation::Openlb;
    assert!(changed.validate().is_err());
    let mut changed = plan.clone();
    changed.stages[0].ram_bytes = 1;
    assert!(changed.validate().is_err());
    let mut changed = plan.clone();
    changed.observation.max_artifact_bytes = 16 * 1024 * 1024;
    assert!(changed.validate().is_err());
    let mut changed = plan.clone();
    changed.cad_variant.as_mut().unwrap().request.dimensions[0].unit = "kg".into();
    assert!(changed.validate().is_err());
    let mut changed = plan.clone();
    changed
        .cad_variant
        .as_mut()
        .unwrap()
        .request
        .provenance
        .clear();
    assert!(changed.validate().is_err());
    let mut changed = plan.clone();
    changed.source = baseline.source;
    changed.schema_version = 1;
    assert!(changed.validate().is_err());
}

#[test]
fn variant_staging_copies_originals_before_ack_and_preserves_unverifiable_orphans() {
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = archived_source(&store);
    let request=serde_json::from_value(serde_json::json!({"schema_version":1,"source_job":id,"region_name":"solid","dimensions":[{"value":30.,"unit":"mm"},{"value":20.,"unit":"mm"},{"value":10.,"unit":"mm"}],"geometry_tolerance":{"value":1e-6,"unit":"m"},"provenance":"storage-only controlled variant fixture; no native qualification"})).unwrap();
    let plan = harbor_cad::cad_variant::plan(&store, request, "research".into()).unwrap();
    let mut profile = store.job_profile(&id).unwrap();
    profile.service_mode = "foreground".into();
    let binding = ExecutionBinding::capture(
        &plan,
        &profile,
        &std::env::current_exe().unwrap(),
        std::collections::BTreeMap::new(),
    )
    .unwrap();
    let authority = store
        .execution_authorization(&id)
        .unwrap()
        .unwrap()
        .authority;
    let auth = ExecutionAuthorization::capture(&plan, &profile, &binding, &authority).unwrap();
    let submit = |key| {
        store
            .submit_authorized(&plan, key, &profile, &binding, &auth)
            .unwrap()
    };
    let first = submit("copy-before-ack");
    let retained = harbor_cad::cad_variant::registered(&store, &first.id, &plan).unwrap();
    let original = store.job_dir(&id).unwrap().join("input.FCStd");
    assert_ne!(
        std::fs::metadata(&original).unwrap().ino(),
        std::fs::metadata(&retained).unwrap().ino()
    );
    assert_eq!(
        std::fs::read(&original).unwrap(),
        std::fs::read(&retained).unwrap()
    );
    assert_eq!(submit("copy-before-ack").id, first.id);
    let intact = submit("intact-orphan");
    store
        .connection
        .execute("DELETE FROM jobs WHERE id=?1", [&intact.id])
        .unwrap();
    store.cleanup_retention(|_| Ok(false)).unwrap();
    assert!(!store.root.join("artifacts").join(intact.id).exists());
    let lost = submit("lost-original-orphan");
    store
        .connection
        .execute("DELETE FROM jobs WHERE id=?1", [&lost.id])
        .unwrap();
    std::fs::write(&original, b"changed source after submission").unwrap();
    // The acknowledged independent copy retains the original scientific input.
    harbor_cad::cad_variant::registered(&store, &first.id, &plan).unwrap();
    store.cleanup_retention(|_| Ok(false)).unwrap();
    assert!(
        store
            .root
            .join("artifacts")
            .join(lost.id)
            .join("source-document.FCStd")
            .exists()
    );
    std::fs::write(retained, b"changed private original copy").unwrap();
    assert!(harbor_cad::cad_variant::registered(&store, &first.id, &plan).is_err());
}

#[test]
fn variant_original_receipt_geometry_and_export_mutations_cannot_qualify_history() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = archived_source(&store);
    let request=serde_json::from_value(serde_json::json!({"schema_version":1,"source_job":id,"region_name":"solid","dimensions":[{"value":30.,"unit":"mm"},{"value":20.,"unit":"mm"},{"value":10.,"unit":"mm"}],"geometry_tolerance":{"value":1e-6,"unit":"m"},"provenance":"persisted native-shape envelope fixture; not native execution evidence"})).unwrap();
    let plan = harbor_cad::cad_variant::plan(&store, request, "research".into()).unwrap();
    let spec = plan.cad_variant.as_ref().unwrap();
    let mut profile = store.job_profile(&id).unwrap();
    profile.service_mode = "foreground".into();
    let binding = ExecutionBinding::capture(
        &plan,
        &profile,
        &std::env::current_exe().unwrap(),
        std::collections::BTreeMap::new(),
    )
    .unwrap();
    let authority = store
        .execution_authorization(&id)
        .unwrap()
        .unwrap()
        .authority;
    let auth = ExecutionAuthorization::capture(&plan, &profile, &binding, &authority).unwrap();
    let job = store
        .submit_authorized(&plan, "persisted-envelope-only", &profile, &binding, &auth)
        .unwrap();
    let root = store.job_dir(&job.id).unwrap();
    let save = |path: &str, data: &[u8], format| {
        let art = commit_artifact(
            &root,
            path,
            data,
            format,
            "persisted-byte verification fixture; no native or physical qualification",
        )
        .unwrap();
        store.add_artifact(&job.id, &art).unwrap();
        art
    };
    let document = save(
        "variant.FCStd",
        b"new distinct controlled variant document",
        "FCStd",
    );
    let brep = save("solid.brep", b"opaque closed fixture variant BREP", "brep");
    save("solid.stl", b"opaque controlled fixture variant STL", "stl");
    let transform = spec.source.geometry.source_transform;
    let before = serde_json::json!({"bounds_m":spec.source.geometry.bounds_m,"volume_m3":spec.source.geometry.volume_m3,"transform":transform,"dimensions_m":spec.source.geometry.lengths_m()});
    let after = serde_json::json!({"bounds_m":spec.bounds_m().unwrap(),"volume_m3":6e-6,"transform":transform,"dimensions_m":[0.03,0.02,0.01]});
    let report = serde_json::json!({"schema_version":1,"approved_variant":spec,"source_preserved":true,"object_type":"Part::Box","gap_healing":false,"expressions":false,"before":before,"after":after,
        "document":{"path":"variant.FCStd","sha256":document.sha256,"bytes":document.bytes}});
    let mut snapshot: harbor_cad::cad::RegionSnapshot = serde_json::from_slice(
        &std::fs::read(store.job_dir(&id).unwrap().join("regions.json")).unwrap(),
    )
    .unwrap();
    snapshot.geometry_tolerance = spec.request.geometry_tolerance.clone();
    snapshot.regions[0].bounds_m = spec.bounds_m().unwrap();
    snapshot.regions[0].volume_m3 = 6e-6;
    save(
        "regions.json",
        &serde_json::to_vec(&snapshot).unwrap(),
        "json",
    );
    let manifest = serde_json::json!({"schema_version":1,"synthetic":true,"gap_healing":false,"regions":[{"region_name":"solid","path":"solid.brep","sha256":brep.sha256,"bytes":brep.bytes,"source_unit":"mm","scale_to_m":0.001,"bounds_m":snapshot.regions[0].bounds_m,"volume_m3":6e-6,"source_transform":transform,"placement_translation_unit":"mm"}]});
    save(
        "brep-manifest.json",
        &serde_json::to_vec(&manifest).unwrap(),
        "json",
    );
    save(
        "cad-variant-recompute.json",
        &serde_json::to_vec(&report).unwrap(),
        "json",
    );
    let receipt = serde_json::json!({"adapter":"FreeCAD","version":["1","1","4"],"executed":true,"backend":"cpu","software_fallback":false,"security_minimum":"1.1.4","sandbox_required":true,"import_policy":"harbor-cad-importer-v1"});
    save(
        "cad_inspect-receipt.json",
        &serde_json::to_vec(&receipt).unwrap(),
        "json",
    );
    store.finish(&job.id, 0, None).unwrap();
    let verified =
        || harbor_cad::cad_variant::verify_registered_outputs(&store, &job.id, &plan, &receipt);
    verified().unwrap();
    let record = harbor_cad::qualification::inspect(&store, &job.id).unwrap();
    assert!(matches!(
        record.capabilities[0].runtime_execution,
        harbor_cad::qualification::EvidenceState::NotObserved
    ));
    assert!(matches!(
        record.capabilities[0].numerical_verification,
        harbor_cad::qualification::EvidenceState::NotAssessed
    ));
    // Coherently replace test-only recorded reports to exercise semantic gates
    // beyond the independent byte-integrity check; real publication is immutable.
    let replace_fixture = |path: &str, data: &[u8]| {
        use sha2::{Digest, Sha256};
        let recorded: String = store
            .connection
            .query_row(
                "SELECT manifest FROM artifacts WHERE job=?1 AND path=?2",
                rusqlite::params![job.id, path],
                |row| row.get(0),
            )
            .unwrap();
        let mut artifact: ArtifactManifest = serde_json::from_str(&recorded).unwrap();
        std::fs::write(root.join(path), data).unwrap();
        artifact.sha256 = format!("{:x}", Sha256::digest(data));
        artifact.bytes = data.len() as u64;
        store
            .connection
            .execute(
                "UPDATE artifacts SET manifest=?1 WHERE job=?2 AND path=?3",
                rusqlite::params![serde_json::to_string(&artifact).unwrap(), job.id, path],
            )
            .unwrap();
    };
    for (key, value) in [
        ("source_preserved", serde_json::json!(false)),
        ("object_type", serde_json::json!("Part::Feature")),
        ("expressions", serde_json::json!(true)),
        ("after", before.clone()),
        ("before", after.clone()),
    ] {
        let mut changed = report.clone();
        changed[key] = value;
        replace_fixture(
            "cad-variant-recompute.json",
            &serde_json::to_vec(&changed).unwrap(),
        );
        assert!(verified().is_err(), "{key}");
    }
    replace_fixture(
        "cad-variant-recompute.json",
        &serde_json::to_vec(&report).unwrap(),
    );
    let mut changed = manifest.clone();
    changed["regions"][0]["placement_translation_unit"] = serde_json::json!("m");
    replace_fixture("brep-manifest.json", &serde_json::to_vec(&changed).unwrap());
    assert!(verified().is_err());
    replace_fixture(
        "brep-manifest.json",
        &serde_json::to_vec(&manifest).unwrap(),
    );
    for path in [
        "regions.json",
        "cad-variant-recompute.json",
        "variant.FCStd",
        "brep-manifest.json",
        "solid.brep",
        "solid.stl",
        "source-document.FCStd",
        "variant-source-regions.json",
        "cad-variant-source-execution.json",
    ] {
        let file = root.join(path);
        let original = std::fs::read(&file).unwrap();
        std::fs::write(&file, b"changed original bytes").unwrap();
        assert!(verified().is_err(), "{path}");
        std::fs::write(&file, original).unwrap();
    }
    verified().unwrap();
}

fn manufactured_box_triangles(points: [[f32; 3]; 8]) -> Vec<u8> {
    let mut data = vec![0u8; 80];
    data.extend(12u32.to_le_bytes());
    for (face, normal) in [
        ([0, 4, 7, 3], [-1f32, 0., 0.]),
        ([1, 2, 6, 5], [1., 0., 0.]),
        ([0, 1, 5, 4], [0., -1., 0.]),
        ([3, 7, 6, 2], [0., 1., 0.]),
        ([0, 3, 2, 1], [0., 0., -1.]),
        ([4, 5, 6, 7], [0., 0., 1.]),
    ] {
        for indices in [[face[0], face[1], face[2]], [face[0], face[2], face[3]]] {
            for value in normal
                .into_iter()
                .chain(indices.into_iter().flat_map(|i| points[i]))
            {
                data.extend(value.to_le_bytes());
            }
            data.extend(0u16.to_le_bytes());
        }
    }
    data
}

#[test]
fn far_original_scene_prepares_but_native_float32_plan_refuses_without_source_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = archived_source(&store);
    let data = manufactured_box_triangles([
        [100., -20., 300.],
        [120., -20., 300.],
        [120., -10., 300.],
        [100., -10., 300.],
        [100., -20., 310.],
        [120., -20., 310.],
        [120., -10., 310.],
        [100., -10., 310.],
    ]);
    let root = store.job_dir(&id).unwrap();
    let original = commit_artifact(
        &root,
        "solid.stl",
        &data,
        "stl",
        "manufactured far original; no native execution",
    )
    .unwrap();
    store.add_artifact(&id, &original).unwrap();
    let mut transport: harbor_cad::cad_transport::CadSpectralTransportRequest =
        serde_json::from_str(include_str!("../examples/cad-spectral-transport.json")).unwrap();
    transport.scene.source_job = id;
    harbor_cad::cad_spectral::prepare(&store, transport.scene.clone()).unwrap();
    let database = std::fs::read(store.root.join("jobs.sqlite3")).unwrap();
    assert!(
        harbor_cad::cad_transport::resolve(&store, transport)
            .unwrap_err()
            .to_string()
            .contains("Float32")
    );
    assert_eq!(std::fs::read(root.join("solid.stl")).unwrap(), data);
    assert_eq!(
        std::fs::read(store.root.join("jobs.sqlite3")).unwrap(),
        database
    );
}

#[test]
fn material_triangles_bind_every_original_region_and_refuse_substitution_without_mutating_source() {
    use harbor_cad::cad_spectral::{CadSpectralSceneRequest, prepare};
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    // Separate admissible synthetic placement; the original far placement has
    // its own unchanged-source refusal test. This does not rebase that source.
    let mut geometry = descriptor();
    geometry["bounds_m"] = serde_json::json!([0.01, 0.03, -0.002, 0.008, 0.03, 0.04]);
    geometry["source_transform"] = serde_json::json!([
        1., 0., 0., 10., 0., 1., 0., -2., 0., 0., 1., 30., 0., 0., 0., 1.
    ]);
    let id = archived_source_with_geometry(&store, geometry);
    let data = manufactured_box_triangles([
        [10., -2., 30.],
        [30., -2., 30.],
        [30., 8., 30.],
        [10., 8., 30.],
        [10., -2., 40.],
        [30., -2., 40.],
        [30., 8., 40.],
        [10., 8., 40.],
    ]);
    let root = store.job_dir(&id).unwrap();
    let original = commit_artifact(
        &root,
        "solid.stl",
        &data,
        "stl",
        "manufactured persisted geometry fixture; not native execution",
    )
    .unwrap();
    store.add_artifact(&id, &original).unwrap();
    let mut request: CadSpectralSceneRequest =
        serde_json::from_str(include_str!("../examples/cad-spectral-scene.json")).unwrap();
    request.source_job = id.clone();
    let database = std::fs::read(store.root.join("jobs.sqlite3")).unwrap();
    let prepared = prepare(&store, request.clone()).unwrap();
    let mut transport: harbor_cad::cad_transport::CadSpectralTransportRequest =
        serde_json::from_str(include_str!("../examples/cad-spectral-transport.json")).unwrap();
    transport.scene = request.clone();
    let bound = harbor_cad::cad_transport::resolve(&store, transport.clone()).unwrap();
    bound.validate().unwrap();
    let optical_plan =
        ExecutionPlan::cad_spectral_transport(bound.clone(), "research".into()).unwrap();
    assert_eq!(optical_plan.schema_version, 17);
    let raw = serde_json::to_value(&optical_plan).unwrap();
    assert_eq!(
        serde_json::from_value::<ExecutionPlan>(raw.clone())
            .unwrap()
            .id()
            .unwrap(),
        optical_plan.id().unwrap()
    );
    for version in 1..17 {
        let mut changed = raw.clone();
        changed["schema_version"] = serde_json::json!(version);
        assert!(serde_json::from_value::<ExecutionPlan>(changed).is_err());
    }
    let mut understated = optical_plan.clone();
    understated.observation.max_artifact_bytes = 1;
    assert!(understated.validate().is_err());
    // The scientific scene can be locally valid yet too large to carry its
    // complete approval and submission envelope within the wire allowance.
    let mut large = bound.clone();
    large.scene.limitations.clear();
    let overhead = serde_json::to_vec(&large).unwrap().len();
    large
        .scene
        .limitations
        .push("x".repeat(MAX_MESSAGE as usize - 3000 - overhead - 2));
    large.scene.scene_id.clear();
    large.scene.scene_id = digest(&large.scene).unwrap();
    large.validate().unwrap();
    assert!(serde_json::to_vec(&large).unwrap().len() < MAX_MESSAGE as usize);
    assert!(ExecutionPlan::cad_spectral_transport(large, "research".into()).is_err());
    let mut mixed = raw;
    mixed["spectral"] = serde_json::to_value(
        serde_json::from_str::<harbor_cad::radiation::SpectralReferenceSpec>(include_str!(
            "../examples/spectral-reference.json"
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(serde_json::from_value::<ExecutionPlan>(mixed).is_err());
    assert_eq!(bound.scene.scene_id, prepared.scene_id);
    assert_eq!(
        bound.native_request().unwrap()["scene"],
        serde_json::to_value(&prepared).unwrap()
    );
    assert_eq!(
        bound.native_request().unwrap()["scene"]["ageing_readiness"],
        "missing_inputs"
    );
    for key in ["scene_id", "transport_readiness", "physical_validation"] {
        let mut value = serde_json::to_value(&bound).unwrap();
        value["scene"][key] = serde_json::json!("changed");
        let changed: harbor_cad::cad_transport::CadSpectralTransportSpec =
            serde_json::from_value(value).unwrap();
        assert!(changed.validate().is_err());
    }
    let mut changed = bound.clone();
    changed.request.source_provenance = "different source approval".into();
    changed.validate().unwrap();
    assert_ne!(digest(&bound).unwrap(), digest(&changed).unwrap());
    assert!(!prepared.executed);
    assert_eq!(prepared.physical_validation, "unqualified");
    assert_eq!(prepared.transport_readiness, "prepared_not_executed");
    assert_eq!(prepared.ageing_readiness, "missing_inputs");
    assert_eq!(prepared.regions[0].geometry.triangles, 12);
    assert_eq!(
        prepared.regions[0].original_triangles.sha256,
        original.sha256
    );
    assert_eq!(
        prepared.regions[0].source.execution_id,
        store.plan(&id).unwrap().id().unwrap()
    );
    assert_eq!(
        prepare(&store, request.clone()).unwrap().scene_id,
        prepared.scene_id
    );
    assert_eq!(
        std::fs::read(store.root.join("jobs.sqlite3")).unwrap(),
        database
    );
    // The separately approved optical job retains its own durable originals;
    // changing the importer artifacts later cannot change acknowledged inputs.
    let profile = store.job_profile(&id).unwrap();
    let optical_job = store
        .submit_with_profile(&optical_plan, "original-optical", &profile)
        .unwrap();
    let pending = harbor_cad::qualification::inspect(&store, &optical_job.id).unwrap();
    assert_eq!(
        pending.capabilities[0].formulation,
        "opaque_lambertian_direct_only"
    );
    assert_eq!(pending.capabilities[0].refinement, 64);
    assert!(matches!(
        pending.capabilities[0].runtime_execution,
        harbor_cad::qualification::EvidenceState::NotObserved
    ));
    assert!(matches!(
        harbor_cad::cad_transport_results::inspect(
            &store,
            harbor_cad::cad_transport_results::CadOpticalResultsRequest {
                schema_version: 1,
                job_id: optical_job.id.clone(),
                seed: 17,
                region_name: "solid".into()
            }
        ),
        Err(harbor_cad::Error::Unqualified(_))
    ));
    let copied =
        harbor_cad::cad_transport::registered(&store, &optical_job.id, &optical_plan).unwrap();
    use std::os::unix::fs::MetadataExt;
    assert_ne!(
        std::fs::metadata(copied.join("solid.stl")).unwrap().ino(),
        std::fs::metadata(root.join("solid.stl")).unwrap().ino()
    );
    assert_eq!(std::fs::read(copied.join("solid.stl")).unwrap(), data);
    std::fs::write(root.join("solid.stl"), b"original source later changed").unwrap();
    harbor_cad::cad_transport::registered(&store, &optical_job.id, &optical_plan).unwrap();
    optical_packet_fixture(&bound, &data);
    std::fs::write(root.join("solid.stl"), &data).unwrap();
    // Isolated storage-only foreground bindings exercise interrupted admission;
    // these fixtures do not establish native execution or a systemd guarantee.
    let mut staging_profile = profile.clone();
    staging_profile.service_mode = "foreground".into();
    let staging_binding = ExecutionBinding::capture(
        &optical_plan,
        &staging_profile,
        &std::env::current_exe().unwrap(),
        std::collections::BTreeMap::new(),
    )
    .unwrap();
    let staging_authority = store
        .execution_authorization(&id)
        .unwrap()
        .unwrap()
        .authority;
    let staging_auth = ExecutionAuthorization::capture(
        &optical_plan,
        &staging_profile,
        &staging_binding,
        &staging_authority,
    )
    .unwrap();
    let intact = store
        .submit_authorized(
            &optical_plan,
            "optical-intact-orphan",
            &staging_profile,
            &staging_binding,
            &staging_auth,
        )
        .unwrap();
    store
        .connection
        .execute("DELETE FROM jobs WHERE id=?1", [&intact.id])
        .unwrap();
    store.cleanup_retention(|_| Ok(false)).unwrap();
    assert!(!store.root.join("artifacts").join(&intact.id).exists());
    let lost = store
        .submit_authorized(
            &optical_plan,
            "optical-lost-source-orphan",
            &staging_profile,
            &staging_binding,
            &staging_auth,
        )
        .unwrap();
    store
        .connection
        .execute("DELETE FROM jobs WHERE id=?1", [&lost.id])
        .unwrap();
    std::fs::write(
        root.join("solid.stl"),
        b"source unavailable after interrupted publication",
    )
    .unwrap();
    store.cleanup_retention(|_| Ok(false)).unwrap();
    assert_eq!(
        std::fs::read(
            store
                .root
                .join("artifacts")
                .join(&lost.id)
                .join("source-cad/solid.stl")
        )
        .unwrap(),
        data
    );
    std::fs::write(root.join("solid.stl"), &data).unwrap();
    store.cleanup_retention(|_| Ok(false)).unwrap();
    // Once unverifiable input has been quarantined, its runtime roots are
    // released and later cleanup does not rediscover or delete its artifacts.
    // Restoring the source permits new submissions, not retrospective deletion.
    harbor_cad::cad_transport::source(&store, &bound).unwrap();
    assert!(store.root.join("artifacts").join(&lost.id).exists());
    assert!(!store.root.join("retentions").join(&lost.id).exists());
    std::fs::write(root.join("solid.stl"), b"original source later changed").unwrap();
    assert_eq!(
        store
            .submit_with_profile(&optical_plan, "original-optical", &profile)
            .unwrap()
            .id,
        optical_job.id
    );
    assert!(
        store
            .submit_with_profile(&optical_plan, "new-changed-source", &profile)
            .is_err()
    );
    std::fs::write(root.join("solid.stl"), &data).unwrap();
    std::fs::write(copied.join("solid.stl"), b"acknowledged copy changed").unwrap();
    assert!(harbor_cad::cad_transport::registered(&store, &optical_job.id, &optical_plan).is_err());
    std::fs::write(copied.join("solid.stl"), &data).unwrap();
    harbor_cad::cad_transport::registered(&store, &optical_job.id, &optical_plan).unwrap();
    let mut missing = request.clone();
    missing.materials[0].response = harbor_cad::materials::PhysicalInput::Missing {
        reason: "UV data unavailable".into(),
    };
    let missing = prepare(&store, missing).unwrap();
    assert_eq!(missing.transport_readiness, "missing_optical_inputs");
    assert_ne!(prepared.scene_id, missing.scene_id);
    let mut changed = request.clone();
    changed.assignments[0].region_name = "foreign".into();
    assert!(prepare(&store, changed).is_err());
    std::fs::write(root.join("solid.stl"), b"changed original triangles").unwrap();
    assert!(prepare(&store, request.clone()).is_err());
    assert!(harbor_cad::cad_transport::resolve(&store, transport).is_err());
    std::fs::write(root.join("solid.stl"), &data).unwrap();
    let mut artifact = original.clone();
    artifact.units = Some("m".into());
    store
        .connection
        .execute(
            "UPDATE artifacts SET manifest=?1 WHERE job=?2 AND path='solid.stl'",
            rusqlite::params![serde_json::to_string(&artifact).unwrap(), id],
        )
        .unwrap();
    assert!(prepare(&store, request.clone()).is_err());
    store
        .connection
        .execute(
            "UPDATE artifacts SET manifest=?1 WHERE job=?2 AND path='solid.stl'",
            rusqlite::params![serde_json::to_string(&original).unwrap(), id],
        )
        .unwrap();
    prepare(&store, request).unwrap();
}

fn optical_packet_fixture(
    approved: &harbor_cad::cad_transport::CadSpectralTransportSpec,
    data: &[u8],
) {
    // Manufactured original-point replay only. These constructed packets claim
    // no native execution; native/worker evidence comes from guarded campaigns.
    use harbor_cad::cad_transport_fields::verify_numerical;
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let output = temp.path().join("output");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&output).unwrap();
    let mut spec = approved.clone();
    let mut data = data.to_vec();
    // Move the manufactured original down by 250 mm, with the source context
    // updated explicitly so its Float32 metre storage meets the same budget.
    for facet in data[84..].as_chunks_mut::<50>().0 {
        for vertex in 0..3 {
            let at = 20 + vertex * 12;
            let z = f32::from_le_bytes(facet[at..at + 4].try_into().unwrap());
            facet[at..at + 4].copy_from_slice(&(z - 250.).to_le_bytes());
        }
    }
    let region = &mut spec.scene.regions[0];
    for axis in [4, 5] {
        region.source.geometry.bounds_m[axis] -= 0.25;
    }
    region.source.geometry.source_transform[11] -= 250.;
    region.original_triangles.sha256 = format!("{:x}", Sha256::digest(&data));
    let mesh =
        harbor_cad::cad_triangles::verify_binary_box(&data, &region.source.geometry).unwrap();
    region.geometry = mesh.assessment;
    std::fs::write(source.join("solid.stl"), &data).unwrap();
    spec.scene.scene_id.clear();
    spec.scene.scene_id = digest(&spec.scene).unwrap();
    spec.validate().unwrap();
    let header = "region,facet,sample,knot_offset,x_m,y_m,z_m,normal_x,normal_y,normal_z,towards_source_x,towards_source_y,towards_source_z,native_cosine,native_pdf,native_weight_w_m2_nm_0,native_weight_w_m2_nm_1,native_weight_w_m2_nm_2,native_weight_w_m2_nm_3,native_reflectance_0,native_reflectance_1,native_reflectance_2,native_reflectance_3\n";
    let mut csv = header.to_owned();
    let mut conversions = Vec::new();
    let mut results = Vec::new();
    for (index, facet) in mesh.triangles.iter().enumerate() {
        let native = facet.vertices_m.map(|p| p.map(|v| f64::from(v as f32)));
        let rounding = facet
            .vertices_m
            .into_iter()
            .flatten()
            .zip(native.into_iter().flatten())
            .map(|(a, b)| (a - b).abs())
            .fold(0_f64, f64::max);
        let a: [f64; 3] = std::array::from_fn(|i| native[1][i] - native[0][i]);
        let b: [f64; 3] = std::array::from_fn(|i| native[2][i] - native[0][i]);
        let cross = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let area = cross.iter().map(|v| v * v).sum::<f64>().sqrt() / 2.;
        conversions.push(serde_json::json!({"region":"solid","facet":index,"original_area_m2":facet.area_m2,"native_area_m2":area,"native_area_relative_error":(area/facet.area_m2-1.).abs(),"maximum_vertex_rounding_error_m":rounding,"native_vertices_m":native,"original_sha256":spec.scene.regions[0].original_triangles.sha256}));
        let point: [f64; 3] = std::array::from_fn(|axis| {
            f64::from(((native[0][axis] + native[1][axis] + native[2][axis]) / 3.) as f32)
        });
        let cosine = if facet.boundary == 5 { 1. } else { 0. };
        for sample in 0..spec.request.samples_per_triangle {
            let values = [
                point[0],
                point[1],
                point[2],
                facet.normal[0],
                facet.normal[1],
                facet.normal[2],
                0.,
                0.,
                1.,
                cosine,
                1.,
                1.,
                2.,
                2.,
                2.,
                f64::from(0.2f32),
                f64::from(0.4f32),
                f64::from(0.4f32),
                f64::from(0.4f32),
            ];
            csv.push_str(&format!(
                "solid,{index},{sample},0,{}\n",
                values.map(|v| v.to_string()).join(",")
            ));
        }
        let channels = serde_json::json!({"incident":150.*cosine,"absorbed":(310./3.)*cosine,"reflected_outgoing":(140./3.)*cosine});
        let scaled = |scale: f64| {
            serde_json::Value::Object(
                channels
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(name, v)| (name.clone(), serde_json::json!(v.as_f64().unwrap() * scale)))
                    .collect(),
            )
        };
        results.push(serde_json::json!({"region":"solid","material":"synthetic_opaque","facet":index,"geometric_boundary":facet.boundary,"original_area_m2":facet.area_m2,"samples":64,"mean_spectral_irradiance_w_m2_nm":[cosine,2.*cosine],"channels_w_m2":channels,"reference_channels_w_m2":channels,"relative_errors":{"incident":0.,"absorbed":0.,"reflected_outgoing":0.},"power_w":scaled(facet.area_m2),"dose_j_m2":scaled(7200.),"energy_j":scaled(7200.*facet.area_m2),"ageing_status":"missing_inputs"}));
    }
    let observations=spec.request.seeds.map(|seed|{let path=format!("triangles-{seed}.csv");std::fs::write(output.join(&path),&csv).unwrap();serde_json::json!({"seed":seed,"original":{"path":path,"sha256":format!("{:x}",Sha256::digest(csv.as_bytes())),"bytes":csv.len()},"facets":results})});
    let receipt = serde_json::json!({"schema_version":1,"adapter":"Mitsuba","versions":{"mitsuba":"3.9.1","drjit":"1.5.0"},"backend":"cpu","variant":"scalar_spectral","precision":"Float32","reduction_precision":"Float64_compensated","formulation":"opaque_lambertian_direct_only","input":spec.native_request().unwrap(),"geometry_conversions":conversions,"observations":observations,"history_integral_s":7200.,"physical_validation":"unqualified","sampling_convergence":"not_assessed","interreflection":"excluded_by_explicit_direct_only_model"});
    let evidence = verify_numerical(&spec, &source, &output, &receipt).unwrap();
    assert!(evidence.error < 1e-12);
    assert!(
        harbor_cad::cad_transport_fields::verify_receipt(&spec, &source, &output, &receipt)
            .is_err()
    );
    for pointer in [
        "/observations/0/facets/0/original_area_m2",
        "/observations/0/facets/10/channels_w_m2/incident",
        "/observations/0/facets/10/energy_j/absorbed",
        "/geometry_conversions/0/native_vertices_m/0/0",
    ] {
        let mut changed = receipt.clone();
        *changed.pointer_mut(pointer).unwrap() = serde_json::json!(999.);
        assert!(
            verify_numerical(&spec, &source, &output, &changed).is_err(),
            "{pointer}"
        );
    }
    for field in [
        "precision",
        "physical_validation",
        "interreflection",
        "sampling_convergence",
    ] {
        let mut changed = receipt.clone();
        changed[field] = serde_json::json!("invented_pass");
        assert!(verify_numerical(&spec, &source, &output, &changed).is_err());
    }
    // Even an internally rehashed original with all reported reductions retained
    // cannot substitute its position, direction, association or spectral weight.
    for column in [0, 1, 2, 3, 4, 7, 10, 13, 14, 15, 19] {
        let mut lines = csv.lines().map(str::to_owned).collect::<Vec<_>>();
        let mut row = lines[1].split(',').map(str::to_owned).collect::<Vec<_>>();
        row[column] = "99".into();
        lines[1] = row.join(",");
        let changed_csv = lines.join("\n") + "\n";
        let mut changed = receipt.clone();
        changed["observations"][0]["original"]["bytes"] = serde_json::json!(changed_csv.len());
        changed["observations"][0]["original"]["sha256"] =
            serde_json::json!(format!("{:x}", Sha256::digest(changed_csv.as_bytes())));
        std::fs::write(output.join("triangles-17.csv"), changed_csv).unwrap();
        assert!(
            verify_numerical(&spec, &source, &output, &changed).is_err(),
            "column {column}"
        );
    }
    std::fs::write(output.join("triangles-17.csv"), csv).unwrap();
    verify_numerical(&spec, &source, &output, &receipt).unwrap();
}

#[test]
fn imported_static_fem_plan_binds_registered_geometry_and_explicit_material_boundary_provenance() {
    use harbor_cad::fem_imported::ImportedFemSpec;
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = archived_source(&store);
    let (_, source) = harbor_cad::cad_source::source(&store, &id, "solid", 4, 1e-6).unwrap();
    let spec:ImportedFemSpec=serde_json::from_value(serde_json::json!({"schema_version":1,"reference":{
        "schema_version":1,"synthetic":true,"backend":"cpu","mode":"thermal_boundary","size_m":[0.02,0.01,0.01],"resolution":4,"geometry_tolerance_m":1e-6,
        "temperatures_k":[293.15,303.15],"numerical_tolerance":1e-6,"conductivity_w_m_k":20.},"material_provenance":"controlled synthetic constant conductivity", "boundary_provenance":"prescribed world XMIN/XMAX temperatures; transverse adiabatic"})).unwrap();
    let plan =
        ExecutionPlan::fem_imported(source.clone(), spec.clone(), "research".into()).unwrap();
    let raw = serde_json::to_value(&plan).unwrap();
    assert_eq!(raw["schema_version"], 8);
    assert_eq!(raw["stages"][0]["operation"], "fem_imported");
    assert!(raw.get("case").is_none() && raw.get("fem").is_none() && raw.get("thermal").is_none());
    assert!(plan.observation.retained_times_s.is_empty());
    assert_eq!(
        plan.id().unwrap(),
        serde_json::from_value::<ExecutionPlan>(raw.clone())
            .unwrap()
            .id()
            .unwrap()
    );
    for version in 1..=7 {
        let mut old = raw.clone();
        old["schema_version"] = serde_json::json!(version);
        assert!(serde_json::from_value::<ExecutionPlan>(old).is_err());
    }
    let mut wrong = spec.clone();
    wrong.reference.size_m[0] = 0.021;
    assert!(ExecutionPlan::fem_imported(source.clone(), wrong, "research".into()).is_err());
    let mut missing = spec.clone();
    missing.material_provenance.clear();
    assert!(ExecutionPlan::fem_imported(source.clone(), missing, "research".into()).is_err());
    let mut wrong = source.clone();
    wrong.geometry.synthetic = false;
    assert!(ExecutionPlan::fem_imported(wrong, spec.clone(), "research".into()).is_err());
    let mut changed = spec;
    changed.reference.conductivity_w_m_k = Some(21.);
    let changed = ExecutionPlan::fem_imported(source, changed, "research".into()).unwrap();
    assert_ne!(plan.science_id().unwrap(), changed.science_id().unwrap());
}

#[test]
fn imported_fem_receipt_checks_native_world_origin_schema_mesh_and_raw_field_binding() {
    use harbor_cad::fem_imported::{ImportedFemSpec, descriptor, verify_receipt};
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = archived_source(&store);
    let (_, source) = harbor_cad::cad_source::source(&store, &id, "solid", 4, 1e-6).unwrap();
    let spec:ImportedFemSpec=serde_json::from_value(serde_json::json!({"schema_version":1,"reference":{
        "schema_version":1,"synthetic":true,"backend":"cpu","mode":"thermal_boundary","size_m":[0.02,0.01,0.01],"resolution":4,"geometry_tolerance_m":1e-6,
        "temperatures_k":[293.15,303.15],"numerical_tolerance":1e-6,"conductivity_w_m_k":20.},"material_provenance":"controlled synthetic constant conductivity", "boundary_provenance":"prescribed world planes"})).unwrap();
    let plan = ExecutionPlan::fem_imported(source, spec, "research".into()).unwrap();
    let geometry = &plan.cad_source.as_ref().unwrap().geometry;
    let root = temp.path().join("native");
    std::fs::create_dir(&root).unwrap();
    let mesh = commit_artifact(
        &root,
        "mesh.json",
        &serde_json::to_vec(&cartesian_mesh(geometry)).unwrap(),
        "json",
        "binding fixture",
    )
    .unwrap();
    let raw = commit_artifact(
        &root,
        "reference.dat",
        b"retained raw static reference output",
        "dat",
        "binding fixture, not numerical qualification",
    )
    .unwrap();
    // Keys and formulation are those emitted by the imported adapter, which
    // differ from the historical synthetic/static receipt envelope.
    let mut receipt = serde_json::json!({"schema_version":1,"adapter":"CalculiX","backend":"cpu","factorization":"SPOOLES","executed":true,
        "software_fallback":false,"synthetic":true,"precision":"float64","request_sha256":digest(&descriptor(&plan).unwrap()).unwrap(),"formulation":"thermal_boundary",
        "calculix_version":"2.23","gmsh_version":"4.15.2","calculix_source_sha256":"9c88385c10fb04f5dc6c4e98027a51bebdd8aee3920e05190d6c1dd08357d6e7",
        "gmsh_source_sha256":"be3f66f225d27ba9fa014f07e83169285da8a051b0e8ab7103d88066b39bdd3e","brep_sha256":geometry.brep_sha256,
        "region_name":geometry.region_name,"geometry_provenance":geometry.geometry_provenance,"source_unit":"mm","coordinate_unit":"m","scale_to_m":0.001,
        "placement_translation_unit":"mm","source_transform":geometry.source_transform,"world_bounds_m":geometry.bounds_m,
        "world_origin_m":[geometry.bounds_m[0],geometry.bounds_m[2],geometry.bounds_m[4]],"gap_healing":false,
        "material_provenance":"controlled synthetic constant conductivity","boundary_provenance":"prescribed world planes","mesh_sha256":mesh.sha256,"native_field_sha256":raw.sha256,
        "nodes":125,"elements":64,"physical_validation":"unqualified"});
    receipt["sandbox"] = serde_json::json!({"policy":"harbor-cad-fem-imported-cpu-v1","checks":{"operation_closure_only":true,"no_gpu_nodes":true,"no_sysfs":true,"no_host_home":true,"no_session_bus":true,"no_worker_socket":true,"network_namespace_isolated":true,"descriptor_readonly":true,"source_brep_readonly":true,"named_source_only":true}});
    receipt["numerical_verification"] = serde_json::json!({"temperature":{"passed":true,"normalized_max_abs_error":0.,"tolerance":1e-6,"reference":"linear temperature about explicit CAD XMIN","unit":"K","samples":125},
            "heat_flux":{"passed":true,"normalized_max_abs_error":1e-12,"tolerance":1e-6,"reference":"Fourier flux","unit":"W/m2","samples":512}});
    assert_eq!(verify_receipt(&plan, &root, &receipt).unwrap().error, 1e-12);
    for (key, value) in [
        ("world_origin_m", serde_json::json!([0., 0., 0.])),
        (
            "formulation",
            serde_json::json!("imported_thermal_boundary"),
        ),
        ("mesh_sha256", serde_json::json!("b".repeat(64))),
        ("material_provenance", serde_json::json!("")),
        ("request_sha256", serde_json::json!("b".repeat(64))),
    ] {
        let mut changed = receipt.clone();
        changed[key] = value;
        assert!(verify_receipt(&plan, &root, &changed).is_err(), "{key}");
    }
    std::fs::write(
        root.join("reference.dat"),
        b"raw bytes corrupted after receipt",
    )
    .unwrap();
    assert!(verify_receipt(&plan, &root, &receipt).is_err());
}

fn submit_mesh(
    store: &Store,
    source: harbor_cad::cad_source::CadSource,
    key: &str,
) -> harbor_cad::storage::Job {
    // Storage-only foreground fixture. Native execution still requires exact
    // packages, authoritative admission and systemd in the worker.
    let plan = ExecutionPlan::cad_mesh(source.clone(), "research".into()).unwrap();
    let mut profile = store.job_profile(&source.job_id).unwrap();
    profile.service_mode = "foreground".into();
    let binding = ExecutionBinding::capture(
        &plan,
        &profile,
        &std::env::current_exe().unwrap(),
        std::collections::BTreeMap::new(),
    )
    .unwrap();
    let authority = HostAuthority {
        schema_version: 1,
        fleetix_revision: FLEETIX_REV.into(),
        fleetix_contract_digest: fleetix_digest(),
        max_ram_bytes: profile.max_ram_bytes,
        ram_headroom_bytes: 0,
        filesystems: vec![],
        cards: vec![],
        routes: vec![],
        allowed_devices: vec![],
        overrides: vec![],
        native_runtimes: vec![],
        allowed_input_roots: vec![profile.allowed_input_root.clone()],
        kfd_exclusion: None,
    };
    let authorization =
        ExecutionAuthorization::capture(&plan, &profile, &binding, &authority).unwrap();
    store
        .submit_authorized(&plan, key, &profile, &binding, &authorization)
        .unwrap()
}

#[test]
fn immutable_cad_retention_uses_distinct_inodes_survives_original_mutation_and_rejects_local_drift()
{
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = archived_source(&store);
    let (original, source) = harbor_cad::cad_source::source(&store, &id, "solid", 4, 1e-6).unwrap();
    let job = submit_mesh(&store, source.clone(), "bound-mesh");
    let plan = store.plan(&job.id).unwrap();
    let retained = harbor_cad::cad_source::registered(&store, &job.id, &plan).unwrap();
    assert_ne!(
        std::fs::metadata(original.join("solid.brep"))
            .unwrap()
            .ino(),
        std::fs::metadata(retained.join("solid.brep"))
            .unwrap()
            .ino()
    );
    assert_eq!(submit_mesh(&store, source, "bound-mesh").id, job.id);
    std::fs::write(
        original.join("solid.brep"),
        b"changed original after dependent submission",
    )
    .unwrap();
    harbor_cad::cad_source::registered(&store, &job.id, &plan).unwrap();
    std::fs::write(retained.join("solid.brep"), b"changed retained input").unwrap();
    assert!(harbor_cad::cad_source::registered(&store, &job.id, &plan).is_err());
}

#[test]
fn interrupted_cad_staging_cleans_only_when_original_authorized_source_is_intact() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = archived_source(&store);
    let (original, source) = harbor_cad::cad_source::source(&store, &id, "solid", 4, 1e-6).unwrap();
    let first = submit_mesh(&store, source.clone(), "lost-insert-1");
    store
        .connection
        .execute("DELETE FROM jobs WHERE id=?1", [&first.id])
        .unwrap();
    store.cleanup_retention(|_| Ok(false)).unwrap();
    assert!(!store.root.join("artifacts").join(first.id).exists());
    let second = submit_mesh(&store, source, "lost-insert-2");
    store
        .connection
        .execute("DELETE FROM jobs WHERE id=?1", [&second.id])
        .unwrap();
    std::fs::write(
        original.join("solid.brep"),
        b"original damaged after staging",
    )
    .unwrap();
    store.cleanup_retention(|_| Ok(false)).unwrap();
    assert!(
        store
            .root
            .join("artifacts")
            .join(second.id)
            .join("retained-cad/solid.brep")
            .exists()
    );
}

fn cartesian_mesh(spec: &CadMeshDescriptor) -> serde_json::Value {
    let n = spec.resolution;
    let node = |i: u32, j: u32, k: u32| 1 + u64::from(i + (n + 1) * j + (n + 1).pow(2) * k);
    let widths = spec.lengths_m();
    let mut nodes = serde_json::Map::new();
    let mut elements = serde_json::Map::new();
    let mut sets: std::collections::BTreeMap<String, Vec<u64>> = std::collections::BTreeMap::new();
    for k in 0..=n {
        for j in 0..=n {
            for i in 0..=n {
                let ijk = [i, j, k];
                let xyz: [f64; 3] = std::array::from_fn(|axis| {
                    spec.bounds_m[2 * axis] + widths[axis] * f64::from(ijk[axis]) / f64::from(n)
                });
                nodes.insert(node(i, j, k).to_string(), serde_json::json!(xyz));
                for (axis, name) in ["x", "y", "z"].into_iter().enumerate() {
                    for (suffix, index) in [("min", 0), ("max", n)] {
                        if ijk[axis] == index {
                            sets.entry(format!("{name}{suffix}"))
                                .or_default()
                                .push(node(i, j, k));
                        }
                    }
                }
            }
        }
    }
    for k in 0..n {
        for j in 0..n {
            for i in 0..n {
                elements.insert(
                    (elements.len() + 1).to_string(),
                    serde_json::json!([
                        node(i, j, k),
                        node(i + 1, j, k),
                        node(i + 1, j + 1, k),
                        node(i, j + 1, k),
                        node(i, j, k + 1),
                        node(i + 1, j, k + 1),
                        node(i + 1, j + 1, k + 1),
                        node(i, j + 1, k + 1)
                    ]),
                );
            }
        }
    }
    serde_json::json!({"schema_version":1,"synthetic":spec.synthetic,"coordinate_unit":"m","nodes":nodes,"elements":elements,"element_type":"C3D8","boundary_node_sets":sets,
        "semantic_face_selection":"planar bounding box plus area; no ordinal face dependency","positive_gauss_jacobians":true,"integrated_volume_m3":spec.volume_m3})
}

#[test]
fn independent_imported_mesh_gate_rejects_translated_inverted_duplicate_or_wrong_plane_meshes() {
    let spec: CadMeshDescriptor = serde_json::from_value(descriptor()).unwrap();
    let mesh = cartesian_mesh(&spec);
    assert!(harbor_cad::cad_mesh::verify_mesh(&spec, mesh.clone()).unwrap() <= 1e-10);
    for key in [
        "translated",
        "duplicate-node",
        "duplicate-cell",
        "inverted",
        "warped",
        "plane",
        "volume",
        "units",
        "gap-healing",
    ] {
        let mut changed = mesh.clone();
        match key {
            "translated" => changed["nodes"]["1"][0] = serde_json::json!(0.2),
            "duplicate-node" => changed["nodes"]["2"] = changed["nodes"]["1"].clone(),
            "duplicate-cell" => changed["elements"]["2"] = changed["elements"]["1"].clone(),
            "inverted" => changed["elements"]["1"].as_array_mut().unwrap().swap(1, 3),
            "warped" => changed["elements"]["1"].as_array_mut().unwrap().swap(2, 6),
            "plane" => changed["boundary_node_sets"]["xmin"][0] = serde_json::json!(2),
            "volume" => changed["integrated_volume_m3"] = serde_json::json!(4e-6),
            "units" => changed["coordinate_unit"] = serde_json::json!("mm"),
            _ => changed["gap_healing"] = serde_json::json!(true),
        }
        assert!(
            harbor_cad::cad_mesh::verify_mesh(&spec, changed).is_err(),
            "{key}"
        );
    }
}

fn sheet_source(store: &Store) -> String {
    // Persisted alias evidence only; no native spreadsheet was opened.
    let id = archived_source(store);
    let evidence = serde_json::json!({"schema_version":1,"sheets":[{"name":"Params","label":"Params","aliases":{
        "length":{"cell":"B1","dimension":"length","value_si":0.02},
        "fin_count":{"cell":"B2","dimension":"dimensionless","value_si":8.0},
        "angle":{"cell":"B3","dimension":"other","value_si":null}}}]});
    let root = store.job_dir(&id).unwrap();
    let record = commit_artifact(
        &root,
        "sheets.json",
        &serde_json::to_vec(&evidence).unwrap(),
        "json",
        "persisted-record fixture",
    )
    .unwrap();
    store.add_artifact(&id, &record).unwrap();
    id
}

fn sheet_request(id: &str) -> serde_json::Value {
    serde_json::json!({"schema_version":2,"source_job":id,"sheet_name":"Params",
        "parameters":{"length":{"value":30.,"unit":"mm"},"fin_count":{"value":6.,"unit":"1"}},
        "regions":["solid"],"geometry_tolerance":{"value":1e-6,"unit":"m"},
        "provenance":"explicit spreadsheet copy fixture; no physical claim"})
}

fn plan_sheet(store: &Store, request: serde_json::Value) -> harbor_cad::Result<ExecutionPlan> {
    match serde_json::from_value(request)? {
        harbor_cad::cad_sheet_variant::AnyCadVariantRequest::Sheet(request) => {
            harbor_cad::cad_sheet_variant::plan(store, request, "research".into())
        }
        harbor_cad::cad_sheet_variant::AnyCadVariantRequest::Box(_) => {
            panic!("version-2 request decoded as a box edit")
        }
    }
}

/// Replace (Some) or remove (None) the object member a JSON pointer names.
fn mutate(value: &mut serde_json::Value, pointer: &str, replacement: Option<serde_json::Value>) {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    let object = value.pointer_mut(parent).unwrap().as_object_mut().unwrap();
    match replacement {
        Some(v) => object.insert(key.into(), v),
        None => object.remove(key),
    };
}

#[test]
fn sheet_variant_binds_registered_aliases_and_refuses_unknown_bad_or_excess_parameters() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = sheet_source(&store);
    let plan = plan_sheet(&store, sheet_request(&id)).unwrap();
    let spec = plan.cad_sheet_variant.as_ref().unwrap();
    assert_eq!(plan.schema_version, 16);
    assert!(plan.cad_variant.is_none());
    assert_eq!(plan.channel_case().unwrap().regions, ["solid"]);
    assert_eq!(
        spec.source.regions[0].bounds_m,
        [0.1, 0.12, -0.02, -0.01, 0.3, 0.31]
    );
    assert_eq!(spec.document.path, "input.FCStd");
    let raw = serde_json::to_value(&plan).unwrap();
    assert!(raw.get("cad_variant").is_none());
    assert_eq!(
        serde_json::from_value::<ExecutionPlan>(raw.clone())
            .unwrap()
            .id()
            .unwrap(),
        plan.id().unwrap()
    );
    for version in [1, 15, 17, 18] {
        let mut old = raw.clone();
        old["schema_version"] = serde_json::json!(version);
        assert!(
            serde_json::from_value::<ExecutionPlan>(old).is_err(),
            "{version}"
        );
    }
    let boxed = serde_json::json!({"schema_version":1,"source_job":id,"region_name":"solid",
        "dimensions":[{"value":30.,"unit":"mm"},{"value":20.,"unit":"mm"},{"value":10.,"unit":"mm"}],
        "geometry_tolerance":{"value":1e-6,"unit":"m"},"provenance":"box"});
    let mut both = plan.clone();
    both.cad_variant = harbor_cad::cad_variant::plan(
        &store,
        serde_json::from_value(boxed).unwrap(),
        "research".into(),
    )
    .unwrap()
    .cad_variant;
    assert!(both.validate().is_err());
    assert!(serde_json::from_value::<ExecutionPlan>(serde_json::to_value(&both).unwrap()).is_err());
    let mut changed = plan.clone();
    let parameters = &mut changed
        .cad_sheet_variant
        .as_mut()
        .unwrap()
        .request
        .parameters;
    parameters.get_mut("length").unwrap().value = 31.;
    changed.validate().unwrap();
    assert_ne!(changed.science_id().unwrap(), plan.science_id().unwrap());
    let mut changed = plan.clone();
    changed.cad_sheet_variant.as_mut().unwrap().request.regions = vec!["other".into()];
    assert!(changed.validate().is_err());
    assert!(store.submit(&plan, "unbound-sheet-variant").is_err());
    // An empty parameter set re-exports the baseline with its metrics.
    let mut baseline = sheet_request(&id);
    baseline["parameters"] = serde_json::json!({});
    plan_sheet(&store, baseline).unwrap();

    let many: serde_json::Map<_, _> = (0..33)
        .map(|i| (format!("a{i}"), serde_json::json!({"value":1.,"unit":"mm"})))
        .collect();
    for (key, value) in [
        (
            "parameters",
            serde_json::json!({"missing":{"value":1.,"unit":"mm"}}),
        ),
        ("parameters", serde_json::Value::Object(many)),
        (
            "parameters",
            serde_json::json!({"length":{"value":1.,"unit":"kg"}}),
        ),
        (
            "parameters",
            serde_json::json!({"length":{"value":2.,"unit":"1"}}),
        ),
        (
            "parameters",
            serde_json::json!({"fin_count":{"value":2.,"unit":"mm"}}),
        ),
        (
            "parameters",
            serde_json::json!({"angle":{"value":2.,"unit":"mm"}}),
        ),
        (
            "parameters",
            serde_json::json!({"length":{"value":20.,"unit":"m"}}),
        ),
        (
            "parameters",
            serde_json::json!({"length":{"value":1.,"unit":"nm"}}),
        ),
        (
            "parameters",
            serde_json::json!({"fin_count":{"value":-1.,"unit":"1"}}),
        ),
        (
            "parameters",
            serde_json::json!({"Params.length":{"value":1.,"unit":"mm"}}),
        ),
        ("sheet_name", serde_json::json!("Other")),
        ("sheet_name", serde_json::json!("=Params")),
        ("regions", serde_json::json!(["missing"])),
        ("regions", serde_json::json!(["solid", "solid"])),
        ("regions", serde_json::json!([])),
        (
            "geometry_tolerance",
            serde_json::json!({"value":1e-10,"unit":"m"}),
        ),
        ("provenance", serde_json::json!(" ")),
        ("expression", serde_json::json!("Params.length * 2")),
    ] {
        let mut request = sheet_request(&id);
        request[key] = value;
        assert!(plan_sheet(&store, request).is_err(), "{key}");
    }
    // A source without registered spreadsheet evidence cannot be edited.
    let other = Store::open(&temp.path().join("plain")).unwrap();
    let plain = archived_source(&other);
    assert!(plan_sheet(&other, sheet_request(&plain)).is_err());
    // Neither can one whose registered sheet evidence changed on disk.
    std::fs::write(store.job_dir(&id).unwrap().join("sheets.json"), b"{}").unwrap();
    assert!(plan_sheet(&store, sheet_request(&id)).is_err());
}

#[test]
fn sheet_variant_receipts_and_outputs_gate_publication() {
    use harbor_cad::cad_sheet_variant::verify_registered_outputs;
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = sheet_source(&store);
    let plan = plan_sheet(&store, sheet_request(&id)).unwrap();
    let spec = plan.cad_sheet_variant.as_ref().unwrap();
    let mut profile = store.job_profile(&id).unwrap();
    profile.service_mode = "foreground".into();
    let binding = ExecutionBinding::capture(
        &plan,
        &profile,
        &std::env::current_exe().unwrap(),
        std::collections::BTreeMap::new(),
    )
    .unwrap();
    let authority = store
        .execution_authorization(&id)
        .unwrap()
        .unwrap()
        .authority;
    let auth = ExecutionAuthorization::capture(&plan, &profile, &binding, &authority).unwrap();
    let job = store
        .submit_authorized(&plan, "sheet-envelope-only", &profile, &binding, &auth)
        .unwrap();
    // Original staging is reused from version 1: a distinct verified copy.
    let retained = harbor_cad::cad_variant::registered(&store, &job.id, &plan).unwrap();
    assert_eq!(
        std::fs::read(store.job_dir(&id).unwrap().join("input.FCStd")).unwrap(),
        std::fs::read(retained).unwrap()
    );
    let root = store.job_dir(&job.id).unwrap();
    let save = |path: &str, data: &[u8], format| {
        let art = commit_artifact(&root, path, data, format, "persisted-byte fixture").unwrap();
        store.add_artifact(&job.id, &art).unwrap();
        art
    };
    let document = save(
        "variant.FCStd",
        b"new spreadsheet variant document",
        "FCStd",
    );
    let brep = save("solid.brep", b"opaque variant BREP", "brep");
    save("solid.stl", b"opaque variant STL", "stl");
    let original = &spec.source.regions[0];
    let bounds = [0.1, 0.13, -0.02, -0.01, 0.3, 0.31];
    let mut snapshot: harbor_cad::cad::RegionSnapshot = serde_json::from_slice(
        &std::fs::read(store.job_dir(&id).unwrap().join("regions.json")).unwrap(),
    )
    .unwrap();
    snapshot.geometry_tolerance = spec.request.geometry_tolerance.clone();
    snapshot.regions[0].bounds_m = bounds;
    snapshot.regions[0].volume_m3 = 3e-6;
    save(
        "regions.json",
        &serde_json::to_vec(&snapshot).unwrap(),
        "json",
    );
    let manifest = serde_json::json!({"schema_version":1,"synthetic":true,"gap_healing":false,"regions":[{"region_name":"solid","path":"solid.brep","sha256":brep.sha256,"bytes":brep.bytes,"source_unit":"mm","scale_to_m":0.001,"bounds_m":bounds,"volume_m3":3e-6,"source_transform":original.transform,"placement_translation_unit":"mm"}]});
    save(
        "brep-manifest.json",
        &serde_json::to_vec(&manifest).unwrap(),
        "json",
    );
    let report = serde_json::json!({"schema_version":2,"approved_variant":spec,"source_preserved":true,
        "object_type":"Spreadsheet::Sheet","sheet_name":"Params",
        "parameters":{"fin_count":{"cell":"B2","before":"8","after":"6"},"length":{"cell":"B1","before":"=20 mm","after":"=30 mm"}},
        "only_sheet_cells_changed":true,
        "before":{"solid":{"bounds_m":original.bounds_m,"volume_m3":original.volume_m3,"transform":original.transform}},
        "after":{"solid":{"bounds_m":bounds,"volume_m3":3e-6,"transform":original.transform,"surface_area_m2":1.4e-3,"min_wall_m":0.01}},
        "min_wall_method":"inward face-normal ray","gap_healing":false,"expressions":true,
        "document":{"path":"variant.FCStd","sha256":document.sha256,"bytes":document.bytes}});
    save(
        "cad-variant-recompute.json",
        &serde_json::to_vec(&report).unwrap(),
        "json",
    );
    let receipt = serde_json::json!({"adapter":"FreeCAD","version":["1","1","4"],"executed":true,"backend":"cpu","software_fallback":false,"security_minimum":"1.1.4","sandbox_required":true,"import_policy":"harbor-cad-importer-v1"});
    save(
        "cad_inspect-receipt.json",
        &serde_json::to_vec(&receipt).unwrap(),
        "json",
    );
    store.finish(&job.id, 0, None).unwrap();
    let verified = || verify_registered_outputs(&store, &job.id, &plan, &receipt);
    verified().unwrap();
    let replace = |path: &str, data: &[u8]| {
        use sha2::{Digest, Sha256};
        let recorded: String = store
            .connection
            .query_row(
                "SELECT manifest FROM artifacts WHERE job=?1 AND path=?2",
                rusqlite::params![job.id, path],
                |row| row.get(0),
            )
            .unwrap();
        let mut artifact: ArtifactManifest = serde_json::from_str(&recorded).unwrap();
        std::fs::write(root.join(path), data).unwrap();
        artifact.sha256 = format!("{:x}", Sha256::digest(data));
        artifact.bytes = data.len() as u64;
        store
            .connection
            .execute(
                "UPDATE artifacts SET manifest=?1 WHERE job=?2 AND path=?3",
                rusqlite::params![serde_json::to_string(&artifact).unwrap(), job.id, path],
            )
            .unwrap();
    };
    for (pointer, value) in [
        ("/schema_version", Some(serde_json::json!(1))),
        ("/source_preserved", Some(serde_json::json!(false))),
        ("/object_type", Some(serde_json::json!("Part::Box"))),
        ("/only_sheet_cells_changed", Some(serde_json::json!(false))),
        ("/expressions", Some(serde_json::json!(false))),
        ("/parameters/length", None),
        ("/before/solid/volume_m3", Some(serde_json::json!(2.5e-6))),
        ("/after/solid/volume_m3", Some(serde_json::json!(-3e-6))),
        ("/after/solid/volume_m3", Some(serde_json::json!(4e-6))),
        (
            "/after/solid/bounds_m",
            Some(serde_json::json!([0.1, 0.09, -0.02, -0.01, 0.3, 0.31])),
        ),
        (
            "/after/solid/surface_area_m2",
            Some(serde_json::json!(1e-6)),
        ),
        ("/after/solid/min_wall_m", Some(serde_json::json!(0.0))),
        ("/after/solid/min_wall_m", Some(serde_json::json!(0.5))),
        ("/after/solid/min_wall_m", None),
        ("/after/solid", None),
        ("/after/extra", Some(serde_json::json!({}))),
    ] {
        let mut changed = report.clone();
        mutate(&mut changed, pointer, value);
        replace(
            "cad-variant-recompute.json",
            &serde_json::to_vec(&changed).unwrap(),
        );
        assert!(verified().is_err(), "{pointer}");
    }
    replace(
        "cad-variant-recompute.json",
        &serde_json::to_vec(&report).unwrap(),
    );
    let mut wrong = snapshot.clone();
    wrong.regions[0].name = "other".into();
    replace("regions.json", &serde_json::to_vec(&wrong).unwrap());
    assert!(verified().is_err());
    replace("regions.json", &serde_json::to_vec(&snapshot).unwrap());
    verified().unwrap();
    std::fs::write(root.join("solid.stl"), b"changed").unwrap();
    assert!(verified().is_err());
}

#[test]
fn enclosure_fixture_plan_is_fixed_synthetic_and_owns_its_region_snapshot() {
    use harbor_cad::cad_sheet_variant::ENCLOSURE_REGIONS;
    let plan = ExecutionPlan::enclosure_fixture("research".into()).unwrap();
    assert_eq!(plan.stages[0].operation, StageOperation::EnclosureFixture);
    assert_eq!(plan.stages[1].operation, StageOperation::Bundle);
    let case = plan.channel_case().unwrap();
    assert!(case.geometry.synthetic);
    assert_eq!(case.regions, ENCLOSURE_REGIONS);
    assert_eq!(
        serde_json::from_value::<ExecutionPlan>(serde_json::to_value(&plan).unwrap())
            .unwrap()
            .id()
            .unwrap(),
        plan.id().unwrap()
    );
    assert!(ExecutionPlan::enclosure_fixture("ci".into()).is_err());
    let example = include_str!("../examples/enclosure-variant.json");
    match serde_json::from_str(example).unwrap() {
        harbor_cad::cad_sheet_variant::AnyCadVariantRequest::Sheet(request) => {
            request.normalized().unwrap();
            assert_eq!(request.regions, ENCLOSURE_REGIONS);
        }
        harbor_cad::cad_sheet_variant::AnyCadVariantRequest::Box(_) => panic!("box example"),
    }
    let mut changed = plan.clone();
    changed.case.as_mut().unwrap().regions.pop();
    assert!(changed.validate().is_err());
    let mut changed = plan.clone();
    changed.case.as_mut().unwrap().geometry.synthetic = false;
    assert!(changed.validate().is_err());
    let region = |name: &str| harbor_cad::cad::Region {
        name: name.into(),
        label: name.into(),
        volume_m3: 1e-6,
        bounds_m: [0., 0.01, 0., 0.01, 0., 0.01],
        transform: [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ],
        triangles: 12,
        source_unit: "mm".into(),
        stl_scale_to_m: 0.001,
    };
    let mut snapshot = harbor_cad::cad::RegionSnapshot {
        synthetic: true,
        regions: ENCLOSURE_REGIONS.iter().map(|n| region(n)).collect(),
        gap_healing: false,
        geometry_tolerance: case.geometry_tolerance.clone(),
    };
    snapshot
        .verify(case, StageOperation::EnclosureFixture)
        .unwrap();
    snapshot.regions.pop();
    assert!(
        snapshot
            .verify(case, StageOperation::EnclosureFixture)
            .is_err()
    );
}
