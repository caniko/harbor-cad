//! Explicit execution authorization, separate from historical approval schemas.
use crate::{Result, contracts::*, devices::KfdExclusion, execution::ExecutionBinding};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilesystemBudget {
    pub root: String,
    pub max_bytes: u64,
    pub free_headroom_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardBudget {
    pub pci: String,
    pub max_vram_bytes: u64,
    pub headroom_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteOverride {
    pub selection: GpuSelection,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostAuthority {
    pub schema_version: u32,
    pub fleetix_revision: String,
    pub fleetix_contract_digest: String,
    pub max_ram_bytes: u64,
    pub ram_headroom_bytes: u64,
    pub filesystems: Vec<FilesystemBudget>,
    pub cards: Vec<CardBudget>,
    pub routes: Vec<GpuSelection>,
    pub allowed_devices: Vec<GpuSelection>,
    pub overrides: Vec<RouteOverride>,
    pub native_runtimes: Vec<String>,
    pub allowed_input_roots: Vec<String>,
    /// Operator-installed direct-evidence exclusion of a second live KFD GPU.
    /// Absent keeps the single-KFD-GPU policy, which refuses any second GPU.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kfd_exclusion: Option<KfdExclusion>,
}

fn absolute(path: &str) -> bool {
    Path::new(path).is_absolute()
        && !Path::new(path)
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        && path.len() <= 4096
}

fn capacity(total: u64, headroom: u64) -> bool {
    total > 0 && total <= i64::MAX as u64 && headroom < total
}

fn selection_valid(selection: &GpuSelection) -> Result<()> {
    if crate::resources::pci_key(&selection.pci)? != selection.pci {
        return Err(invalid(
            "authority requires canonical lowercase PCI identities",
        ));
    }
    let supported = match selection.role {
        Role::Compute => {
            ["hip", "cuda"].contains(&selection.backend.as_str())
                && selection.backend_uuid.as_deref().is_some_and(token)
        }
        Role::Render => selection.backend == "egl" && selection.backend_uuid.is_none(),
        Role::Media => selection.backend == "vaapi" && selection.backend_uuid.is_none(),
    };
    if !supported {
        return Err(invalid("authority operation/backend/UUID mismatch"));
    }
    Ok(())
}

impl HostAuthority {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.fleetix_revision != FLEETIX_REV
            || self.fleetix_contract_digest != fleetix_digest()
            || !capacity(self.max_ram_bytes, self.ram_headroom_bytes)
        {
            return Err(invalid("authority version/Fleetix/capacity drift"));
        }
        for size in [
            self.filesystems.len(),
            self.cards.len(),
            self.allowed_devices.len(),
            self.overrides.len(),
            self.native_runtimes.len(),
            self.allowed_input_roots.len(),
        ] {
            if size > 128 {
                return Err(invalid("bounded authority lists required"));
            }
        }
        let mut roots = BTreeSet::new();
        for budget in &self.filesystems {
            if !absolute(&budget.root)
                || !roots.insert(&budget.root)
                || budget.max_bytes == 0
                || budget.max_bytes > i64::MAX as u64
                || budget.free_headroom_bytes > i64::MAX as u64
            {
                return Err(invalid(
                    "unique absolute filesystem roots and bounded budgets required",
                ));
            }
        }
        let mut cards = BTreeSet::new();
        for card in &self.cards {
            if crate::resources::pci_key(&card.pci)? != card.pci
                || !cards.insert(&card.pci)
                || !capacity(card.max_vram_bytes, card.headroom_bytes)
            {
                return Err(invalid(
                    "unique canonical cards and explicit VRAM headroom required",
                ));
            }
        }
        let mut allowed = BTreeSet::new();
        for selection in &self.allowed_devices {
            selection_valid(selection)?;
            if !cards.contains(&selection.pci) || !allowed.insert(digest(selection)?) {
                return Err(invalid(
                    "every unique allowed device requires one shared physical-card budget",
                ));
            }
        }
        let mut roles = BTreeSet::new();
        for route in &self.routes {
            if !roles.insert(serde_json::to_string(&route.role)?)
                || !allowed.contains(&digest(route)?)
            {
                return Err(invalid(
                    "one authoritative allowed route per operation role required",
                ));
            }
        }
        let mut overrides = BTreeSet::new();
        for record in &self.overrides {
            if record.reason.trim().is_empty()
                || record.reason.len() > 1024
                || !allowed.contains(&digest(&record.selection)?)
                || !overrides.insert(digest(&record.selection)?)
            {
                return Err(invalid(
                    "unique allowed override and explicit bounded reason required",
                ));
            }
        }
        for paths in [&self.allowed_input_roots, &self.native_runtimes] {
            let mut unique = BTreeSet::new();
            if paths.iter().any(|p| !absolute(p) || !unique.insert(p)) {
                return Err(invalid(
                    "unique absolute authority input/runtime paths required",
                ));
            }
        }
        if self.native_runtimes.iter().any(|p| {
            !Path::new(p).starts_with("/nix/store") || Path::new(p).components().count() < 4
        }) {
            return Err(invalid(
                "authority runtime manifests must identify immutable store files",
            ));
        }
        if let Some(exclusion) = &self.kfd_exclusion {
            exclusion.validate()?;
            // The exclusion's selected card must be an allowed HIP compute card:
            // the receipt cannot authorize a device the authority never allowed.
            if !self.allowed_devices.iter().any(|d| {
                d.role == Role::Compute && d.backend == "hip" && d.pci == exclusion.selected_pci
            }) {
                return Err(invalid(
                    "KFD exclusion selected card must be an allowed HIP compute device",
                ));
            }
        }
        Ok(())
    }

    pub fn authorize_selection(&self, selected: &GpuSelection, vram: u64) -> Result<()> {
        self.validate()?;
        let key = digest(selected)?;
        if !self
            .allowed_devices
            .iter()
            .any(|s| digest(s).is_ok_and(|d| d == key))
            || !self
                .routes
                .iter()
                .chain(self.overrides.iter().map(|o| &o.selection))
                .any(|s| digest(s).is_ok_and(|d| d == key))
        {
            return Err(invalid(
                "device/route disabled without an exact recorded override",
            ));
        }
        let card = self
            .cards
            .iter()
            .find(|c| c.pci == selected.pci)
            .ok_or_else(|| invalid("physical-card budget missing"))?;
        if vram > card.max_vram_bytes - card.headroom_bytes {
            return Err(crate::Error::Resource(
                "stage exceeds physical-card VRAM budget/headroom; parameters preserved".into(),
            ));
        }
        Ok(())
    }

    pub fn authorize(&self, plan: &ExecutionPlan, profile: &HostExecutionProfile) -> Result<()> {
        self.validate()?;
        if plan.peak_ram() > self.max_ram_bytes - self.ram_headroom_bytes {
            return Err(crate::Error::Resource(
                "plan exceeds authoritative RAM capacity/headroom".into(),
            ));
        }
        if !self
            .allowed_input_roots
            .contains(&profile.allowed_input_root)
            || profile
                .native_runtime
                .as_ref()
                .is_some_and(|r| !self.native_runtimes.contains(r))
        {
            return Err(invalid("profile input root/runtime is not authorized"));
        }
        for stage in &plan.stages {
            if let Some(selected) = &stage.selection {
                self.authorize_selection(selected, stage.vram_bytes)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionAuthorization {
    pub schema_version: u32,
    pub plan_digest: String,
    pub host_profile_digest: String,
    pub execution_binding_digest: String,
    pub authority: HostAuthority,
}

impl ExecutionAuthorization {
    pub fn capture(
        plan: &ExecutionPlan,
        profile: &HostExecutionProfile,
        binding: &ExecutionBinding,
        authority: &HostAuthority,
    ) -> Result<Self> {
        authority.authorize(plan, profile)?;
        Ok(Self {
            schema_version: 1,
            plan_digest: plan.id()?,
            host_profile_digest: digest(profile)?,
            execution_binding_digest: digest(binding)?,
            authority: authority.clone(),
        })
    }
    pub fn verify(
        &self,
        plan: &ExecutionPlan,
        profile: &HostExecutionProfile,
        binding: &ExecutionBinding,
    ) -> Result<()> {
        if self.schema_version != 1
            || self.plan_digest != plan.id()?
            || self.host_profile_digest != digest(profile)?
            || self.execution_binding_digest != digest(binding)?
        {
            return Err(invalid(
                "execution authorization version or immutable identity mismatch",
            ));
        }
        self.authority.authorize(plan, profile)
    }
}
