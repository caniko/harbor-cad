use harbor_cad::{
    admission::Admission,
    authority::{ExecutionAuthorization, FilesystemBudget, HostAuthority},
    contracts::*,
    execution::ExecutionBinding,
    storage::Store,
    worker::load_profile,
};
use std::path::Path;

fn policy(root: &Path) -> HostAuthority {
    HostAuthority {
        schema_version: 1,
        fleetix_revision: FLEETIX_REV.into(),
        fleetix_contract_digest: fleetix_digest(),
        max_ram_bytes: 24 * 1024 * 1024,
        ram_headroom_bytes: 0,
        filesystems: vec![FilesystemBudget {
            root: root.to_str().unwrap().into(),
            max_bytes: 8 * 1024 * 1024,
            free_headroom_bytes: 0,
        }],
        cards: vec![],
        routes: vec![],
        allowed_devices: vec![],
        overrides: vec![],
        native_runtimes: vec![],
        allowed_input_roots: vec!["/nonexistent".into()],
        kfd_exclusion: None,
    }
}
fn submit(store: &Store, host: &HostAuthority, key: &str) -> String {
    let profile = load_profile(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/profiles/ci.json"
    )))
    .unwrap();
    let plan = ExecutionPlan::reference(CaseSpec::reference()).unwrap();
    let binding = ExecutionBinding::capture(
        &plan,
        &profile,
        Path::new(env!("CARGO_BIN_EXE_harbor-cad")),
        Default::default(),
    )
    .unwrap();
    let authorization = ExecutionAuthorization::capture(&plan, &profile, &binding, host).unwrap();
    store
        .submit_authorized(&plan, key, &profile, &binding, &authorization)
        .unwrap()
        .id
}

#[test]
fn two_state_roots_share_ram_and_keep_reservations_until_verified_tree_closure() {
    let temp = tempfile::tempdir().unwrap();
    let host = policy(temp.path());
    let ledger = temp.path().join("admission");
    let one = Store::open(&temp.path().join("one")).unwrap();
    let two = Store::open(&temp.path().join("two")).unwrap();
    let a = submit(&one, &host, "a");
    let b = submit(&two, &host, "b");
    let admission = Admission::open(&ledger, &host).unwrap();
    assert!(admission.reserve(&one, &a).unwrap());
    assert!(!admission.reserve(&two, &b).unwrap());
    drop(admission);
    let admission = Admission::open(&ledger, &host).unwrap();
    assert!(admission.reserve(&one, &a).unwrap());
    assert!(!admission.reserve(&two, &b).unwrap());
    one.transition(&a, "queued", "starting", None).unwrap();
    one.finish(&a, 0, None).unwrap();
    admission.reconcile(|_, _| Ok(false)).unwrap();
    assert!(!admission.reserve(&two, &b).unwrap());
    admission.reconcile(|_, _| Ok(true)).unwrap();
    assert!(admission.reserve(&two, &b).unwrap());
}

#[test]
fn shared_filesystem_counts_retained_and_quarantined_bytes_and_rejects_policy_split() {
    let temp = tempfile::tempdir().unwrap();
    let mut host = policy(temp.path());
    host.max_ram_bytes *= 4;
    let ledger = temp.path().join("admission");
    let one = Store::open(&temp.path().join("one")).unwrap();
    let two = Store::open(&temp.path().join("two")).unwrap();
    let a = submit(&one, &host, "a");
    let b = submit(&two, &host, "b");
    let admission = Admission::open(&ledger, &host).unwrap();
    admission.register_state(&one.root).unwrap();
    admission.register_state(&two.root).unwrap();
    let raw = one.root.join("quarantined");
    std::fs::write(&raw, vec![0; host.filesystems[0].max_bytes as usize]).unwrap();
    assert!(!admission.reserve(&two, &b).unwrap());
    assert!(!admission.reserve(&one, &a).unwrap());
    std::fs::remove_file(raw).unwrap();
    assert!(admission.reserve(&two, &b).unwrap());
    let mut split = host.clone();
    split.max_ram_bytes *= 2;
    assert!(Admission::open(&ledger, &split).is_err());
    let duplicate = host.filesystems[0].clone();
    host.filesystems.push(duplicate);
    assert!(Admission::open(&temp.path().join("different"), &host).is_err());
}

#[test]
fn shared_admission_rejects_foreign_database_links_and_unbound_jobs() {
    let temp = tempfile::tempdir().unwrap();
    let host = policy(temp.path());
    let ledger = temp.path().join("admission");
    harbor_cad::storage::private_dir(&ledger).unwrap();
    let outside = temp.path().join("foreign");
    std::fs::write(&outside, b"untouched").unwrap();
    std::os::unix::fs::symlink(&outside, ledger.join("admission.sqlite3")).unwrap();
    assert!(Admission::open(&ledger, &host).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"untouched");
    std::fs::remove_file(ledger.join("admission.sqlite3")).unwrap();
    let store = Store::open(&temp.path().join("state")).unwrap();
    let plan = ExecutionPlan::reference(CaseSpec::reference()).unwrap();
    let job = store.submit(&plan, "legacy").unwrap();
    let admission = Admission::open(&ledger, &host).unwrap();
    assert!(admission.reserve(&store, &job.id).is_err());
    drop(admission);
    std::fs::remove_file(ledger.join("initialize.lock")).unwrap();
    std::os::unix::fs::symlink(&outside, ledger.join("initialize.lock")).unwrap();
    assert!(Admission::open(&ledger, &host).is_err());
    assert_eq!(std::fs::read(outside).unwrap(), b"untouched");
}

#[test]
fn bundled_sqlite_includes_the_upstream_wal_reset_corruption_fix() {
    // Upstream identifies 3.51.3 (3051003) as the first fixed release in
    // this version line. Admission uses multiple WAL connections/processes.
    assert!(
        rusqlite::version_number() >= 3_051_003,
        "{} lacks the required WAL-reset fix",
        rusqlite::version()
    );
}

#[test]
fn concurrent_workers_cannot_both_admit_against_one_ram_pool() {
    let temp = tempfile::tempdir().unwrap();
    let host = policy(temp.path());
    let ledger = temp.path().join("admission");
    let one = Store::open(&temp.path().join("one")).unwrap();
    let two = Store::open(&temp.path().join("two")).unwrap();
    let a = submit(&one, &host, "a");
    let b = submit(&two, &host, "b");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = [(one.root.clone(), a), (two.root.clone(), b)]
        .into_iter()
        .map(|(root, id)| {
            let barrier = barrier.clone();
            let ledger = ledger.clone();
            let host = host.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let admission = Admission::open(&ledger, &host).unwrap();
                let store = Store::open(&root).unwrap();
                barrier.wait();
                admission.reserve(&store, &id).unwrap()
            })
        })
        .collect();
    assert_eq!(
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|admitted| *admitted)
            .count(),
        1
    );
}

#[test]
fn policy_promotion_requires_closed_reservations_and_never_upgrades_bound_jobs() {
    let temp = tempfile::tempdir().unwrap();
    let host = policy(temp.path());
    let root = temp.path().join("admission");
    let store = Store::open(&temp.path().join("state")).unwrap();
    let id = submit(&store, &host, "a");
    let queued = submit(&store, &host, "queued");
    let admission = Admission::open(&root, &host).unwrap();
    assert!(admission.reserve(&store, &id).unwrap());
    let mut changed = host.clone();
    changed.max_ram_bytes *= 2;
    assert!(Admission::install(&root, &changed).is_err());
    store.transition(&id, "queued", "cancelled", None).unwrap();
    admission.reconcile(|_, _| Ok(true)).unwrap();
    Admission::install(&root, &changed).unwrap();
    assert!(admission.reserve(&store, &queued).is_err());
    let current = Admission::open(&root, &changed).unwrap();
    assert!(current.reserve(&store, &queued).is_err());
    assert_eq!(
        digest(
            &store
                .execution_authorization(&queued)
                .unwrap()
                .unwrap()
                .authority
        )
        .unwrap(),
        digest(&host).unwrap()
    );
}
