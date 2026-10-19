//! Immutable per-job execution identities, independent of historical approvals.
use crate::{Error, Result, contracts::*};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Component, Path, PathBuf},
};

pub const SANDBOX_POLICY: &str = "harbor-cad-native-v2";
pub const HIP_SANDBOX_POLICY: &str = "harbor-cad-native-hip-single-kfd-v1";
pub const PRESENTATION_SANDBOX_POLICY: &str = "harbor-cad-presentation-v1";
pub const FILTER_SANDBOX_POLICY: &str = "harbor-cad-filter-hip-single-kfd-v1";
pub const FEM_SANDBOX_POLICY: &str = "harbor-cad-fem-cpu-v1";
pub const THERMAL_SANDBOX_POLICY: &str = "harbor-cad-thermal-cpu-v1";
pub const CAD_MESH_SANDBOX_POLICY: &str = "harbor-cad-cad-mesh-cpu-v1";
pub const FEM_IMPORTED_SANDBOX_POLICY: &str = "harbor-cad-fem-imported-cpu-v1";
pub const WETTING_SANDBOX_POLICY: &str = "harbor-cad-wetting-cpu-v1";
pub const CONTACT_SANDBOX_POLICY: &str = crate::contact::SANDBOX_POLICY;

fn sandbox_policy(plan: &ExecutionPlan) -> &'static str {
    if plan.retained_cooling.is_some() {
        crate::cooling_execution::SANDBOX_POLICY
    } else if plan.cad_transport.is_some() {
        crate::cad_transport::SANDBOX_POLICY
    } else if plan.atmospheric_transport.is_some() {
        crate::atmospheric_transport::SANDBOX_POLICY
    } else if plan.atmosphere.is_some() {
        crate::atmosphere::SANDBOX_POLICY
    } else if plan.spectral.is_some() {
        crate::radiation::SANDBOX_POLICY
    } else if plan.thermal_contact.is_some() {
        "harbor-cad-thermal-contact-cpu-v1"
    } else if plan.freezing.is_some() {
        crate::freezing::SANDBOX_POLICY
    } else if plan.contact.is_some() {
        CONTACT_SANDBOX_POLICY
    } else if plan.wetting.is_some() {
        WETTING_SANDBOX_POLICY
    } else if plan.imported_fem.is_some() {
        FEM_IMPORTED_SANDBOX_POLICY
    } else if plan.cad_source.is_some() {
        CAD_MESH_SANDBOX_POLICY
    } else if plan.thermal.is_some() {
        THERMAL_SANDBOX_POLICY
    } else if plan.fem.is_some() {
        FEM_SANDBOX_POLICY
    } else if plan.filter.is_some() {
        FILTER_SANDBOX_POLICY
    } else if plan.source.is_some() {
        PRESENTATION_SANDBOX_POLICY
    } else if plan.stages.iter().any(|stage| {
        stage
            .selection
            .as_ref()
            .is_some_and(|s| s.role == Role::Compute && s.backend == "hip")
    }) {
        HIP_SANDBOX_POLICY
    } else {
        SANDBOX_POLICY
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

pub fn packaged_file(path: &Path) -> Result<PathBuf> {
    if !path.starts_with("/nix/store")
        || path.components().count() < 4
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err(invalid(
            "packaged path must remain inside one immutable Nix store object",
        ));
    }
    let canonical = fs::canonicalize(path)?;
    if !canonical.starts_with("/nix/store") || !canonical.is_file() {
        return Err(invalid(
            "packaged path resolves outside the immutable store or is not a regular file",
        ));
    }
    Ok(canonical)
}

impl FileIdentity {
    pub fn capture(path: &Path, packaged: bool) -> Result<Self> {
        let path = if packaged {
            packaged_file(path)?
        } else {
            fs::canonicalize(path)?
        };
        let mut input = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)?;
        let metadata = input.metadata()?;
        if !metadata.is_file() || metadata.len() > 1024 * 1024 * 1024 {
            return Err(invalid("bounded regular executable/runtime required"));
        }
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        let mut total = 0u64;
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > metadata.len() {
                return Err(invalid("execution file changed while hashing"));
            }
            hash.update(&buffer[..count]);
        }
        if total != metadata.len() {
            return Err(invalid("execution file changed while hashing"));
        }
        Ok(Self {
            path: path
                .to_str()
                .ok_or_else(|| invalid("UTF-8 execution path required"))?
                .into(),
            sha256: format!("{:x}", hash.finalize()),
            bytes: total,
        })
    }
    pub fn verify(&self, packaged: bool) -> Result<()> {
        let observed = Self::capture(Path::new(&self.path), packaged)?;
        if digest(&observed)? != digest(self)? {
            return Err(invalid(
                "bound execution file identity changed; no automatic upgrade",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBinding {
    pub schema_version: u32,
    pub runner_protocol: u32,
    pub sandbox_policy: String,
    pub plan_digest: String,
    pub host_profile_digest: String,
    pub runner: FileIdentity,
    pub native_runtime: Option<FileIdentity>,
    pub native_files: BTreeMap<String, FileIdentity>,
}

/// A read-only submission check, never an approval or a capacity reservation.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPreflight {
    pub schema_version: u32,
    pub execution_binding: ExecutionBinding,
    pub peak_ram_bytes: u64,
    pub disk_reservation_bytes: u64,
    pub execution_approved: bool,
    pub resources_reserved: bool,
    pub executed: bool,
    pub qualification: String,
}

impl ExecutionBinding {
    pub fn capture(
        plan: &ExecutionPlan,
        profile: &HostExecutionProfile,
        runner: &Path,
        native_files: BTreeMap<String, String>,
    ) -> Result<Self> {
        Ok(Self {
            schema_version: 1,
            runner_protocol: PROTOCOL_VERSION,
            sandbox_policy: sandbox_policy(plan).into(),
            plan_digest: plan.id()?,
            host_profile_digest: digest(profile)?,
            runner: FileIdentity::capture(runner, profile.service_mode == "systemd")?,
            native_runtime: profile
                .native_runtime
                .as_ref()
                .map(|p| FileIdentity::capture(Path::new(p), true))
                .transpose()?,
            native_files: native_files
                .iter()
                .map(|(k, p)| Ok((k.clone(), FileIdentity::capture(Path::new(p), true)?)))
                .collect::<Result<_>>()?,
        })
    }
    pub fn verify(&self, plan: &ExecutionPlan, profile: &HostExecutionProfile) -> Result<()> {
        if self.schema_version != 1
            || self.runner_protocol != PROTOCOL_VERSION
            || self.sandbox_policy != sandbox_policy(plan)
        {
            return Err(Error::Unqualified(
                "unsupported execution binding/policy version; no automatic relaunch".into(),
            ));
        }
        if self.plan_digest != plan.id()? || self.host_profile_digest != digest(profile)? {
            return Err(invalid(
                "execution binding differs from immutable plan/profile",
            ));
        }
        self.runner.verify(profile.service_mode == "systemd")?;
        match (&self.native_runtime, &profile.native_runtime) {
            (None, None) => {}
            (Some(runtime), Some(path))
                if packaged_file(Path::new(path))? == Path::new(&runtime.path) =>
            {
                runtime.verify(true)?
            }
            _ => return Err(invalid("execution binding runtime/profile mismatch")),
        }
        for file in self.native_files.values() {
            file.verify(true)?;
        }
        Ok(())
    }
    pub fn verify_running_runner(&self, profile: &HostExecutionProfile) -> Result<()> {
        let current =
            FileIdentity::capture(&std::env::current_exe()?, profile.service_mode == "systemd")?;
        if digest(&current)? != digest(&self.runner)? {
            return Err(invalid("running executable differs from bound job runner"));
        }
        Ok(())
    }
}
