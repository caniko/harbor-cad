//! Source-backed ROCm 7.2.3 identity for the single-KFD-GPU policy candidate.
use super::DrmSandbox;
use crate::{
    Error, Result,
    contracts::invalid,
    execution::{HIP_SANDBOX_POLICY, KFD_EXCLUSION_POLICY},
    resources::pci_key,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    path::Path,
    process::Command,
};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct HipIdentity {
    pub pci: String,
    pub render_node: String,
    pub backend_uuid: String,
    pub architecture: String,
    pub kfd_node: u32,
    pub kfd_gpu_id: u32,
    pub unique_id: u64,
    pub generation: u64,
    pub qualified: bool,
    /// Which policy admitted this device: the single-KFD-GPU policy when the host
    /// exposes one live GPU, or the exclusion policy when a verified receipt bound
    /// a second live KFD GPU out of reach.
    pub applied_policy: String,
}

/// Running-kernel identity a KFD exclusion receipt is bound to. `/proc/version`
/// pins the exact build (compiler, config, timestamp); the release alone is not
/// enough to tie direct ioctl evidence to the kernel it was observed on.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KernelIdentity {
    pub release: String,
    pub proc_version_sha256: String,
}

impl KernelIdentity {
    pub fn current() -> Result<Self> {
        let release = fs::read_to_string("/proc/sys/kernel/osrelease")?
            .trim()
            .to_string();
        let proc_version = fs::read("/proc/version")?;
        if release.is_empty() || proc_version.is_empty() {
            return Err(invalid("running-kernel identity unavailable"));
        }
        Ok(Self {
            release,
            proc_version_sha256: format!("{:x}", Sha256::digest(&proc_version)),
        })
    }
}

/// Operator-installed, direct-evidence exclusion of a second live KFD GPU. Bound
/// to the exact running kernel, the selected compute card and the foreign card
/// set, and the exclusion policy. Carried on `HostAuthority`, so it travels the
/// same install/verify trust path as every other authoritative device fact.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KfdExclusion {
    pub schema_version: u32,
    pub policy: String,
    pub kernel: KernelIdentity,
    pub selected_pci: String,
    pub foreign_pcis: Vec<String>,
}

impl KfdExclusion {
    /// Canonical self-consistency, independent of any observed topology.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 || self.policy != KFD_EXCLUSION_POLICY {
            return Err(invalid("KFD exclusion receipt version/policy mismatch"));
        }
        if pci_key(&self.selected_pci)? != self.selected_pci {
            return Err(invalid("KFD exclusion selected PCI must be canonical"));
        }
        if self.kernel.release.is_empty()
            || self.kernel.proc_version_sha256.len() != 64
            || !self
                .kernel
                .proc_version_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid("KFD exclusion kernel identity required"));
        }
        let mut foreign = BTreeSet::new();
        if self.foreign_pcis.is_empty() || self.foreign_pcis.len() > 64 {
            return Err(invalid("one or more foreign KFD PCI cards required"));
        }
        for pci in &self.foreign_pcis {
            if pci_key(pci)? != *pci || *pci == self.selected_pci || !foreign.insert(pci) {
                return Err(invalid(
                    "foreign KFD PCI cards must be canonical, distinct and exclude the selected card",
                ));
            }
        }
        Ok(())
    }

    /// Verify the receipt against a live observation: exact running kernel, the
    /// exact selected card, and the exact foreign-card set. Any drift refuses.
    pub fn verify(
        &self,
        kernel: &KernelIdentity,
        selected_pci: &str,
        foreign: &BTreeSet<String>,
    ) -> Result<()> {
        self.validate()?;
        if &self.kernel != kernel {
            return Err(Error::Unqualified(
                "KFD exclusion receipt is bound to a different running kernel".into(),
            ));
        }
        if self.selected_pci != selected_pci
            || self.foreign_pcis.iter().cloned().collect::<BTreeSet<_>>() != *foreign
        {
            return Err(Error::Unqualified(
                "KFD exclusion receipt does not match the observed selected/foreign KFD cards"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// Qualification candidate; mounting a shared KFD is limited to a host with
/// exactly one live GPU topology node. No visibility variable grants access.
#[derive(Debug, Serialize)]
pub struct HipSandbox {
    pub identity: HipIdentity,
    drm: DrmSandbox,
}

impl HipSandbox {
    pub fn resolve(pci: &str, uuid: &str) -> Result<Self> {
        let identity = HipIdentity::resolve(pci)?;
        if identity.backend_uuid != uuid {
            return Err(invalid("approved HIP UUID differs from PCI/KFD identity"));
        }
        Ok(Self {
            identity,
            drm: DrmSandbox::resolve(pci)?,
        })
    }

    pub fn apply(&self, command: &mut Command) {
        self.drm.apply_compute(command);
        command.args(["--dev-bind", "/dev/kfd", "/dev/kfd"]);
        for path in [
            "/sys/devices/virtual/kfd/kfd/topology",
            "/sys/devices/system/cpu",
            "/sys/devices/system/node",
        ] {
            command.args(["--ro-bind", path, path]);
        }
    }
}

fn attribute(path: &Path) -> Result<String> {
    let input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !input.metadata()?.is_file() {
        return Err(invalid("KFD identity attributes must be regular files"));
    }
    let mut bytes = Vec::new();
    input.take(16385).read_to_end(&mut bytes)?;
    if bytes.len() > 16384 {
        return Err(invalid("bounded KFD identity attributes required"));
    }
    String::from_utf8(bytes).map_err(|_| invalid("KFD identity encoding"))
}

fn number(path: &Path) -> Result<u64> {
    attribute(path)?
        .trim()
        .parse()
        .map_err(|_| invalid("KFD identity number"))
}

fn properties(path: &Path) -> Result<BTreeMap<String, u64>> {
    let mut properties = BTreeMap::new();
    for line in attribute(path)?.lines() {
        let mut fields = line.split_whitespace();
        let name = fields.next().ok_or_else(|| invalid("KFD property name"))?;
        let value = fields
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| invalid("KFD property value"))?;
        if fields.next().is_some() || properties.insert(name.into(), value).is_some() {
            return Err(invalid(
                "unique KFD properties without trailing fields required",
            ));
        }
    }
    Ok(properties)
}

impl HipIdentity {
    pub fn verify_filter_receipt(&self, receipt: &serde_json::Value) -> Result<()> {
        let compiled = receipt["compiled_hip_version"]
            .as_u64()
            .filter(|v| *v > 0)
            .ok_or_else(|| invalid("compiled filter HIP version missing"))?;
        if receipt["adapter"] != "Viskores"
            || receipt["backend"] != "hip"
            || receipt["pci"] != self.pci
            || receipt["backend_uuid"] != self.backend_uuid
            || receipt["architecture"] != self.architecture
            || receipt["compiled_architecture"] != self.architecture
            || receipt["hip_runtime_version"].as_u64() != Some(compiled)
            || receipt["hip_driver_version"].as_u64() != Some(compiled)
            || receipt["source_revision"] != "7c0494a68bff379d32d6b1fbaa3d10d27a73af54"
            || receipt["viskores_revision"] != "521f3b72aabe0bf37e9972975700df27adbbae71"
            || receipt["kokkos_revision"] != "6ecdf605e0f7639adec599d25cf0e206d7b8f9f5"
            || receipt["executed"] != true
            || receipt["software_fallback"] != false
            || receipt["precision"] != "float64"
            || receipt["gpu_kernel_completion_verified"] != true
            || receipt["hip_dispatches"].as_u64().is_none_or(|n| n == 0)
        {
            return Err(Error::Unqualified(
                "filter receipt differs from exact HIP device/source/ABI or completion evidence"
                    .into(),
            ));
        }
        Ok(())
    }
    pub fn verify_receipt(&self, receipt: &serde_json::Value) -> Result<()> {
        let compiled = receipt["compiled_hip_version"]
            .as_u64()
            .filter(|v| *v > 0)
            .ok_or_else(|| invalid("compiled HIP version missing"))?;
        if receipt["adapter"] != "OpenLB"
            || receipt["backend"] != "hip"
            || receipt["pci"] != self.pci
            || receipt["backend_uuid"] != self.backend_uuid
            || receipt["architecture"] != self.architecture
            || receipt["compiled_architecture"] != self.architecture
            || receipt["hip_runtime_version"].as_u64() != Some(compiled)
            || receipt["hip_driver_version"].as_u64() != Some(compiled)
            || receipt["source_revision"] != "145cd54810b468f4b6fd3ed86b10644264841578"
            || receipt["executed"] != true
            || receipt["software_fallback"] != false
            || receipt["precision"] != "float64"
            || receipt["gpu_blocks"] != 1
            || receipt["gpu_kernel_completion_verified"] != true
        {
            return Err(Error::Unqualified(
                "HIP receipt differs from exact device/source/compiled-runtime identity; no fallback".into(),
            ));
        }
        Ok(())
    }

    pub fn resolve(pci: &str) -> Result<Self> {
        let drm = DrmSandbox::resolve(pci)?;
        if attribute(&drm.sysfs_device.join("vendor"))?.trim() != "0x1002"
            || fs::canonicalize(drm.sysfs_device.join("driver"))?
                .file_name()
                .and_then(|s| s.to_str())
                != Some("amdgpu")
        {
            return Err(invalid(
                "HIP requires the selected AMD device bound to amdgpu",
            ));
        }
        let kfd = fs::symlink_metadata("/dev/kfd")?;
        if !kfd.file_type().is_char_device() {
            return Err(invalid("KFD must be a direct character-device node"));
        }
        let kfd_sys = Path::new("/sys/dev/char").join(format!(
            "{}:{}",
            libc::major(kfd.rdev()),
            libc::minor(kfd.rdev())
        ));
        if fs::canonicalize(kfd_sys)? != Path::new("/sys/devices/virtual/kfd/kfd") {
            return Err(invalid("KFD character-device/sysfs mismatch"));
        }
        // A second live KFD GPU is admitted only by a verified exclusion receipt
        // installed on the authoritative same-user policy; this is the same trust
        // path as every other authoritative device fact, never ad-hoc file trust.
        let kernel = KernelIdentity::current()?;
        // A missing or unreadable policy store means no installed exclusion, so
        // the single-KFD-GPU policy applies and a second live GPU fails closed.
        let exclusion = crate::admission::installed_kfd_exclusion().unwrap_or(None);
        Self::from_topology_checked(
            Path::new("/sys/devices/virtual/kfd/kfd/topology"),
            pci,
            libc::minor(fs::metadata(&drm.node)?.rdev()),
            attribute(&drm.sysfs_device.join("unique_id"))?.trim(),
            Some(&kernel),
            exclusion.as_ref(),
        )
    }

    /// Single-KFD-GPU parse: rejects any second live GPU. Preserved for callers
    /// and tests that assert the default refusal with no exclusion receipt.
    pub fn from_topology(root: &Path, pci: &str, minor: u32, pci_unique_id: &str) -> Result<Self> {
        Self::from_topology_checked(root, pci, minor, pci_unique_id, None, None)
    }

    /// Parse a complete observed topology. A second live KFD GPU is rejected
    /// unless a verified exclusion receipt binds the exact kernel, selected card
    /// and foreign-card set; visibility variables are never access enforcement.
    pub fn from_topology_checked(
        root: &Path,
        pci: &str,
        minor: u32,
        pci_unique_id: &str,
        kernel: Option<&KernelIdentity>,
        exclusion: Option<&KfdExclusion>,
    ) -> Result<Self> {
        if pci_key(pci)? != pci {
            return Err(invalid("canonical HIP PCI identity required"));
        }
        let unique_id = u64::from_str_radix(pci_unique_id, 16)
            .map_err(|_| invalid("PCI unique ID encoding"))?;
        if unique_id == 0 || pci_unique_id != format!("{unique_id:016x}") {
            return Err(invalid("nonzero canonical PCI unique ID required"));
        }
        let generation = number(&root.join("generation_id"))?;
        let mut gpu = None;
        let mut foreign = BTreeSet::new();
        let mut count = 0;
        for entry in fs::read_dir(root.join("nodes"))? {
            count += 1;
            if count > 256 {
                return Err(invalid("bounded KFD topology nodes required"));
            }
            let entry = entry?;
            let name = entry.file_name();
            let index: u32 = name
                .to_str()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| invalid("KFD node index"))?;
            if name != index.to_string().as_str() || !entry.file_type()?.is_dir() {
                return Err(invalid("direct canonical KFD node directories required"));
            }
            let id = number(&entry.path().join("gpu_id"))?;
            let p = properties(&entry.path().join("properties"))?;
            let simd = p
                .get("simd_count")
                .ok_or_else(|| invalid("KFD SIMD count missing"))?;
            if id == 0 && *simd == 0 {
                continue;
            }
            let get = |key| {
                p.get(key)
                    .copied()
                    .ok_or_else(|| invalid(format!("KFD {key} missing")))
            };
            let domain = get("domain")?;
            let location = get("location_id")?;
            if id == 0
                || id > u32::MAX as u64
                || domain > u16::MAX as u64
                || location > u16::MAX as u64
            {
                return Err(invalid("bounded KFD GPU identity required"));
            }
            let node_pci = format!(
                "{domain:04x}:{:02x}:{:02x}.{}",
                location >> 8,
                (location >> 3) & 31,
                location & 7
            );
            // A live GPU node that is not the selected card is a foreign compute
            // device. Its exact PCI is recorded for the exclusion-receipt check;
            // the selected card still gets the full identity correlation below.
            if node_pci != pci {
                if !foreign.insert(node_pci) {
                    return Err(invalid("duplicate foreign KFD PCI card"));
                }
                continue;
            }
            if gpu.is_some() {
                return Err(invalid("duplicate selected KFD PCI card"));
            }
            let target = get("gfx_target_version")?;
            let architecture = format!(
                "gfx{}{:x}{:x}",
                target / 10000,
                target / 100 % 100,
                target % 100
            );
            if *simd == 0
                || get("vendor_id")? != 0x1002
                || get("drm_render_minor")? != u64::from(minor)
                || get("unique_id")? != unique_id
                || target == 0
                || target / 100 % 100 > 15
                || target % 100 > 15
            {
                return Err(invalid(
                    "HIP PCI/KFD/DRM/UUID/architecture identity mismatch",
                ));
            }
            // ROCr formats UniqueID as 16 ASCII hex characters; CLR copies
            // those characters into hipUUID. Our adapter hex-encodes its bytes.
            let encoded: String = format!("{unique_id:016x}")
                .bytes()
                .map(|b| format!("{b:02x}"))
                .collect();
            gpu = Some(Self {
                pci: pci.into(),
                render_node: format!("/dev/dri/renderD{minor}"),
                backend_uuid: format!("GPU-{encoded}"),
                architecture,
                kfd_node: index,
                kfd_gpu_id: id as u32,
                unique_id,
                generation,
                qualified: false,
                applied_policy: HIP_SANDBOX_POLICY.into(),
            });
        }
        if number(&root.join("generation_id"))? != generation {
            return Err(invalid("KFD topology changed during identity correlation"));
        }
        let mut gpu = gpu.ok_or_else(|| {
            Error::Unqualified("selected HIP device has no live KFD topology identity".into())
        })?;
        if !foreign.is_empty() {
            let kernel = kernel.ok_or_else(|| {
                Error::Unqualified(
                    "single-KFD-GPU policy rejects another exposed compute device; multi-GPU exclusion unqualified".into(),
                )
            })?;
            let receipt = exclusion.ok_or_else(|| {
                Error::Unqualified(
                    "single-KFD-GPU policy rejects another exposed compute device; multi-GPU exclusion unqualified".into(),
                )
            })?;
            receipt.verify(kernel, pci, &foreign)?;
            gpu.applied_policy = KFD_EXCLUSION_POLICY.into();
        }
        Ok(gpu)
    }
}
