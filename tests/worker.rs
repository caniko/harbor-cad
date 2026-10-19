use harbor_cad::{contracts::*, storage::*, worker::*};
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(root: &std::path::Path) -> Worker {
    start_with_profile(
        root,
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/profiles/ci.json")),
    )
}
fn start_with_profile(root: &std::path::Path, profile: &std::path::Path) -> Worker {
    start_with_binary(
        root,
        profile,
        std::path::Path::new(env!("CARGO_BIN_EXE_harbor-cad")),
    )
}
fn start_with_binary(
    root: &std::path::Path,
    profile: &std::path::Path,
    binary: &std::path::Path,
) -> Worker {
    let child = Command::new(binary)
        .args(["worker", "--state"])
        .arg(root)
        .arg("--profile")
        .arg(profile)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now();
    while !root.join("worker.sock").exists() {
        assert!(deadline.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    Worker(child)
}

#[test]
fn execution_preflight_shares_submission_checks_without_creating_jobs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let _worker = start(&root);
    let socket = root.join("worker.sock");
    let plan = ExecutionPlan::reference(CaseSpec::reference()).unwrap();
    let operation = |plan: &ExecutionPlan| {
        serde_json::from_value(serde_json::json!({
            "operation": "execution_preflight", "plan": plan
        }))
        .unwrap()
    };
    let response = request(&socket, operation(&plan)).unwrap();
    assert!(response.ok, "{response:?}");
    let report = response.data.unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(
        report["execution_binding"]["plan_digest"],
        plan.id().unwrap()
    );
    assert_eq!(report["peak_ram_bytes"], plan.peak_ram());
    assert_eq!(
        report["disk_reservation_bytes"],
        plan.disk_reservation().unwrap()
    );
    assert_eq!(report["execution_approved"], false);
    assert_eq!(report["resources_reserved"], false);
    assert_eq!(report["executed"], false);
    assert_eq!(report["qualification"], "not_assessed");
    assert_eq!(
        request(&socket, operation(&plan)).unwrap().data.unwrap(),
        report
    );

    let path = temp.path().join("plan.json");
    std::fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_harbor-cad"))
        .arg("--socket")
        .arg(&socket)
        .args(["job", "preflight"])
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"],
        report
    );

    let mut changed = plan.clone();
    changed.policy = "research".into();
    assert_eq!(
        request(&socket, operation(&changed))
            .unwrap()
            .error
            .unwrap()["code"],
        "invalid_input"
    );
    changed = plan.clone();
    changed.stages[0].ram_bytes = 1024 * 1024 * 1024;
    assert_eq!(
        request(&socket, operation(&changed))
            .unwrap()
            .error
            .unwrap()["code"],
        "resource_unavailable"
    );
    let store = Store::open(&root).unwrap();
    for table in [
        "jobs",
        "events",
        "artifacts",
        "job_profiles",
        "job_executions",
        "job_authorizations",
        "studies",
    ] {
        let count: i64 = store
            .connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "preflight wrote {table}");
    }
    assert!(!root.join("artifacts").exists());
    assert!(!root.join("runtime-roots").exists());
}

#[test]
fn native_execution_preflight_refuses_a_missing_runtime_without_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let mut profile = load_profile(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/profiles/ci.json"
    )))
    .unwrap();
    profile.policy = "research".into();
    profile.service_mode = "systemd".into();
    profile.max_ram_bytes = 4 * 1024 * 1024 * 1024;
    profile.max_disk_bytes = 4 * 1024 * 1024 * 1024;
    let path = temp.path().join("profile.json");
    std::fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();
    let _worker = start_with_profile(&root, &path);
    let mut case = CaseSpec::reference();
    case.applicability.formulation = "periodic_forced_channel".into();
    case.applicability.numerical_tolerance = 0.02;
    case.length.value = 0.02;
    case.acceleration.value = 0.001;
    case.resolution = 16;
    case.max_time_s = 20.;
    let plan = ExecutionPlan::openlb_reference(case, "research".into()).unwrap();
    let operation =
        serde_json::from_value(serde_json::json!({"operation":"execution_preflight", "plan":plan}))
            .unwrap();
    let response = request(&root.join("worker.sock"), operation).unwrap();
    assert!(!response.ok);
    let error = response.error.unwrap();
    assert_eq!(error["code"], "unqualified");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("runtime not configured")
    );
    let store = Store::open(&root).unwrap();
    let jobs: i64 = store
        .connection
        .query_row("SELECT count(*) FROM jobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(jobs, 0);
}

#[test]
fn queued_job_and_duplicate_submission_keep_runner_a_after_worker_b_restart() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let store = Store::open(&root).unwrap();
    let profile_path =
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/profiles/ci.json"));
    let profile = load_profile(profile_path).unwrap();
    let runner_a = temp.path().join("runner-a");
    std::fs::copy(env!("CARGO_BIN_EXE_harbor-cad"), &runner_a).unwrap();
    std::fs::create_dir(root.join("artifacts")).unwrap();
    let occupied = root.join("artifacts/occupied");
    std::fs::write(&occupied, vec![0; profile.max_disk_bytes as usize]).unwrap();
    let worker_a = start_with_binary(&root, profile_path, &runner_a);
    let plan = ExecutionPlan::reference(CaseSpec::reference()).unwrap();
    let submit = Operation::Submit {
        approved_digest: plan.id().unwrap(),
        plan: Box::new(plan),
        idempotency_key: "runner-upgrade".into(),
    };
    let socket = root.join("worker.sock");
    let response = request(&socket, submit.clone()).unwrap();
    assert!(response.ok, "{response:?}");
    let id = response.data.unwrap()["id"].as_str().unwrap().to_owned();
    assert_eq!(store.job(&id).unwrap().state, "queued");
    drop(worker_a);
    std::fs::remove_file(&socket).unwrap();
    let worker_b = start(&root);
    assert_eq!(request(&socket, submit).unwrap().data.unwrap()["id"], id);
    std::fs::remove_file(occupied).unwrap();
    let deadline = Instant::now();
    loop {
        let job = store.job(&id).unwrap();
        if job.state == "succeeded" {
            break;
        }
        assert_ne!(job.state, "failed", "{job:?}");
        assert!(deadline.elapsed() < Duration::from_secs(10), "{job:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
    let binding: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.job_dir(&id).unwrap().join("execution-binding.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(binding["runner"]["path"], runner_a.to_str().unwrap());
    drop(worker_b);
}
#[test]
fn real_worker_disconnect_idempotency_restart_and_export() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let worker = start(&root);
    let socket = root.join("worker.sock");
    let mut plan = ExecutionPlan::reference(CaseSpec::reference()).unwrap();
    plan.stages.push(Stage {
        id: "bundle".into(),
        dependencies: vec!["reference".into()],
        operation: StageOperation::Bundle,
        gpu: GpuRequirement::CpuOnly,
        selection: None,
        ram_bytes: 16 * 1024 * 1024,
        vram_bytes: 0,
    });
    let submit = Operation::Submit {
        approved_digest: plan.id().unwrap(),
        plan: Box::new(plan.clone()),
        idempotency_key: "disconnect-fixture".into(),
    };
    let response = request(&socket, submit.clone()).unwrap();
    assert!(response.ok, "{response:?}");
    let id = response.data.unwrap()["id"].as_str().unwrap().to_owned();
    let deadline = Instant::now();
    loop {
        let response = request(&socket, Operation::Status { job_id: id.clone() }).unwrap();
        let data = response.data.unwrap();
        if data["state"] == "succeeded" {
            break;
        }
        assert!(data["state"] != "failed", "{data}");
        assert!(deadline.elapsed() < Duration::from_secs(10), "{data}");
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(worker);
    std::fs::remove_file(&socket).unwrap();
    let _worker = start(&root);
    assert_eq!(request(&socket, submit).unwrap().data.unwrap()["id"], id);
    let artifacts = request(
        &socket,
        Operation::Artifacts {
            job_id: id.clone(),
            after: None,
            limit: 20,
        },
    )
    .unwrap()
    .data
    .unwrap();
    assert_eq!(artifacts["items"].as_array().unwrap().len(), 6);
    assert!(
        artifacts["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["path"] == "execution-binding.json")
    );
    assert!(artifacts["next_after"].is_null());
    let destination = temp.path().join("portable");
    let export = Command::new(env!("CARGO_BIN_EXE_harbor-cad"))
        .args(["artifact", "export", "--state"])
        .arg(&root)
        .arg(&id)
        .arg(&destination)
        .output()
        .unwrap();
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stdout)
    );
    assert!(destination.join("channel.csv").is_file());
    assert!(destination.join("manifest.json").is_file());
    let store = Store::open(&root).unwrap();
    assert!(store.logs(&id, 0, 101).is_err());
    assert_eq!(store.active().unwrap().len(), 0);
}

#[test]
fn worker_rejects_approval_drift_unknown_operations_and_gpu_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let _worker = start(&root);
    let socket = root.join("worker.sock");
    let plan = ExecutionPlan::reference(CaseSpec::reference()).unwrap();
    let response = request(
        &socket,
        Operation::Submit {
            plan: Box::new(plan),
            approved_digest: "bad".into(),
            idempotency_key: "a".into(),
        },
    )
    .unwrap();
    assert!(!response.ok);
    assert_eq!(response.error.unwrap()["code"], "invalid_input");
    use std::io::{BufRead, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    stream.write_all(b"{\"protocol_version\":1,\"request_id\":\"x\",\"request\":{\"operation\":\"shell\",\"command\":\"id\"}}\n").unwrap();
    let mut reply = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut reply)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&reply).unwrap()["ok"],
        false
    );
    assert!(Store::open(&root).unwrap().active().unwrap().is_empty());
}

#[test]
fn invalid_operation_inputs_keep_the_valid_protocol_identity_and_never_dispatch() {
    use std::io::{BufRead, Write};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let _worker = start(&root);
    let socket = root.join("worker.sock");
    for body in [
        r#"{"operation":"doctor","unexpected":null}"#,
        r#"{"operation":"unknown"}"#,
        r#"{"operation":"doctor","operation":"doctor"}"#,
        r#"{"operation":"validate","case":null}"#,
    ] {
        let payload = format!(
            "{{\"protocol_version\":1,\"request_id\":\"schema-rejection\",\"request\":{body}}}\n"
        );
        let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
        stream.write_all(payload.as_bytes()).unwrap();
        let mut reply = String::new();
        std::io::BufReader::new(stream)
            .read_line(&mut reply)
            .unwrap();
        let response: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(response["request_id"], "schema-rejection", "{response}");
        assert_eq!(response["protocol_version"], 1);
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "invalid_input");
    }
    // Ambiguous/malformed envelopes cannot supply an authenticated request identity.
    for payload in [
        r#"{"protocol_version":1,"request_id":"a","request_id":"b","request":{"operation":"doctor"}}"#,
        r#"{"protocol_version":1,"request_id":"not/a/token","request":{"operation":"doctor"}}"#,
        r#"{"protocol_version":1,"request_id":"x","extra":null,"request":{"operation":"doctor"}}"#,
    ] {
        let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
        stream.write_all(format!("{payload}\n").as_bytes()).unwrap();
        let mut reply = String::new();
        std::io::BufReader::new(stream)
            .read_line(&mut reply)
            .unwrap();
        let response: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(response["request_id"], "invalid");
        assert_eq!(response["ok"], false);
    }
    assert!(Store::open(&root).unwrap().active().unwrap().is_empty());
    assert!(request(&socket, Operation::Doctor {}).unwrap().ok);
}

#[test]
fn artifact_pages_fit_transport_and_retain_every_shard() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let store = Store::open(&root).unwrap();
    let job = store
        .submit(
            &ExecutionPlan::reference(CaseSpec::reference()).unwrap(),
            "pages",
        )
        .unwrap();
    store
        .transition(&job.id, "queued", "starting", None)
        .unwrap();
    store.finish(&job.id, 0, None).unwrap();
    let directory = store.job_dir(&job.id).unwrap();
    for n in 0..302 {
        let artifact = commit_artifact(
            &directory,
            &format!("shards/{n:04}.csv"),
            b"x\n1\n",
            "csv",
            &"long provenance ".repeat(250),
        )
        .unwrap();
        store.add_artifact(&job.id, &artifact).unwrap();
    }
    let _worker = start(&root);
    let socket = root.join("worker.sock");
    let mut after = None::<String>;
    let mut paths = Vec::new();
    loop {
        let op = serde_json::from_value(
            serde_json::json!({"operation":"artifacts","job_id":job.id,"after":after,"limit":100}),
        )
        .unwrap();
        let response = request(&socket, op).unwrap();
        assert!(response.ok, "{response:?}");
        assert!(serde_json::to_vec(&response).unwrap().len() < MAX_MESSAGE as usize);
        let page = response.data.unwrap();
        assert_eq!(page["total"], 302);
        for item in page["items"].as_array().unwrap() {
            paths.push(item["path"].as_str().unwrap().to_owned());
        }
        after = page["next_after"].as_str().map(str::to_owned);
        if after.is_none() {
            break;
        }
        assert_eq!(after.as_ref(), paths.last());
    }
    assert_eq!(
        paths,
        (0..302)
            .map(|n| format!("shards/{n:04}.csv"))
            .collect::<Vec<_>>()
    );
    let described = request(
        &socket,
        Operation::Describe {
            job_id: job.id.clone(),
        },
    )
    .unwrap();
    assert!(described.ok, "{described:?}");
    assert_eq!(described.data.unwrap()["artifacts"]["total"], 302);
    let invalid = serde_json::from_value(
        serde_json::json!({"operation":"artifacts","job_id":job.id,"limit":101}),
    )
    .unwrap();
    assert_eq!(
        request(&socket, invalid).unwrap().error.unwrap()["code"],
        "invalid_input"
    );
}

#[test]
fn queued_job_keeps_effective_profile_when_source_configuration_changes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let store = Store::open(&root).unwrap();
    let profile_path = temp.path().join("profile.json");
    let profile = load_profile(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/profiles/ci.json"
    )))
    .unwrap();
    std::fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
    std::fs::create_dir(root.join("artifacts")).unwrap();
    let occupied = root.join("artifacts/retained-fixture");
    std::fs::write(&occupied, vec![0u8; profile.max_disk_bytes as usize]).unwrap();
    let _worker = start_with_profile(&root, &profile_path);
    let socket = root.join("worker.sock");
    let plan = ExecutionPlan::reference(CaseSpec::reference()).unwrap();
    let response = request(
        &socket,
        Operation::Submit {
            approved_digest: plan.id().unwrap(),
            plan: Box::new(plan),
            idempotency_key: "frozen-queued".into(),
        },
    )
    .unwrap();
    assert!(response.ok, "{response:?}");
    let id = response.data.unwrap()["id"].as_str().unwrap().to_owned();
    assert_eq!(store.job(&id).unwrap().state, "queued");
    let mut invalid = profile.clone();
    invalid.threads = 0;
    std::fs::write(&profile_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
    std::fs::remove_file(occupied).unwrap();
    let deadline = Instant::now();
    loop {
        let job = store.job(&id).unwrap();
        if job.state == "succeeded" {
            break;
        }
        assert_ne!(job.state, "failed", "{job:?}");
        assert!(deadline.elapsed() < Duration::from_secs(5), "{job:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
    let retained: HostExecutionProfile = serde_json::from_slice(
        &std::fs::read(store.job_dir(&id).unwrap().join("host-profile.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(digest(&retained).unwrap(), digest(&profile).unwrap());
}
