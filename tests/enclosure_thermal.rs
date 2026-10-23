//! General-solid mesh and enclosure thermal contracts: planning, refusals,
//! retention and independent receipt gates. Persisted-record fixtures only;
//! nothing here executes Gmsh or CalculiX.
use harbor_cad::enclosure_thermal::{
    self as et, CadSolidsRecipe, EnclosureThermalInputs, EnclosureThermalSpec, SolidMesh,
    SolidMeshRequest,
};
use harbor_cad::{
    authority::{ExecutionAuthorization, HostAuthority},
    contracts::*,
    execution::{ExecutionBinding, FileIdentity},
    storage::{Store, commit_artifact},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const IDENTITY: [f64; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];

fn profile(store: &Store) -> HostExecutionProfile {
    HostExecutionProfile {
        schema_version: 1,
        policy: "research".into(),
        allowed_input_root: store.root.to_string_lossy().into(),
        max_ram_bytes: 64 * 1024 * 1024 * 1024,
        max_disk_bytes: 64 * 1024 * 1024 * 1024,
        threads: 1,
        timeout_seconds: 60,
        native_runtime: None,
        service_mode: "systemd".into(),
    }
}

fn authority(profile: &HostExecutionProfile) -> HostAuthority {
    HostAuthority {
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
    }
}

fn finish(store: &Store, id: &str) {
    store.transition(id, "queued", "starting", None).unwrap();
    store.transition(id, "starting", "running", None).unwrap();
    store.finish(id, 0, None).unwrap();
}

fn register(store: &Store, id: &str, path: &str, bytes: &[u8], format: &str) -> ArtifactManifest {
    let root = store.job_dir(id).unwrap();
    let record = commit_artifact(&root, path, bytes, format, "persisted-record fixture").unwrap();
    store.add_artifact(id, &record).unwrap();
    record
}

/// An archived authorized CAD job with named regions `(name, bounds, volume)`.
fn archived(store: &Store, regions: &[(&str, [f64; 6], f64)]) -> String {
    let mut case = CaseSpec::reference();
    case.regions = regions.iter().map(|r| r.0.to_string()).collect();
    case.geometry.source = "controlled.FCStd".into();
    let document = b"opaque archived original CAD document";
    use sha2::{Digest, Sha256};
    case.geometry.sha256 = Some(format!("{:x}", Sha256::digest(document)));
    let plan = ExecutionPlan::cad_inspection(case, "research".into(), 64 * 1024 * 1024).unwrap();
    let profile = profile(store);
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
        native_files: BTreeMap::from([
            (
                serde_json::to_string(&StageOperation::CadInspect).unwrap(),
                identity.clone(),
            ),
            ("cad_closure".into(), identity),
        ]),
    };
    let authorization =
        ExecutionAuthorization::capture(&plan, &profile, &binding, &authority(&profile)).unwrap();
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
    register(store, &job.id, "input.FCStd", document, "FCStd");
    let (mut snapshot, mut manifest) = (vec![], vec![]);
    for (name, bounds, volume) in regions {
        let bytes = format!("opaque archived BREP bytes of {name}");
        let record = register(
            store,
            &job.id,
            &format!("{name}.brep"),
            bytes.as_bytes(),
            "brep",
        );
        snapshot.push(json!({"name":name,"label":name,"volume_m3":volume,"bounds_m":bounds,"transform":IDENTITY,
            "triangles":12,"source_unit":"mm","stl_scale_to_m":0.001}));
        manifest.push(json!({"region_name":name,"path":format!("{name}.brep"),"bytes":record.bytes,"sha256":record.sha256,
            "source_unit":"mm","scale_to_m":0.001,"bounds_m":bounds,"volume_m3":volume,"source_transform":IDENTITY,"placement_translation_unit":"mm"}));
    }
    for (path, value) in [
        (
            "cad_inspect-receipt.json",
            json!({"adapter":"FreeCAD","executed":true,"backend":"cpu","software_fallback":false,"security_minimum":"1.1.4",
                "sandbox_required":true,"import_policy":harbor_cad::sandbox::IMPORT_POLICY}),
        ),
        (
            "regions.json",
            json!({"synthetic":true,"regions":snapshot,"gap_healing":false,"geometry_tolerance":{"value":1e-5,"unit":"m"}}),
        ),
        (
            "brep-manifest.json",
            json!({"schema_version":1,"synthetic":true,"regions":manifest,"gap_healing":false}),
        ),
    ] {
        register(
            store,
            &job.id,
            path,
            &serde_json::to_vec(&value).unwrap(),
            "json",
        );
    }
    finish(store, &job.id);
    job.id
}

const HOUSING: [f64; 6] = [0., 0.02, 0., 0.01, 0., 0.01];
const PCB: [f64; 6] = [0.02, 0.03, 0., 0.01, 0., 0.01];

fn boxes(store: &Store) -> String {
    archived(store, &[("housing", HOUSING, 2e-6), ("pcb", PCB, 1e-6)])
}

fn mesh_spec() -> SolidMesh {
    serde_json::from_value(
        json!({"characteristic_length_max_m":0.005,"element_order":1,"max_elements":10000,
        "volume_relative_tolerance":1e-9,"min_quality":0.05,"geometry_tolerance_m":1e-6}),
    )
    .unwrap()
}

fn request(id: &str) -> SolidMeshRequest {
    SolidMeshRequest {
        source_job: id.into(),
        regions: vec!["housing".into(), "pcb".into()],
        mesh: mesh_spec(),
    }
}

#[test]
fn general_solid_mesh_plan_is_an_exclusive_v7_field_and_keeps_the_box_path() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    // A finned housing: volume below its bounding box, which the box path refuses.
    let id = archived(&store, &[("housing", HOUSING, 1.5e-6), ("pcb", PCB, 1e-6)]);
    assert!(harbor_cad::cad_source::source(&store, &id, "housing", 4, 1e-6).is_err());
    let plan = et::plan_mesh(&store, request(&id), "research".into()).unwrap();
    let raw = serde_json::to_value(&plan).unwrap();
    assert_eq!(raw["schema_version"], 7);
    assert_eq!(raw["stages"][0]["operation"], "cad_mesh");
    assert!(raw.get("cad_source").is_none() && raw.get("imported_fem").is_none());
    assert!(raw["cad_solids"].get("thermal").is_none());
    let source = &plan.cad_solids.as_ref().unwrap().source;
    assert_eq!(source.geometry.names(), ["housing", "pcb"]);
    assert_eq!(source.breps[1].path, "pcb.brep");
    assert_eq!(
        serde_json::from_value::<ExecutionPlan>(raw.clone())
            .unwrap()
            .id()
            .unwrap(),
        plan.id().unwrap()
    );
    for version in [1, 6, 8, 16] {
        let mut old = raw.clone();
        old["schema_version"] = json!(version);
        assert!(
            serde_json::from_value::<ExecutionPlan>(old).is_err(),
            "{version}"
        );
    }
    let mut injected =
        serde_json::to_value(ExecutionPlan::reference(CaseSpec::reference()).unwrap()).unwrap();
    injected["cad_solids"] = raw["cad_solids"].clone();
    assert!(serde_json::from_value::<ExecutionPlan>(injected).is_err());
    let mut weak = plan.clone();
    weak.stages[0].ram_bytes = 1;
    assert!(weak.validate().is_err());
    let mut wrong = plan.clone();
    wrong.stages[0].operation = StageOperation::FemImported;
    assert!(wrong.validate().is_err());
    assert!(et::plan_mesh(&store, request(&id), "ci".into()).is_err());
    assert!(store.submit(&plan, "unauthorized-solids").is_err());
}

#[test]
fn mesh_requests_refuse_unmapped_regions_budgets_and_unbounded_gates() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = boxes(&store);
    let refused = |change: &dyn Fn(&mut SolidMeshRequest)| {
        let mut changed = request(&id);
        change(&mut changed);
        et::plan_mesh(&store, changed, "research".into()).is_err()
    };
    assert!(refused(
        &|r| r.regions = vec!["housing".into(), "lid".into()]
    ));
    assert!(refused(
        &|r| r.regions = vec!["pcb".into(), "housing".into()]
    ));
    assert!(refused(&|r| r.regions.clear()));
    assert!(refused(&|r| r.mesh.max_elements = 10));
    assert!(refused(&|r| r.mesh.max_elements = et::MAX_ELEMENTS + 1));
    assert!(refused(&|r| r.mesh.element_order = 3));
    assert!(refused(&|r| r.mesh.volume_relative_tolerance = 0.5));
    assert!(refused(&|r| r.mesh.min_quality = 1.));
    assert!(refused(&|r| r.source_job = uuid::Uuid::nil().to_string()));
    let mut foreign = serde_json::to_value(request(&id)).unwrap();
    foreign["mesh"]["healing"] = json!(true);
    assert!(serde_json::from_value::<SolidMeshRequest>(foreign).is_err());
}

fn known(value: f64, unit: &str) -> Value {
    json!({"availability":"known","value":{"value":value,"unit":unit},"provenance":"synthetic contract value","synthetic":true})
}

fn inputs() -> Value {
    json!({"schema_version":1,
        "materials":{"housing":{"kind":"library","id":"al-6061-t6__cnc-3axis-al"},
                     "pcb":{"kind":"declared","conductivity":known(5., "W/(m*K)")}},
        "heat_source_region":"pcb","heat_source_power":known(2., "W"),
        "ambient_temperature":known(40., "degC"),"film_coefficient":known(8., "W/(m2*K)"),
        "exposed_regions":["housing"],"internal_film_coefficient":known(3., "W/(m2*K)"),
        "solar":{"irradiance":known(800., "W/m2"),"direction":[0.,0.,1.]},
        "radiation":true,"energy_balance_tolerance":1e-3})
}

#[test]
fn thermal_inputs_resolve_library_routes_conservatively_and_refuse_missing_data() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = boxes(&store);
    let (_, source) = et::solids(&store, &id, &request(&id).regions, &mesh_spec()).unwrap();
    let resolve = |value: Value| {
        et::resolve(
            &serde_json::from_value::<EnclosureThermalInputs>(value)?,
            &source.geometry,
        )
    };
    let model = resolve(inputs()).unwrap();
    let housing = &model.materials["housing"];
    assert_eq!(housing.source, "library:al-6061-t6");
    assert_eq!(housing.conductivity_w_m_k, 166.);
    assert_eq!(housing.emissivity, Some(0.03));
    assert_eq!(housing.solar_absorptance, Some(0.3));
    assert!((model.ambient_k - 313.15).abs() < 1e-12);
    assert!(model.synthetic);
    assert_eq!(model.provenance.len(), 5);
    let refused = |pointer: &str, value: Value| {
        let mut changed = inputs();
        *changed.pointer_mut(pointer).unwrap() = value;
        resolve(changed).unwrap_err().to_string()
    };
    // PA12 has no sourced solar absorptance: the solar term is refused, not defaulted.
    let message = refused(
        "/materials/housing",
        json!({"kind":"library","id":"pa12-mjf__mjf-pa12"}),
    );
    assert!(message.contains("solar absorptance"), "{message}");
    assert!(
        refused(
            "/materials/housing",
            json!({"kind":"library","id":"unobtainium"})
        )
        .contains("unknown material")
    );
    assert!(
        refused(
            "/heat_source_power",
            json!({"availability":"missing","reason":"not measured"})
        )
        .contains("not measured")
    );
    assert!(
        refused(
            "/materials/pcb/conductivity",
            json!({"availability":"missing","reason":"no board data"})
        )
        .contains("no board data")
    );
    // Declared emissivity missing with radiation on an exposed declared region.
    let mut declared = inputs();
    declared["materials"]["housing"] = json!({"kind":"declared","conductivity":known(100., "W/(m*K)"),
        "solar_absorptance":known(0.5, "1"),"emissivity":{"availability":"missing","reason":"unmeasured coating"}});
    assert!(
        resolve(declared.clone())
            .unwrap_err()
            .to_string()
            .contains("emissivity")
    );
    declared["radiation"] = json!(false);
    assert!(resolve(declared).is_ok());
    refused("/heat_source_region", json!("lid"));
    refused("/exposed_regions", json!(["pcb", "housing"]));
    refused("/energy_balance_tolerance", json!(1e-2));
    refused("/solar/direction", json!([0., 0., 2.]));
    let mut extra = inputs();
    extra["materials"]["lid"] = json!({"kind":"library","id":"al-6061-t6"});
    assert!(resolve(extra).unwrap_err().to_string().contains("lid"));
    let mut absent = inputs();
    absent["materials"].as_object_mut().unwrap().remove("pcb");
    assert!(resolve(absent).unwrap_err().to_string().contains("pcb"));
}

/// Kuhn-split structured tetrahedra of the HOUSING and PCB boxes, sharing x=0.02.
fn box_mesh(n: u64) -> Value {
    let (nx, ny, nz) = (3 * n, n, n);
    let id = |i: u64, j: u64, k: u64| 1 + i + (nx + 1) * (j + (ny + 1) * k);
    let mut nodes = serde_json::Map::new();
    for k in 0..=nz {
        for j in 0..=ny {
            for i in 0..=nx {
                let xyz = [
                    0.03 * i as f64 / nx as f64,
                    0.01 * j as f64 / ny as f64,
                    0.01 * k as f64 / nz as f64,
                ];
                nodes.insert(id(i, j, k).to_string(), json!(xyz));
            }
        }
    }
    let point =
        |n: u64| -> [f64; 3] { serde_json::from_value(nodes[&n.to_string()].clone()).unwrap() };
    let mut elements = serde_json::Map::new();
    let mut regions: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
    let mut tets = vec![];
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                for order in [
                    [0, 1, 2],
                    [0, 2, 1],
                    [1, 0, 2],
                    [1, 2, 0],
                    [2, 0, 1],
                    [2, 1, 0],
                ] {
                    let mut c = [i, j, k];
                    let mut path = vec![id(c[0], c[1], c[2])];
                    for axis in order {
                        c[axis] += 1;
                        path.push(id(c[0], c[1], c[2]));
                    }
                    let [a, b, cc, d] = [0, 1, 2, 3].map(|m| point(path[m]));
                    let e = |p: [f64; 3], q: [f64; 3]| [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
                    let (u, v, w) = (e(b, a), e(cc, a), e(d, a));
                    let det = u[0] * (v[1] * w[2] - v[2] * w[1])
                        - u[1] * (v[0] * w[2] - v[2] * w[0])
                        + u[2] * (v[0] * w[1] - v[1] * w[0]);
                    if det < 0. {
                        path.swap(1, 2);
                    }
                    let region = if i < 2 * n { "housing" } else { "pcb" };
                    tets.push((region, path));
                }
            }
        }
    }
    for (index, (region, path)) in tets.iter().enumerate() {
        elements.insert((index + 1).to_string(), json!(path));
        regions.entry(region).or_default().push(index as u64 + 1);
    }
    let faces = [[0, 1, 2], [0, 3, 1], [1, 3, 2], [2, 3, 0]];
    let mut count: BTreeMap<Vec<u64>, Vec<(u64, u64)>> = BTreeMap::new();
    for (index, (_, path)) in tets.iter().enumerate() {
        for (f, corners) in faces.iter().enumerate() {
            let mut key: Vec<u64> = corners.iter().map(|c| path[*c]).collect();
            key.sort_unstable();
            count
                .entry(key)
                .or_default()
                .push((index as u64 + 1, f as u64 + 1));
        }
    }
    let boundary: Vec<Value> = count
        .values()
        .filter(|owners| owners.len() == 1)
        .map(|owners| json!([owners[0].0, owners[0].1, "ambient"]))
        .collect();
    json!({"schema_version":1,"synthetic":true,"coordinate_unit":"m","element_type":"C3D4",
        "nodes":nodes,"elements":elements,"regions":regions,"boundary_faces":boundary,
        "classification":"fixture: every unshared face ambient"})
}

#[test]
fn independent_mesh_gate_rejects_inversion_budget_volume_faces_and_regions() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = boxes(&store);
    let (_, source) = et::solids(&store, &id, &request(&id).regions, &mesh_spec()).unwrap();
    let geometry = &source.geometry;
    let mesh = box_mesh(2);
    let facts = et::verify_mesh(geometry, &serde_json::to_vec(&mesh).unwrap()).unwrap();
    assert_eq!(facts.elements, 6 * 24);
    assert!(facts.max_relative_volume_error < 1e-12);
    assert!((facts.interfaces_m2["housing|pcb"] - 1e-4).abs() < 1e-15);
    assert!((facts.ambient_m2["pcb"] - 5e-4).abs() < 1e-15);
    let refused = |change: &dyn Fn(&mut Value)| {
        let mut changed = mesh.clone();
        change(&mut changed);
        et::verify_mesh(geometry, &serde_json::to_vec(&changed).unwrap()).is_err()
    };
    assert!(refused(&|m| {
        let element = m["elements"]["1"].as_array_mut().unwrap();
        element.swap(1, 2);
    }));
    assert!(refused(&|m| m["element_type"] = json!("C3D10")));
    assert!(refused(&|m| m["boundary_faces"]
        .as_array_mut()
        .unwrap()
        .truncate(3)));
    assert!(refused(&|m| m["boundary_faces"][0][2] = json!("outdoors")));
    assert!(refused(&|m| {
        let pcb = m["regions"]["pcb"].as_array_mut().unwrap().pop().unwrap();
        m["regions"]["housing"].as_array_mut().unwrap().push(pcb);
    }));
    assert!(refused(&|m| {
        let pcb = m["regions"].as_object_mut().unwrap().remove("pcb").unwrap();
        m["regions"]["lid"] = pcb;
    }));
    assert!(refused(&|m| m["nodes"]["1"] = json!([-0.001, 0., 0.])));
    let mut small = geometry.clone();
    small.mesh.max_elements = 100;
    assert!(et::verify_mesh(&small, &serde_json::to_vec(&mesh).unwrap()).is_err());
    let mut strict = geometry.clone();
    strict.mesh.min_quality = 0.9;
    assert!(et::verify_mesh(&strict, &serde_json::to_vec(&mesh).unwrap()).is_err());
}

fn submit_authorized(store: &Store, plan: &ExecutionPlan, key: &str, cad: &str) -> String {
    let mut profile = store.job_profile(cad).unwrap();
    profile.service_mode = "foreground".into();
    let binding = ExecutionBinding::capture(
        plan,
        &profile,
        &std::env::current_exe().unwrap(),
        BTreeMap::new(),
    )
    .unwrap();
    let authorization =
        ExecutionAuthorization::capture(plan, &profile, &binding, &authority(&profile)).unwrap();
    store
        .submit_authorized(plan, key, &profile, &binding, &authorization)
        .unwrap()
        .id
}

#[test]
fn retained_solids_and_thermal_planning_bind_a_succeeded_mesh_job() {
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let cad = boxes(&store);
    let mesh_plan = et::plan_mesh(&store, request(&cad), "research".into()).unwrap();
    let mesh_job = submit_authorized(&store, &mesh_plan, "solids", &cad);
    let copies = et::registered(&store, &mesh_job, &mesh_plan).unwrap();
    for name in [
        "housing.brep",
        "pcb.brep",
        "brep-manifest.json",
        "regions.json",
    ] {
        let original = store.job_dir(&cad).unwrap().join(name);
        assert_ne!(
            std::fs::metadata(copies.join(name)).unwrap().ino(),
            std::fs::metadata(original).unwrap().ino()
        );
    }
    let thermal_request = |job: &str| {
        serde_json::from_value::<et::EnclosureThermalRequest>(
            json!({"mesh_job":job,"spec":inputs()}),
        )
        .unwrap()
    };
    // Not yet succeeded, and a CAD job is not a mesh job.
    assert!(et::plan_thermal(&store, thermal_request(&mesh_job), "research".into()).is_err());
    assert!(et::plan_thermal(&store, thermal_request(&cad), "research".into()).is_err());
    let mesh = serde_json::to_vec(&box_mesh(2)).unwrap();
    let sha = register(&store, &mesh_job, "stages/mesh/mesh.json", &mesh, "json").sha256;
    register(
        &store,
        &mesh_job,
        "stages/mesh/cad-mesh-receipt.json",
        &serde_json::to_vec(&json!({"mesh_sha256":sha})).unwrap(),
        "json",
    );
    finish(&store, &mesh_job);
    let plan = et::plan_thermal(&store, thermal_request(&mesh_job), "research".into()).unwrap();
    let raw = serde_json::to_value(&plan).unwrap();
    assert_eq!(raw["schema_version"], 8);
    assert_eq!(raw["stages"][0]["operation"], "fem_imported");
    let spec = plan.cad_solids.as_ref().unwrap().thermal.as_ref().unwrap();
    assert_eq!(spec.mesh_sha256, sha);
    assert_eq!(spec.mesh_plan_digest, mesh_plan.id().unwrap());
    assert_eq!(
        serde_json::from_value::<ExecutionPlan>(raw.clone())
            .unwrap()
            .id()
            .unwrap(),
        plan.id().unwrap()
    );
    let mut old = raw.clone();
    old["schema_version"] = json!(7);
    assert!(serde_json::from_value::<ExecutionPlan>(old).is_err());
    let descriptor = serde_json::to_value(et::thermal_descriptor(&plan).unwrap()).unwrap();
    assert_eq!(descriptor["schema_version"], 2);
    assert_eq!(
        descriptor["thermal"]["materials"]["pcb"]["emissivity"],
        Value::Null
    );
    // Substituted original source bytes refuse re-planning.
    std::fs::write(
        store.job_dir(&cad).unwrap().join("pcb.brep"),
        b"substituted",
    )
    .unwrap();
    assert!(et::plan_thermal(&store, thermal_request(&mesh_job), "research".into()).is_err());
}

fn thermal_plan(store: &Store, cad: &str, mesh_sha256: &str) -> ExecutionPlan {
    let (_, source) = et::solids(store, cad, &request(cad).regions, &mesh_spec()).unwrap();
    let mut model =
        et::resolve(&serde_json::from_value(inputs()).unwrap(), &source.geometry).unwrap();
    model.solar = None;
    model.radiation = false;
    ExecutionPlan::cad_solids(
        CadSolidsRecipe {
            source,
            thermal: Some(EnclosureThermalSpec {
                schema_version: 1,
                mesh_job: uuid::Uuid::new_v4().to_string(),
                mesh_plan_digest: "c".repeat(64),
                mesh_sha256: mesh_sha256.into(),
                thermal: model,
            }),
        },
        "research".into(),
    )
    .unwrap()
}

#[test]
fn thermal_receipt_recomputes_statistics_and_gates_the_energy_balance() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let cad = boxes(&store);
    let root = temp.path().join("native");
    std::fs::create_dir(&root).unwrap();
    let mesh_value = box_mesh(2);
    let mesh = commit_artifact(
        &root,
        "mesh.json",
        &serde_json::to_vec(&mesh_value).unwrap(),
        "json",
        "fixture",
    )
    .unwrap();
    let plan = thermal_plan(&store, &cad, &mesh.sha256);
    let model = &plan
        .cad_solids
        .as_ref()
        .unwrap()
        .thermal
        .as_ref()
        .unwrap()
        .thermal;
    // Only the exposed housing films (0.0009 m2 without the shared face); a
    // uniform field at Ta + P/(h A) balances the 2 W source exactly.
    let uniform = model.ambient_k + 2. / (model.film_w_m2_k * 0.0009);
    let write_field = |temperature: &dyn Fn(f64) -> f64| {
        let mut text = String::from(" temperatures for set NALL and time  0.1000000E+01\n\n");
        let nodes = mesh_value["nodes"].as_object().unwrap();
        let mut ids: Vec<u64> = nodes.keys().map(|k| k.parse().unwrap()).collect();
        ids.sort_unstable();
        for n in ids {
            let x = nodes[&n.to_string()][0].as_f64().unwrap();
            text.push_str(&format!("{n:10} {:.15E}\n", temperature(x)));
        }
        std::fs::write(root.join("reference.dat"), text).unwrap();
        use sha2::{Digest, Sha256};
        format!(
            "{:x}",
            Sha256::digest(std::fs::read(root.join("reference.dat")).unwrap())
        )
    };
    let field = write_field(&|_| uniform);
    let region = json!({"max_k":uniform,"min_k":uniform,"mean_k":uniform});
    let checks: serde_json::Map<String, Value> = [
        "operation_closure_only",
        "no_gpu_nodes",
        "no_sysfs",
        "no_host_home",
        "no_session_bus",
        "no_worker_socket",
        "network_namespace_isolated",
        "descriptor_readonly",
        "source_brep_readonly",
        "named_source_only",
    ]
    .into_iter()
    .map(|k| (k.to_string(), json!(true)))
    .collect();
    let receipt = json!({"schema_version":1,"adapter":"CalculiX","backend":"cpu","factorization":"SPOOLES","executed":true,
        "software_fallback":false,"synthetic":true,"precision":"float64","formulation":et::THERMAL_FORMULATION,
        "calculix_version":"2.23","gmsh_version":"4.15.2",
        "calculix_source_sha256":"9c88385c10fb04f5dc6c4e98027a51bebdd8aee3920e05190d6c1dd08357d6e7",
        "gmsh_source_sha256":"be3f66f225d27ba9fa014f07e83169285da8a051b0e8ab7103d88066b39bdd3e",
        "request_sha256":digest(&et::thermal_descriptor(&plan).unwrap()).unwrap(),"mesh_sha256":mesh.sha256,
        "native_field_sha256":field,"element_type":"C3D4","nodes":mesh_value["nodes"].as_object().unwrap().len(),
        "elements":144,"regions":{"housing":region,"pcb":region},"global_max":{"temperature_k":uniform},
        "internal_air_k":null,"energy_balance":{"heat_source_w":2.,"solar_absorbed_w":0.,"film_w":2.,"radiation_w":0.,
        "heat_in_w":2.,"heat_out_w":2.,"relative_error":0.,"tolerance":1e-3,"internal_air_residual_w":0.,"internal_air_relative_error":0.},
        "temperature_unit":"K","applicability":{"regime":"steady state"},"gap_healing":false,"physical_validation":"unqualified",
        "sandbox":{"policy":"harbor-cad-fem-imported-cpu-v1","checks":checks}});
    let evidence = et::verify_thermal_receipt(&plan, &root, &receipt).unwrap();
    assert!(evidence.error < 1e-12 && evidence.tolerance == 1e-3);
    for (pointer, value) in [
        ("/regions/pcb/max_k", json!(uniform + 1.)),
        ("/global_max/temperature_k", json!(uniform - 1.)),
        ("/mesh_sha256", json!("b".repeat(64))),
        ("/internal_air_k", json!(320.)),
        ("/energy_balance/tolerance", json!(1e-2)),
        ("/request_sha256", json!("b".repeat(64))),
        ("/sandbox/checks/named_source_only", json!(false)),
        ("/applicability", Value::Null),
    ] {
        let mut changed = receipt.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert!(
            et::verify_thermal_receipt(&plan, &root, &changed).is_err(),
            "{pointer}"
        );
    }
    // A field 1 K hotter loses ~2.3 % more heat than the source supplies.
    let hotter = uniform + 1.;
    let mut imbalanced = receipt.clone();
    imbalanced["native_field_sha256"] = json!(write_field(&|_| hotter));
    let hot = json!({"max_k":hotter,"min_k":hotter,"mean_k":hotter});
    imbalanced["regions"] = json!({"housing":hot,"pcb":hot});
    imbalanced["global_max"]["temperature_k"] = json!(hotter);
    imbalanced["energy_balance"]["film_w"] =
        json!(2. * (hotter - model.ambient_k) / (uniform - model.ambient_k));
    let message = et::verify_thermal_receipt(&plan, &root, &imbalanced)
        .unwrap_err()
        .to_string();
    assert!(message.contains("energy-balance"), "{message}");
    // The approved mesh digest is bound: a different mesh refuses before any field check.
    let other = thermal_plan(&store, &cad, &"d".repeat(64));
    assert!(et::verify_thermal_receipt(&other, &root, &receipt).is_err());
}

#[test]
fn final_temperature_block_is_the_steady_result() {
    let text = " temperatures for set NALL and time  0.5000000E+00\n\n 1 3.0E+02\n\n temperatures for set NALL and time  0.1000000E+01\n\n         1  3.100000E+02\n         2  3.2D+02\n";
    let values = et::read_temperatures(text).unwrap();
    assert_eq!(values[&1], 310.);
    assert_eq!(values[&2], 320.);
    assert!(
        et::read_temperatures(" temperatures for set NALL and time  0.5E+00\n 1 3E2\n").is_err()
    );
    assert!(
        et::read_temperatures(" temperatures for set NALL and time  1.0E+00\n 1 3E2\n 1 3E2\n")
            .is_err()
    );
    assert!(
        et::read_temperatures(" stresses (elem, integ.pnt.,sxx) for set EALL and time 1.\n")
            .is_err()
    );
}

