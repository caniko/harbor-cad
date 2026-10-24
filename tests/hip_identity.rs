use harbor_cad::devices::{HipIdentity, KernelIdentity, KfdExclusion};
use std::collections::BTreeSet;
use std::fs;

const EXCLUSION_POLICY: &str = "harbor-cad-native-hip-kfd-exclusion-v1";

fn kernel() -> KernelIdentity {
    KernelIdentity {
        release: "7.2.8-cachyos-lto".into(),
        proc_version_sha256: "a".repeat(64),
    }
}

/// Add a live foreign KFD GPU node (distinct PCI) to an existing topology.
fn foreign_gpu(root: &std::path::Path, index: u32, gpu_id: u32, location_id: u32) {
    let node = root.join("nodes").join(index.to_string());
    fs::create_dir_all(&node).unwrap();
    fs::write(node.join("gpu_id"), format!("{gpu_id}\n")).unwrap();
    fs::write(
        node.join("properties"),
        format!("simd_count 16\ndomain 0\nlocation_id {location_id}\n"),
    )
    .unwrap();
}

fn exclusion(foreign_pcis: &[&str]) -> KfdExclusion {
    KfdExclusion {
        schema_version: 1,
        policy: EXCLUSION_POLICY.into(),
        kernel: kernel(),
        selected_pci: "0000:03:00.0".into(),
        foreign_pcis: foreign_pcis.iter().map(|s| (*s).into()).collect(),
    }
}

fn topology(root: &std::path::Path) {
    let nodes = root.join("nodes");
    fs::create_dir_all(nodes.join("0")).unwrap();
    fs::create_dir_all(nodes.join("1")).unwrap();
    fs::write(root.join("generation_id"), "1\n").unwrap();
    fs::write(nodes.join("0/gpu_id"), "0\n").unwrap();
    fs::write(
        nodes.join("0/properties"),
        "cpu_cores_count 32\nsimd_count 0\n",
    )
    .unwrap();
    fs::write(nodes.join("1/gpu_id"), "4672\n").unwrap();
    fs::write(
        nodes.join("1/properties"),
        concat!(
            "cpu_cores_count 0\nsimd_count 192\n",
            "gfx_target_version 110000\nvendor_id 4098\n",
            "location_id 768\ndomain 0\ndrm_render_minor 128\n",
            "unique_id 9004149066219862957\n",
        ),
    )
    .unwrap();
}

#[test]
fn kfd_pci_minor_uuid_and_architecture_are_one_exact_identity() {
    let root = tempfile::tempdir().unwrap();
    topology(root.path());
    let identity =
        HipIdentity::from_topology(root.path(), "0000:03:00.0", 128, "7cf529dea4b33bad").unwrap();
    assert_eq!(
        identity.backend_uuid,
        "GPU-37636635323964656134623333626164"
    );
    assert_eq!(identity.architecture, "gfx1100");
    assert_eq!(identity.kfd_gpu_id, 4672);
    assert!(
        HipIdentity::from_topology(root.path(), "0000:04:00.0", 128, "7cf529dea4b33bad").is_err()
    );
    assert!(
        HipIdentity::from_topology(root.path(), "0000:03:00.0", 129, "7cf529dea4b33bad").is_err()
    );
    assert!(
        HipIdentity::from_topology(root.path(), "0000:03:00.0", 128, "0000000000000000").is_err()
    );
    assert!(
        HipIdentity::from_topology(root.path(), "0000:03:00.0", 128, "7cf529dea4b33bae").is_err()
    );
}

#[test]
fn second_kfd_gpu_is_rejected_even_when_selected_metadata_matches() {
    let root = tempfile::tempdir().unwrap();
    topology(root.path());
    fs::create_dir(root.path().join("nodes/2")).unwrap();
    fs::write(root.path().join("nodes/2/gpu_id"), "4800\n").unwrap();
    fs::write(root.path().join("nodes/2/properties"), "simd_count 8\n").unwrap();
    assert!(
        HipIdentity::from_topology(root.path(), "0000:03:00.0", 128, "7cf529dea4b33bad").is_err()
    );
}

#[test]
fn malformed_properties_and_symlinks_cannot_supply_device_identity() {
    let root = tempfile::tempdir().unwrap();
    topology(root.path());
    let path = root.path().join("nodes/1/properties");
    let original = fs::read_to_string(&path).unwrap();
    for suffix in ["domain 0\n", "domain nope\n", "unknown 1 trailing\n"] {
        fs::write(&path, format!("{original}{suffix}")).unwrap();
        assert!(
            HipIdentity::from_topology(root.path(), "0000:03:00.0", 128, "7cf529dea4b33bad")
                .is_err()
        );
    }
    fs::write(&path, &original).unwrap();
    fs::rename(&path, root.path().join("foreign")).unwrap();
    std::os::unix::fs::symlink(root.path().join("foreign"), &path).unwrap();
    assert!(
        HipIdentity::from_topology(root.path(), "0000:03:00.0", 128, "7cf529dea4b33bad").is_err()
    );
}

#[test]
fn verified_exclusion_receipt_admits_the_selected_gpu_beside_a_foreign_gpu() {
    let root = tempfile::tempdir().unwrap();
    topology(root.path());
    foreign_gpu(root.path(), 2, 4800, 0x7d00); // 0000:7d:00.0
    let k = kernel();
    let excl = exclusion(&["0000:7d:00.0"]);
    let identity = HipIdentity::from_topology_checked(
        root.path(),
        "0000:03:00.0",
        128,
        "7cf529dea4b33bad",
        Some(&k),
        Some(&excl),
    )
    .unwrap();
    assert_eq!(identity.kfd_gpu_id, 4672);
    assert_eq!(identity.applied_policy, EXCLUSION_POLICY);
}

#[test]
fn second_gpu_without_a_receipt_or_kernel_is_refused() {
    let root = tempfile::tempdir().unwrap();
    topology(root.path());
    foreign_gpu(root.path(), 2, 4800, 0x7d00);
    let k = kernel();
    let excl = exclusion(&["0000:7d:00.0"]);
    // Missing receipt.
    assert!(
        HipIdentity::from_topology_checked(
            root.path(),
            "0000:03:00.0",
            128,
            "7cf529dea4b33bad",
            Some(&k),
            None,
        )
        .is_err()
    );
    // Missing running-kernel binding.
    assert!(
        HipIdentity::from_topology_checked(
            root.path(),
            "0000:03:00.0",
            128,
            "7cf529dea4b33bad",
            None,
            Some(&excl),
        )
        .is_err()
    );
    // The single-KFD wrapper keeps refusing.
    assert!(
        HipIdentity::from_topology(root.path(), "0000:03:00.0", 128, "7cf529dea4b33bad").is_err()
    );
}

#[test]
fn receipt_bound_to_a_different_kernel_or_pci_set_is_refused() {
    let root = tempfile::tempdir().unwrap();
    topology(root.path());
    foreign_gpu(root.path(), 2, 4800, 0x7d00);
    let k = kernel();
    let resolve = |excl: &KfdExclusion| {
        HipIdentity::from_topology_checked(
            root.path(),
            "0000:03:00.0",
            128,
            "7cf529dea4b33bad",
            Some(&k),
            Some(excl),
        )
    };
    // Kernel release drift.
    let mut other_kernel = exclusion(&["0000:7d:00.0"]);
    other_kernel.kernel.release = "7.2.9-cachyos-lto".into();
    assert!(resolve(&other_kernel).is_err());
    // /proc/version digest drift.
    let mut other_build = exclusion(&["0000:7d:00.0"]);
    other_build.kernel.proc_version_sha256 = "b".repeat(64);
    assert!(resolve(&other_build).is_err());
    // Foreign PCI set does not match the observed card.
    assert!(resolve(&exclusion(&["0000:aa:00.0"])).is_err());
    // Matching receipt still accepted.
    assert!(resolve(&exclusion(&["0000:7d:00.0"])).is_ok());
}

#[test]
fn an_extra_third_gpu_outside_the_receipt_is_refused() {
    let root = tempfile::tempdir().unwrap();
    topology(root.path());
    foreign_gpu(root.path(), 2, 4800, 0x7d00); // 0000:7d:00.0
    foreign_gpu(root.path(), 3, 4900, 0x0400); // 0000:04:00.0
    let k = kernel();
    // Receipt covers only one of the two foreign cards.
    assert!(
        HipIdentity::from_topology_checked(
            root.path(),
            "0000:03:00.0",
            128,
            "7cf529dea4b33bad",
            Some(&k),
            Some(&exclusion(&["0000:7d:00.0"])),
        )
        .is_err()
    );
    // Receipt covering the exact foreign set is accepted.
    let identity = HipIdentity::from_topology_checked(
        root.path(),
        "0000:03:00.0",
        128,
        "7cf529dea4b33bad",
        Some(&k),
        Some(&exclusion(&["0000:7d:00.0", "0000:04:00.0"])),
    )
    .unwrap();
    assert_eq!(identity.applied_policy, EXCLUSION_POLICY);
}

#[test]
fn exclusion_receipt_validate_rejects_malformed_policy_and_pci_sets() {
    let mut excl = exclusion(&["0000:7d:00.0"]);
    assert!(excl.validate().is_ok());
    // Wrong policy id.
    let mut wrong = excl.clone();
    wrong.policy = "harbor-cad-native-hip-single-kfd-v1".into();
    assert!(wrong.validate().is_err());
    // Selected card listed as foreign.
    excl.foreign_pcis = vec!["0000:03:00.0".into()];
    assert!(excl.validate().is_err());
    // Empty foreign set.
    let mut empty = exclusion(&["0000:7d:00.0"]);
    empty.foreign_pcis.clear();
    assert!(empty.validate().is_err());
    // verify() set comparison is order-independent.
    let both = exclusion(&["0000:7d:00.0", "0000:04:00.0"]);
    let observed: BTreeSet<String> =
        ["0000:04:00.0".to_string(), "0000:7d:00.0".to_string()].into();
    assert!(both.verify(&kernel(), "0000:03:00.0", &observed).is_ok());
}
