//! Compiled source locks, not installed-package, sandbox or numerical evidence.
use crate::{Result, contracts::invalid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlakeSource {
    pub revision: String,
    pub nar_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildProvenance {
    pub schema_version: u32,
    pub package_version: String,
    pub lockfiles_sha256: BTreeMap<String, String>,
    pub flake_inputs: BTreeMap<String, FlakeSource>,
    pub fleetix_contract_sha256: String,
    pub sqlite_version: String,
    pub qualification: String,
    pub executed: bool,
}

pub fn inspect() -> Result<BuildProvenance> {
    let flake = include_bytes!("../flake.lock");
    let lock: serde_json::Value = serde_json::from_slice(flake)?;
    let root = lock["root"]
        .as_str()
        .ok_or_else(|| invalid("compiled flake root absent"))?;
    let inputs = lock["nodes"][root]["inputs"]
        .as_object()
        .ok_or_else(|| invalid("compiled flake inputs absent"))?;
    let mut flake_inputs = BTreeMap::new();
    // Nix lock nodes are graph identities, not input names. Resolve root edges;
    // never consult the caller's checkout, a registry or the Nix evaluator.
    // https://nix.dev/manual/nix/2.33/command-ref/new-cli/nix3-flake.html#lock-files
    for (name, node) in inputs {
        let node = node
            .as_str()
            .ok_or_else(|| invalid("compiled root input must select a locked source"))?;
        let source = &lock["nodes"][node]["locked"];
        let field = |key: &str| {
            source[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid(format!("compiled input {name} lacks {key}")))
        };
        flake_inputs.insert(
            name.clone(),
            FlakeSource {
                revision: field("rev")?,
                nar_hash: field("narHash")?,
            },
        );
    }
    Ok(BuildProvenance {
        schema_version: 1,
        package_version: env!("CARGO_PKG_VERSION").into(),
        flake_inputs,
        lockfiles_sha256: [
            ("flake.lock", flake.as_slice()),
            ("Cargo.lock", include_bytes!("../Cargo.lock").as_slice()),
            ("uv.lock", include_bytes!("../uv.lock").as_slice()),
        ]
        .into_iter()
        .map(|(name, bytes)| (name.into(), format!("{:x}", Sha256::digest(bytes))))
        .collect(),
        fleetix_contract_sha256: crate::contracts::fleetix_digest(),
        sqlite_version: rusqlite::version().into(),
        qualification: "not_assessed".into(),
        executed: false,
    })
}
