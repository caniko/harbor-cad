use crate::{
    Error, Result,
    contracts::{Role, invalid},
};
use serde::{Deserialize, Serialize};
#[path = "hip_identity.rs"]
mod hip_identity;
pub use hip_identity::{HipIdentity, HipSandbox};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub pci: String,
    pub vendor: String,
    pub render_node: Option<String>,
    pub backend_uuid: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct Binding {
    pub physical_key: String,
    pub role: Role,
    pub backend: String,
    pub render_node: Option<String>,
    pub backend_uuid: Option<String>,
    pub qualified: bool,
}
pub fn inventory() -> Result<Vec<Device>> {
    let mut out = Vec::new();
    let path = Path::new("/sys/bus/pci/devices");
    if !path.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let class = fs::read_to_string(entry.path().join("class"))?;
        if !class.starts_with("0x03") {
            continue;
        }
        let pci = entry.file_name().to_string_lossy().into_owned();
        let vendor_id = fs::read_to_string(entry.path().join("vendor"))?;
        let vendor = match vendor_id.trim() {
            "0x1002" => "amd",
            "0x10de" => "nvidia",
            "0x8086" => "intel",
            _ => "unknown",
        };
        let render = format!("/dev/dri/by-path/pci-{pci}-render");
        out.push(Device {
            pci,
            vendor: vendor.into(),
            render_node: Path::new(&render).exists().then_some(render),
            backend_uuid: None,
        });
    }
    out.sort_by(|a, b| a.pci.cmp(&b.pci));
    Ok(out)
}
pub fn resolve(
    devices: &[Device],
    role: Role,
    backend: &str,
    pci: Option<&str>,
) -> Result<Binding> {
    let candidates: Vec<_> = devices
        .iter()
        .filter(|d| pci.is_none_or(|id| id == d.pci))
        .filter(|d| match backend {
            "cuda" => d.vendor == "nvidia",
            "hip" => d.vendor == "amd",
            "egl" | "vaapi" => d.render_node.is_some(),
            _ => false,
        })
        .collect();
    if candidates.len() != 1 {
        return Err(Error::Unqualified(format!(
            "{backend}: expected one exact live device binding, found {}",
            candidates.len()
        )));
    }
    let device = candidates[0];
    let backend_uuid = if role == Role::Compute && backend == "hip" {
        Some(HipIdentity::resolve(&device.pci)?.backend_uuid)
    } else {
        if role == Role::Compute && (backend != "cuda" || device.backend_uuid.is_none()) {
            return Err(Error::Unqualified(
                "CUDA UUID/PCI correlation must be established by the adapter; no fallback".into(),
            ));
        }
        device.backend_uuid.clone()
    };
    if (role == Role::Render && backend != "egl") || (role == Role::Media && backend != "vaapi") {
        return Err(invalid("operation/backend mismatch"));
    }
    if let Some(node) = &device.render_node
        && (fleetix::gpu::pci_selector(node).is_none() || !node.contains(&device.pci))
    {
        return Err(invalid("stale stable render-node identity"));
    }
    if devices
        .iter()
        .map(|d| &d.pci)
        .collect::<BTreeSet<_>>()
        .len()
        != devices.len()
    {
        return Err(invalid("duplicate physical devices"));
    }
    Ok(Binding {
        physical_key: device.pci.clone(),
        role,
        backend: backend.into(),
        render_node: device.render_node.clone(),
        backend_uuid,
        qualified: false,
    })
}

// libdrm 2.4.134 uses selected DRM sysfs metadata even with an open render FD.
// Never mount all /sys, PCI config/resources, or another card to satisfy it.
const DRM_PCI_ATTRIBUTES: [&str; 6] = [
    "vendor",
    "device",
    "subsystem_vendor",
    "subsystem_device",
    "revision",
    "uevent",
];

/// Render (EGL) and media (VAAPI) stages see only their selected render node,
/// its read-only sysfs metadata and the driver tree; never `/dev/kfd` or another card.
pub const RENDER_SANDBOX_POLICY: &str = "harbor-cad-render-drm-single-node-v1";
pub const MEDIA_SANDBOX_POLICY: &str = "harbor-cad-media-drm-single-node-v1";

/// Compute never uses this path: HIP takes `HipSandbox`, CUDA has no qualified mount policy.
pub fn drm_policy(role: Role, backend: &str) -> Result<&'static str> {
    match (role, backend) {
        (Role::Render, "egl") => Ok(RENDER_SANDBOX_POLICY),
        (Role::Media, "vaapi") => Ok(MEDIA_SANDBOX_POLICY),
        _ => Err(invalid(
            "single-node DRM sandbox requires render/EGL or media/VAAPI",
        )),
    }
}

#[derive(Debug, Serialize)]
pub struct DrmSandbox {
    pci: String,
    node: PathBuf,
    sysfs_device: PathBuf,
    sysfs_render: PathBuf,
    sysfs_alias: PathBuf,
}
impl DrmSandbox {
    fn render_alias(pci: &str) -> Result<String> {
        let alias = format!("/dev/dri/by-path/pci-{pci}-render");
        if pci.to_ascii_lowercase() != pci || fleetix::gpu::pci_selector(&alias).is_none() {
            return Err(invalid("canonical PCI render selector required"));
        }
        Ok(alias)
    }
    pub fn resolve(pci: &str) -> Result<Self> {
        let node = fs::canonicalize(Self::render_alias(pci)?)?;
        let metadata = fs::metadata(&node)?;
        if !metadata.file_type().is_char_device() {
            return Err(invalid(
                "selected DRM render node must be a character device",
            ));
        }
        Self::from_sysfs(pci, &node, metadata.rdev(), Path::new("/sys"))
    }
    fn from_sysfs(pci: &str, node: &Path, rdev: u64, sys: &Path) -> Result<Self> {
        let name = node
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| invalid("DRM render node name"))?;
        if node.parent() != Some(Path::new("/dev/dri"))
            || name
                .strip_prefix("renderD")
                .is_none_or(|minor| minor.is_empty() || !minor.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(invalid("canonical DRM render node required"));
        }
        let dev = format!("{}:{}", libc::major(rdev), libc::minor(rdev));
        let sysfs_alias = sys.join("dev/char").join(&dev);
        let sysfs_device = fs::canonicalize(sys.join("bus/pci/devices").join(pci))?;
        let sysfs_render = fs::canonicalize(&sysfs_alias)?;
        if !sysfs_device.starts_with(sys.join("devices"))
            || sysfs_device.file_name().and_then(|n| n.to_str()) != Some(pci)
            || sysfs_render != sysfs_device.join("drm").join(name)
            || fs::canonicalize(sysfs_alias.join("device"))? != sysfs_device
            || fs::read_to_string(sysfs_render.join("dev"))?.trim() != dev
            || fs::read_link(sysfs_device.join("subsystem"))?
                .file_name()
                .and_then(|n| n.to_str())
                != Some("pci")
        {
            return Err(invalid("DRM minor/sysfs/PCI identity mismatch"));
        }
        for name in DRM_PCI_ATTRIBUTES {
            if !fs::symlink_metadata(sysfs_device.join(name))?.is_file() {
                return Err(invalid(
                    "DRM metadata attributes must be regular non-symlink files",
                ));
            }
        }
        Ok(Self {
            pci: pci.into(),
            node: node.into(),
            sysfs_device,
            sysfs_render,
            sysfs_alias,
        })
    }
    /// Append only a verified render node, read-only selected-device metadata,
    /// the driver tree and the stage's policy id; returns that id.
    pub fn apply(&self, role: Role, backend: &str, command: &mut Command) -> Result<&'static str> {
        let policy = drm_policy(role, backend)?;
        self.apply_compute(command);
        command.args([
            "--ro-bind",
            "/run/opengl-driver",
            "/run/opengl-driver",
            "--setenv",
            "HARBOR_CAD_DRM_POLICY",
            policy,
        ]);
        Ok(policy)
    }
    /// The adapter's in-sandbox view must name the policy and show only this node.
    pub fn verify_receipt(&self, policy: &str, receipt: &serde_json::Value) -> Result<()> {
        let sandbox = &receipt["sandbox"];
        let name = self.node.file_name().and_then(|n| n.to_str());
        if sandbox["policy"] != policy
            || sandbox["render_node"].as_str() != self.node.to_str()
            || sandbox["visible_dri"] != serde_json::json!(["by-path", name])
            || sandbox["visible_by_path"] != serde_json::json!([format!("pci-{}-render", self.pci)])
            || sandbox["kfd_visible"] != false
        {
            return Err(Error::Unqualified(
                "render/media receipt lacks single selected DRM node sandbox evidence".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn apply_compute(&self, command: &mut Command) {
        command.args(["--dev-bind"]).arg(&self.node).arg(&self.node);
        command
            .args(["--dir", "/dev/dri/by-path", "--symlink"])
            .arg(&self.node)
            .arg(format!("/dev/dri/by-path/pci-{}-render", self.pci));
        for name in DRM_PCI_ATTRIBUTES {
            let path = self.sysfs_device.join(name);
            command.arg("--ro-bind").arg(&path).arg(&path);
        }
        command
            .arg("--ro-bind")
            .arg(&self.sysfs_render)
            .arg(&self.sysfs_render)
            .args(["--symlink", "/sys/bus/pci"])
            .arg(self.sysfs_device.join("subsystem"))
            .args(["--dir", "/sys/dev/char", "--symlink"])
            .arg(&self.sysfs_render)
            .arg(&self.sysfs_alias);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn drm_metadata_binding_rejects_stale_pci_minor_and_foreign_attribute_links() {
        let root = tempfile::tempdir().unwrap();
        let sys = root.path();
        let pci = "0000:03:00.0";
        let device = sys.join("devices/pci0000:00").join(pci);
        let render = device.join("drm/renderD128");
        fs::create_dir_all(&render).unwrap();
        fs::create_dir_all(sys.join("dev/char")).unwrap();
        fs::create_dir_all(sys.join("bus/pci/devices")).unwrap();
        symlink(&device, sys.join("bus/pci/devices").join(pci)).unwrap();
        symlink(&render, sys.join("dev/char/226:128")).unwrap();
        symlink("../..", render.join("device")).unwrap();
        symlink(sys.join("bus/pci"), device.join("subsystem")).unwrap();
        fs::write(render.join("dev"), "226:128\n").unwrap();
        for name in DRM_PCI_ATTRIBUTES {
            fs::write(device.join(name), "0x1002\n").unwrap();
        }
        let node = Path::new("/dev/dri/renderD128");
        assert_eq!(
            DrmSandbox::render_alias(pci).unwrap(),
            "/dev/dri/by-path/pci-0000:03:00.0-render"
        );
        assert!(DrmSandbox::render_alias("0000:0A:00.0").is_err());
        let binding = DrmSandbox::from_sysfs(pci, node, libc::makedev(226, 128), sys).unwrap();
        let mut command = std::process::Command::new("bwrap");
        assert_eq!(
            binding.apply(Role::Render, "egl", &mut command).unwrap(),
            RENDER_SANDBOX_POLICY
        );
        let args: Vec<_> = command
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        let read_only: Vec<_> = args
            .windows(3)
            .filter(|a| a[0] == "--ro-bind")
            .map(|a| a[1].clone())
            .collect();
        assert_eq!(read_only.len(), DRM_PCI_ATTRIBUTES.len() + 2);
        assert!(!read_only.contains(&device.to_string_lossy().into_owned()));
        assert!(!read_only.contains(&sys.to_string_lossy().into_owned()));
        assert_eq!(args.windows(3).filter(|a| a[0] == "--dev-bind").count(), 1);
        assert!(!args.iter().any(|a| a == "/dev/kfd"));
        assert!(
            args.windows(3)
                .any(|a| a == ["--setenv", "HARBOR_CAD_DRM_POLICY", RENDER_SANDBOX_POLICY])
        );
        assert!(
            DrmSandbox::from_sysfs("0000:04:00.0", node, libc::makedev(226, 128), sys).is_err()
        );
        assert!(
            DrmSandbox::from_sysfs(
                pci,
                Path::new("/dev/dri/renderD129"),
                libc::makedev(226, 128),
                sys
            )
            .is_err()
        );
        fs::write(render.join("dev"), "226:129\n").unwrap();
        assert!(DrmSandbox::from_sysfs(pci, node, libc::makedev(226, 128), sys).is_err());
        fs::write(render.join("dev"), "226:128\n").unwrap();
        fs::remove_file(device.join("vendor")).unwrap();
        symlink("/etc/passwd", device.join("vendor")).unwrap();
        assert!(DrmSandbox::from_sysfs(pci, node, libc::makedev(226, 128), sys).is_err());
        assert!(DrmSandbox::resolve("../invalid").is_err());
    }

    #[test]
    fn render_and_media_select_distinct_single_node_policies_and_verify_receipts() {
        assert_eq!(
            drm_policy(Role::Render, "egl").unwrap(),
            RENDER_SANDBOX_POLICY
        );
        assert_eq!(
            drm_policy(Role::Media, "vaapi").unwrap(),
            MEDIA_SANDBOX_POLICY
        );
        for (role, backend) in [
            (Role::Compute, "hip"),
            (Role::Compute, "cuda"),
            (Role::Render, "vaapi"),
            (Role::Media, "egl"),
        ] {
            assert!(drm_policy(role, backend).is_err(), "{role:?} {backend}");
        }
        let binding = DrmSandbox {
            pci: "0000:03:00.0".into(),
            node: "/dev/dri/renderD128".into(),
            sysfs_device: "/sys/devices/pci0000:00/0000:03:00.0".into(),
            sysfs_render: "/sys/devices/pci0000:00/0000:03:00.0/drm/renderD128".into(),
            sysfs_alias: "/sys/dev/char/226:128".into(),
        };
        let mut command = Command::new("bwrap");
        assert_eq!(
            binding.apply(Role::Media, "vaapi", &mut command).unwrap(),
            MEDIA_SANDBOX_POLICY
        );
        assert!(
            binding
                .apply(Role::Compute, "hip", &mut Command::new("bwrap"))
                .is_err()
        );
        let receipt = serde_json::json!({"sandbox": {
            "policy": MEDIA_SANDBOX_POLICY,
            "render_node": "/dev/dri/renderD128",
            "visible_dri": ["by-path", "renderD128"],
            "visible_by_path": ["pci-0000:03:00.0-render"],
            "kfd_visible": false,
        }});
        binding
            .verify_receipt(MEDIA_SANDBOX_POLICY, &receipt)
            .unwrap();
        assert!(
            binding
                .verify_receipt(RENDER_SANDBOX_POLICY, &receipt)
                .is_err()
        );
        for (key, value) in [
            ("render_node", serde_json::json!("/dev/dri/renderD129")),
            (
                "visible_dri",
                serde_json::json!(["by-path", "renderD128", "renderD129"]),
            ),
            (
                "visible_by_path",
                serde_json::json!(["pci-0000:03:00.0-render", "pci-0000:7a:00.0-render"]),
            ),
            ("kfd_visible", serde_json::json!(true)),
        ] {
            let mut changed = receipt.clone();
            changed["sandbox"][key] = value;
            assert!(
                binding
                    .verify_receipt(MEDIA_SANDBOX_POLICY, &changed)
                    .is_err(),
                "{key}"
            );
        }
        assert!(
            binding
                .verify_receipt(MEDIA_SANDBOX_POLICY, &serde_json::json!({}))
                .is_err()
        );
    }
}
