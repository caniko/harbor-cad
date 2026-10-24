use harbor_cad::devices::HipIdentity;
use serde_json::json;

#[test]
fn hip_receipts_bind_kernel_identity_and_compiled_runtime_without_fallback() {
    let identity = HipIdentity {
        pci: "0000:03:00.0".into(),
        render_node: "/dev/dri/renderD128".into(),
        backend_uuid: "GPU-37636635323964656134623333626164".into(),
        architecture: "gfx1100".into(),
        kfd_node: 1,
        kfd_gpu_id: 4672,
        unique_id: 9004149066219862957,
        generation: 3,
        qualified: false,
        applied_policy: "harbor-cad-native-hip-single-kfd-v1".into(),
    };
    let receipt = json!({
        "adapter":"OpenLB", "backend":"hip", "pci":identity.pci,
        "backend_uuid":identity.backend_uuid, "architecture":"gfx1100",
        "compiled_architecture":"gfx1100", "compiled_hip_version":70253211,
        "hip_runtime_version":70253211, "hip_driver_version":70253211,
        "source_revision":"145cd54810b468f4b6fd3ed86b10644264841578",
        "executed":true, "software_fallback":false, "precision":"float64",
        "gpu_blocks":1, "gpu_kernel_completion_verified":true
    });
    identity.verify_receipt(&receipt).unwrap();
    assert!(identity.verify_filter_receipt(&receipt).is_err());
    let mut filter = receipt.clone();
    filter["adapter"] = json!("Viskores");
    filter["source_revision"] = json!("7c0494a68bff379d32d6b1fbaa3d10d27a73af54");
    filter["viskores_revision"] = json!("521f3b72aabe0bf37e9972975700df27adbbae71");
    filter["kokkos_revision"] = json!("6ecdf605e0f7639adec599d25cf0e206d7b8f9f5");
    filter["hip_dispatches"] = json!(4);
    identity.verify_filter_receipt(&filter).unwrap();
    assert!(identity.verify_receipt(&filter).is_err());
    for (field, value) in [
        ("hip_dispatches", json!(0)),
        ("kokkos_revision", json!("unbound")),
        ("viskores_revision", json!("unbound")),
        ("pci", json!("0000:04:00.0")),
        ("hip_driver_version", json!(0)),
        ("precision", json!("float32")),
        ("software_fallback", json!(true)),
    ] {
        let mut changed = filter.clone();
        changed[field] = value;
        assert!(identity.verify_filter_receipt(&changed).is_err(), "{field}");
    }
    for (field, value) in [
        ("backend_uuid", json!("GPU-stale")),
        ("pci", json!("0000:04:00.0")),
        ("architecture", json!("gfx1030")),
        ("architecture", json!("gfx1100:unqualified-feature")),
        ("compiled_architecture", json!("gfx1030")),
        ("compiled_hip_version", json!(70200000)),
        ("hip_runtime_version", json!(70200000)),
        ("hip_driver_version", json!(70200000)),
        ("source_revision", json!("unbound")),
        ("executed", json!(false)),
        ("software_fallback", json!(true)),
        ("precision", json!("float32")),
        ("gpu_blocks", json!(0)),
        ("gpu_kernel_completion_verified", json!(false)),
    ] {
        let mut changed = receipt.clone();
        changed[field] = value;
        assert!(identity.verify_receipt(&changed).is_err(), "{field}");
    }
    let mut absent = receipt.clone();
    absent
        .as_object_mut()
        .unwrap()
        .remove("compiled_hip_version");
    assert!(identity.verify_receipt(&absent).is_err());
}
