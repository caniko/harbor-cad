//! Steady enclosure conduction on imported general solids (docs/enclosure-thermal.md).
//!
//! One exclusive plan field, `cad_solids`, carries a multi-region BREP binding.
//! Without `thermal` it is a version-7 conformal tetrahedral mesh; with it, a
//! version-8 CalculiX steady conduction solve bound to a succeeded mesh job.
//! Physical inputs are explicit `PhysicalInput`s or W1 library materials and
//! are never defaulted; a missing one refuses planning.
use crate::{
    Result,
    cad_source::{self, hash},
    contracts::*,
    manufacturing::{Library, QuantityRange},
    materials::PhysicalInput,
    qualification::{EvidenceRecord, NumericalEvidence},
    science::Quantity,
    storage::{
        Store, commit_artifact, copy_verified, native_manifest, private_dir, publish_directory,
        safe_path, sync_directories,
    },
    worker::read_bounded,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub const MESH_FORMULATION: &str = "imported_general_solids";
pub const THERMAL_FORMULATION: &str = "enclosure_steady_conduction";
pub const MAX_REGIONS: usize = 8;
pub const MAX_ELEMENTS: u64 = 1_000_000;
const MAX_MESH_BYTES: u64 = 96 * 1024 * 1024;
const MAX_FIELD_BYTES: u64 = 256 * 1024 * 1024;
const SIGMA: f64 = 5.670374419e-8;
const RETAINED: &str = "retained-solids";
const INTENT: &str = "cad-solids-retention.json";
const GMSH_SOURCE: &str = "be3f66f225d27ba9fa014f07e83169285da8a051b0e8ab7103d88066b39bdd3e";
const CALCULIX_SOURCE: &str = "9c88385c10fb04f5dc6c4e98027a51bebdd8aee3920e05190d6c1dd08357d6e7";
/// CalculiX C3D4/C3D10 faces F1..F4 (corner indices) and C3D10 midside nodes.
const FACES: [[usize; 3]; 4] = [[0, 1, 2], [0, 3, 1], [1, 3, 2], [2, 3, 0]];
const FACE_MIDS: [[usize; 3]; 4] = [[4, 5, 6], [7, 8, 4], [8, 9, 5], [9, 7, 6]];
const EDGES: [[usize; 2]; 6] = [[0, 1], [1, 2], [2, 0], [0, 3], [1, 3], [2, 3]];
const SANDBOX_CHECKS: [&str; 10] = [
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
];

fn positive(value: f64, high: f64) -> bool {
    value.is_finite() && value > 0. && value <= high
}

fn text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 4096
}

fn rigid(r: &[f64; 16]) -> bool {
    let rotation = (0..3).all(|i| {
        (0..3).all(|j| {
            let dot = (0..3).map(|k| r[4 * i + k] * r[4 * j + k]).sum::<f64>();
            (dot - f64::from(i == j)).abs() <= 1e-12
        })
    });
    let det = r[0] * (r[5] * r[10] - r[6] * r[9]) - r[1] * (r[4] * r[10] - r[6] * r[8])
        + r[2] * (r[4] * r[9] - r[5] * r[8]);
    r.iter().all(|v| v.is_finite())
        && r[12..] == [0., 0., 0., 1.]
        && rotation
        && (det - 1.).abs() <= 1e-12
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolidMesh {
    /// Gmsh `Mesh.CharacteristicLengthMax`, metres.
    pub characteristic_length_max_m: f64,
    /// 1 (C3D4) or 2 (C3D10).
    pub element_order: u32,
    pub max_elements: u64,
    /// Per-region |mesh - CAD| / CAD volume gate (corner-node tetrahedra).
    pub volume_relative_tolerance: f64,
    /// Minimum 6*sqrt(2)*V/l_rms^3 shape ratio (1 = regular tetrahedron).
    pub min_quality: f64,
    pub geometry_tolerance_m: f64,
}

impl SolidMesh {
    pub fn validate(&self) -> Result<()> {
        if !(1e-4..=1.).contains(&self.characteristic_length_max_m)
            || !matches!(self.element_order, 1 | 2)
            || !(1..=MAX_ELEMENTS).contains(&self.max_elements)
            || !positive(self.volume_relative_tolerance, 0.1)
            || !(self.min_quality.is_finite() && self.min_quality > 0. && self.min_quality < 1.)
            || !(1e-9..=1e-3).contains(&self.geometry_tolerance_m)
        {
            return Err(invalid(
                "bounded explicit mesh size, order (1|2), element budget and quality/volume gates required",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolidRegion {
    pub name: String,
    pub brep_sha256: String,
    pub brep_bytes: u64,
    pub bounds_m: [f64; 6],
    pub volume_m3: f64,
    pub source_transform: [f64; 16],
}

/// Exact native mesh descriptor; also embedded in the thermal descriptor.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolidsGeometry {
    pub schema_version: u32,
    pub synthetic: bool,
    pub backend: String,
    pub formulation: String,
    pub geometry_provenance: String,
    pub source_unit: String,
    pub scale_to_m: f64,
    pub placement_translation_unit: String,
    pub regions: Vec<SolidRegion>,
    pub mesh: SolidMesh,
}

impl SolidsGeometry {
    pub fn names(&self) -> Vec<&str> {
        self.regions.iter().map(|r| r.name.as_str()).collect()
    }
    /// Largest world extent over the characteristic size, as an integer level.
    pub fn refinement(&self) -> u32 {
        let extent = self
            .regions
            .iter()
            .flat_map(|r| [0, 2, 4].map(|i| r.bounds_m[i + 1] - r.bounds_m[i]))
            .fold(0., f64::max);
        (extent / self.mesh.characteristic_length_max_m)
            .ceil()
            .min(f64::from(u32::MAX)) as u32
    }
    pub fn validate(&self) -> Result<()> {
        self.mesh.validate()?;
        let names = self.names();
        if self.schema_version != 1
            || self.backend != "cpu"
            || self.formulation != MESH_FORMULATION
            || !text(&self.geometry_provenance)
            || self.source_unit != "mm"
            || self.scale_to_m != 0.001
            || self.placement_translation_unit != "mm"
            || !(1..=MAX_REGIONS).contains(&self.regions.len())
            || names.iter().any(|n| !token(n))
            || names.windows(2).any(|w| w[0] >= w[1])
        {
            return Err(invalid(
                "one to eight sorted named imported solids with millimetre source units required",
            ));
        }
        for region in &self.regions {
            let sizes = [0, 2, 4].map(|i| region.bounds_m[i + 1] - region.bounds_m[i]);
            if !hash(&region.brep_sha256)
                || !(1..=64 * 1024 * 1024).contains(&region.brep_bytes)
                || !region.bounds_m.iter().all(|v| v.is_finite())
                || sizes.iter().any(|s| *s <= 0.)
                || !region.volume_m3.is_finite()
                || region.volume_m3 <= 0.
                || region.volume_m3 > sizes.iter().product::<f64>() * (1. + 1e-9)
                || !rigid(&region.source_transform)
            {
                return Err(invalid(
                    "exact BREP identity, positive volume inside world bounds and rigid placement required",
                ));
            }
        }
        // A regular tetrahedron of edge h holds h^3/(6*sqrt(2)); fewer cells
        // than that lower bound can never fill the approved volume.
        let h = self.mesh.characteristic_length_max_m;
        let lower = self.regions.iter().map(|r| r.volume_m3).sum::<f64>()
            / (h.powi(3) / (6. * 2f64.sqrt()));
        if lower > self.mesh.max_elements as f64 {
            return Err(invalid(
                "element budget is below the regular-tetrahedron count for the approved volume and size",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CadSolids {
    pub schema_version: u32,
    pub job_id: String,
    pub science_id: String,
    pub execution_id: String,
    pub execution_binding_digest: String,
    pub authorization_digest: String,
    pub manifest: EvidenceRecord,
    pub region_evidence: EvidenceRecord,
    pub breps: Vec<ArtifactManifest>,
    pub geometry: SolidsGeometry,
}

impl CadSolids {
    pub fn validate(&self) -> Result<()> {
        self.geometry.validate()?;
        if self.schema_version != 1
            || uuid::Uuid::parse_str(&self.job_id).is_err()
            || [
                &self.science_id,
                &self.execution_id,
                &self.execution_binding_digest,
                &self.authorization_digest,
                &self.manifest.sha256,
                &self.region_evidence.sha256,
            ]
            .iter()
            .any(|h| !hash(h))
            || self.manifest.path != "brep-manifest.json"
            || !(1..=256 * 1024).contains(&self.manifest.bytes)
            || self.region_evidence.path != "regions.json"
            || !(1..=256 * 1024).contains(&self.region_evidence.bytes)
            || self.breps.len() != self.geometry.regions.len()
            || self.breps.iter().zip(&self.geometry.regions).any(|(b, r)| {
                b.schema_version != 1
                    || b.path != format!("{}.brep", r.name)
                    || b.format != "brep"
                    || b.sha256 != r.brep_sha256
                    || b.bytes != r.brep_bytes
                    || b.units.is_some()
                    || b.time_s.is_some()
                    || b.association.is_some()
            })
        {
            return Err(invalid(
                "immutable authorized named CAD solids and exact BREP bytes required",
            ));
        }
        Ok(())
    }
}

/// The exclusive `cad_solids` plan field.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CadSolidsRecipe {
    pub source: CadSolids,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal: Option<EnclosureThermalSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolidMeshRequest {
    pub source_job: String,
    /// Sorted unique registered region names; touching regions share interfaces.
    pub regions: Vec<String>,
    pub mesh: SolidMesh,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaterialInput {
    /// W1 `Library::builtin()` id, a design `route` `MATERIAL__PROCESS`, or a
    /// triple `MATERIAL__PROCESS__FINISH`; the finish (from the triple or the
    /// separate field, not both) is a `data/finishes.json` id whose sourced
    /// optics override (or, where missing, fall back to) the material's.
    Library {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finish: Option<String>,
    },
    Declared {
        conductivity: PhysicalInput<Quantity>,
        #[serde(default)]
        emissivity: Option<PhysicalInput<Quantity>>,
        #[serde(default)]
        solar_absorptance: Option<PhysicalInput<Quantity>>,
    },
}

/// Explicit solar model for exposed transparent glazing (`pc`, `pc-uv`,
/// `pmma`), whose sourced data carry no absorbed-solar value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransparentSolar {
    /// Conservative bound (modelling assumption): the full irradiance incident
    /// on the glazing's sun-facing exterior faces, G·max(0, n·s)·A, enters the
    /// internal air node. Nothing is reflected, absorbed in the pane or
    /// re-emitted outward, so the interior solar gain is an upper bound; the
    /// pane's own temperature is not bounded.
    DepositIncidentInCavity,
}

pub const TRANSPARENT_SOLAR_STATEMENT: &str = "conservative_bound: the full solar irradiance incident on sun-facing exterior faces of transparent glazing is deposited into the internal air node (no reflection, no absorption in the pane, no re-emission credit); an upper bound on interior solar gain, not on the pane temperature";

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolarInput {
    pub irradiance: PhysicalInput<Quantity>,
    /// Unit vector toward the sun, world frame.
    pub direction: [f64; 3],
    /// Required for a solar-loaded exposed transparent region; without it
    /// such a region refuses (no absorptance is defaulted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparent_solar: Option<TransparentSolar>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnclosureThermalInputs {
    pub schema_version: u32,
    pub materials: BTreeMap<String, MaterialInput>,
    pub heat_source_region: String,
    pub heat_source_power: PhysicalInput<Quantity>,
    pub ambient_temperature: PhysicalInput<Quantity>,
    pub film_coefficient: PhysicalInput<Quantity>,
    pub exposed_regions: Vec<String>,
    /// Effective cavity-surface coefficient to one well-mixed internal air node.
    pub internal_film_coefficient: PhysicalInput<Quantity>,
    #[serde(default)]
    pub solar: Option<SolarInput>,
    pub radiation: bool,
    pub energy_balance_tolerance: f64,
    /// Linear power-sweep mode: the design loop superposes solves at two
    /// heat-source powers, which is exact only for a linear model, so
    /// radiation (T^4) is refused. Constant films and solar keep it affine.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub power_sweep: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnclosureThermalRequest {
    pub mesh_job: String,
    pub spec: EnclosureThermalInputs,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedMaterial {
    pub source: String,
    pub conductivity_w_m_k: f64,
    pub emissivity: Option<f64>,
    pub solar_absorptance: Option<f64>,
    pub provenance: String,
    pub synthetic: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSolar {
    pub irradiance_w_m2: f64,
    pub direction: [f64; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparent_solar: Option<TransparentSolar>,
    /// Sorted exposed regions of transparent library glazing under `transparent_solar`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transparent_regions: Vec<String>,
}

/// Resolved SI model handed to the adapter; every value carries provenance.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThermalModel {
    pub synthetic: bool,
    pub materials: BTreeMap<String, ResolvedMaterial>,
    pub heat_source_region: String,
    pub heat_source_power_w: f64,
    pub ambient_k: f64,
    pub film_w_m2_k: f64,
    pub exposed_regions: Vec<String>,
    pub internal_film_w_m2_k: f64,
    pub solar: Option<ResolvedSolar>,
    pub radiation: bool,
    pub energy_balance_tolerance: f64,
    pub provenance: BTreeMap<String, String>,
}

impl ThermalModel {
    pub fn validate(&self, geometry: &SolidsGeometry) -> Result<()> {
        let names: BTreeSet<&str> = geometry.names().into_iter().collect();
        let materials: BTreeSet<&str> = self.materials.keys().map(String::as_str).collect();
        let exposed = &self.exposed_regions;
        if materials != names
            || !names.contains(self.heat_source_region.as_str())
            || !positive(self.heat_source_power_w, 1e5)
            || !(self.ambient_k.is_finite() && self.ambient_k > 150. && self.ambient_k <= 500.)
            || !positive(self.film_w_m2_k, 1e4)
            || !positive(self.internal_film_w_m2_k, 1e4)
            || !positive(self.energy_balance_tolerance, 1e-3)
            || exposed.is_empty()
            || exposed.windows(2).any(|w| w[0] >= w[1])
            || exposed.iter().any(|n| !names.contains(n.as_str()))
            || self.provenance.is_empty()
            || self.provenance.iter().any(|(k, v)| !text(k) || !text(v))
            || self.materials.values().any(|m| {
                !text(&m.source)
                    || !text(&m.provenance)
                    || !positive(m.conductivity_w_m_k, 1e4)
                    || [m.emissivity, m.solar_absorptance]
                        .into_iter()
                        .flatten()
                        .any(|v| !positive(v, 1.))
            })
            || (!self.synthetic && self.materials.values().any(|m| m.synthetic))
        {
            return Err(invalid(
                "resolved enclosure thermal model differs from approved regions, ranges or provenance",
            ));
        }
        let lacks = |pick: fn(&ResolvedMaterial) -> Option<f64>| {
            exposed
                .iter()
                .find(|n| pick(&self.materials[n.as_str()]).is_none())
        };
        if self.radiation
            && let Some(name) = lacks(|m| m.emissivity)
        {
            return Err(invalid(format!(
                "radiation requires a sourced emissivity for exposed region {name}; not defaulted"
            )));
        }
        if let Some(solar) = &self.solar {
            let norm = solar.direction.iter().map(|v| v * v).sum::<f64>().sqrt();
            if !positive(solar.irradiance_w_m2, 2000.) || norm.is_nan() || (norm - 1.).abs() > 1e-9
            {
                return Err(invalid(
                    "bounded solar irradiance and unit direction required",
                ));
            }
            let transparent = &solar.transparent_regions;
            if solar.transparent_solar.is_some() == transparent.is_empty()
                || transparent.windows(2).any(|w| w[0] >= w[1])
                || transparent.iter().any(|n| {
                    !exposed.contains(n) || self.materials[n.as_str()].solar_absorptance.is_some()
                })
            {
                return Err(invalid(
                    "transparent_solar requires sorted exposed transparent glazing regions without an absorptance",
                ));
            }
            if !transparent.is_empty() {
                let glazing = crate::conditions::Conditions::builtin()?.transparent_materials;
                if let Some(name) = transparent.iter().find(|n| {
                    let source = &self.materials[n.as_str()].source;
                    !glazing.iter().any(|g| source == &format!("library:{g}"))
                }) {
                    return Err(invalid(format!(
                        "region {name} is not transparent library glazing"
                    )));
                }
            }
            if let Some(name) = exposed.iter().find(|n| {
                !transparent.contains(n) && self.materials[n.as_str()].solar_absorptance.is_none()
            }) {
                return Err(invalid(format!(
                    "solar load requires a sourced solar absorptance for exposed region {name} (or transparent_solar for transparent glazing); not defaulted"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnclosureThermalSpec {
    pub schema_version: u32,
    pub mesh_job: String,
    pub mesh_plan_digest: String,
    pub mesh_sha256: String,
    pub thermal: ThermalModel,
}

impl EnclosureThermalSpec {
    pub fn validate(&self, source: &CadSolids) -> Result<()> {
        if self.schema_version != 1
            || uuid::Uuid::parse_str(&self.mesh_job).is_err()
            || !hash(&self.mesh_plan_digest)
            || !hash(&self.mesh_sha256)
        {
            return Err(invalid("succeeded approved mesh job binding required"));
        }
        self.thermal.validate(&source.geometry)
    }
}

/// Resolve the named solids of an authorized CAD job (original or variant).
pub fn solids(
    store: &Store,
    id: &str,
    regions: &[String],
    mesh: &SolidMesh,
) -> Result<(PathBuf, CadSolids)> {
    if regions.is_empty()
        || regions.len() > MAX_REGIONS
        || regions.iter().any(|r| !token(r))
        || regions.windows(2).any(|w| w[0] >= w[1])
    {
        return Err(invalid("one to eight sorted unique region names required"));
    }
    let verified = cad_source::verified_manifest(store, id)?;
    let mut breps = Vec::new();
    let mut solid_regions = Vec::new();
    for name in regions {
        let selected = verified
            .regions
            .iter()
            .find(|r| &r.region_name == name)
            .ok_or_else(|| invalid(format!("approved named BREP region {name} missing")))?;
        if selected.source_unit != "mm" || selected.scale_to_m != 0.001 {
            return Err(invalid("millimetre CAD source units required"));
        }
        let brep = cad_source::verified_brep(store, id, selected)?;
        solid_regions.push(SolidRegion {
            name: name.clone(),
            brep_sha256: brep.sha256.clone(),
            brep_bytes: brep.bytes,
            bounds_m: selected.bounds_m,
            volume_m3: selected.volume_m3,
            source_transform: selected.source_transform,
        });
        breps.push(brep);
    }
    let geometry = SolidsGeometry {
        schema_version: 1,
        synthetic: verified.synthetic,
        backend: "cpu".into(),
        formulation: MESH_FORMULATION.into(),
        geometry_provenance: format!(
            "authorized CAD job {id}, named regions {}; {}",
            regions.join(","),
            verified.plan.channel_case()?.geometry.source
        ),
        source_unit: "mm".into(),
        scale_to_m: 0.001,
        placement_translation_unit: "mm".into(),
        regions: solid_regions,
        mesh: mesh.clone(),
    };
    let bound = CadSolids {
        schema_version: 1,
        job_id: id.into(),
        science_id: verified.plan.science_id()?,
        execution_id: verified.plan.id()?,
        execution_binding_digest: digest(&verified.binding)?,
        authorization_digest: digest(&verified.authorization)?,
        manifest: verified.manifest,
        region_evidence: verified.report.evidence,
        breps,
        geometry,
    };
    bound.validate()?;
    Ok((store.job_dir(id)?, bound))
}

fn known(
    input: &PhysicalInput<Quantity>,
    name: &str,
    dimension: &str,
) -> Result<(f64, String, bool)> {
    input.validate()?;
    match input {
        PhysicalInput::Known {
            value,
            provenance,
            synthetic,
        } => Ok((value.si(dimension)?, provenance.clone(), *synthetic)),
        PhysicalInput::Missing { reason } => Err(invalid(format!(
            "physical input {name} is missing: {reason}"
        ))),
    }
}

fn optional(
    input: Option<&PhysicalInput<Quantity>>,
    name: &str,
) -> Result<(Option<f64>, Option<String>, bool)> {
    match input {
        Some(input @ PhysicalInput::Known { .. }) => {
            let (value, provenance, synthetic) = known(input, name, "dimensionless")?;
            Ok((Some(value), Some(provenance), synthetic))
        }
        Some(input) => input.validate().map(|()| (None, None, false)),
        None => Ok((None, None, false)),
    }
}

/// Conservative end of a sourced dimensionless library range; `None` when missing.
fn range(input: &PhysicalInput<QuantityRange>, high: bool) -> Result<Option<(f64, String)>> {
    Ok(match input {
        PhysicalInput::Known {
            value, provenance, ..
        } => {
            let (low, upper) = value.si("dimensionless")?;
            Some((if high { upper } else { low }, provenance.clone()))
        }
        PhysicalInput::Missing { .. } => None,
    })
}

fn material(region: &str, input: &MaterialInput, library: &Library) -> Result<ResolvedMaterial> {
    match input {
        MaterialInput::Library { id, finish } => {
            // A bare id, a design route `MATERIAL__PROCESS`, or a finished
            // triple `MATERIAL__PROCESS__FINISH` (then no separate finish).
            let (id, finish) = match crate::design_providers::split_route(id) {
                None if !id.contains("__") => (id.as_str(), finish.as_deref()),
                Some((material, _, None)) => (material, finish.as_deref()),
                Some((material, _, Some(triple))) if finish.is_none() => (material, Some(triple)),
                _ => {
                    return Err(invalid(format!(
                        "region {region}: library id must be MATERIAL, MATERIAL__PROCESS or MATERIAL__PROCESS__FINISH, with the finish named once"
                    )));
                }
            };
            let entry = library.material(id)?;
            let conductivity = match &entry.thermal_conductivity {
                PhysicalInput::Known {
                    value, provenance, ..
                } => (value.si("thermal_conductivity")?.0, provenance.clone()),
                PhysicalInput::Missing { reason } => {
                    return Err(invalid(format!(
                        "library material {id} for region {region} has no sourced conductivity: {reason}"
                    )));
                }
            };
            let (absorptance, emissivity) = match finish {
                Some(finish) => crate::conditions::Conditions::builtin()?
                    .surface_optics(id, finish)
                    .map_err(|e| invalid(format!("region {region}: {e}")))?,
                None => (entry.solar_absorptance.clone(), entry.emissivity.clone()),
            };
            let emissivity = range(&emissivity, false)?;
            let absorptance = range(&absorptance, true)?;
            let source = finish.map_or(format!("library:{id}"), |f| {
                format!("library:{id}+finish:{f}")
            });
            let mut provenance = format!(
                "W1 {source}; conservative ends (conductivity low, emissivity low, solar absorptance high); conductivity: {}",
                conductivity.1
            );
            for (label, value) in [
                ("emissivity", &emissivity),
                ("solar absorptance", &absorptance),
            ] {
                if let Some((_, source)) = value {
                    provenance.push_str(&format!("; {label}: {source}"));
                }
            }
            provenance.truncate(4096);
            Ok(ResolvedMaterial {
                source,
                conductivity_w_m_k: conductivity.0,
                emissivity: emissivity.map(|v| v.0),
                solar_absorptance: absorptance.map(|v| v.0),
                provenance,
                synthetic: false,
            })
        }
        MaterialInput::Declared {
            conductivity,
            emissivity,
            solar_absorptance,
        } => {
            let (k, mut provenance, mut synthetic) = known(
                conductivity,
                &format!("{region} conductivity"),
                "thermal_conductivity",
            )?;
            let (eps, eps_source, eps_synthetic) =
                optional(emissivity.as_ref(), &format!("{region} emissivity"))?;
            let (alpha, alpha_source, alpha_synthetic) = optional(
                solar_absorptance.as_ref(),
                &format!("{region} solar absorptance"),
            )?;
            for (label, value) in [
                ("emissivity", eps_source),
                ("solar absorptance", alpha_source),
            ] {
                if let Some(source) = value {
                    provenance.push_str(&format!("; {label}: {source}"));
                }
            }
            provenance.truncate(4096);
            synthetic |= eps_synthetic || alpha_synthetic;
            Ok(ResolvedMaterial {
                source: "declared".into(),
                conductivity_w_m_k: k,
                emissivity: eps,
                solar_absorptance: alpha,
                provenance,
                synthetic,
            })
        }
    }
}

/// Planning-time resolution of declared inputs; refuses rather than defaults.
pub fn resolve(inputs: &EnclosureThermalInputs, geometry: &SolidsGeometry) -> Result<ThermalModel> {
    if inputs.schema_version != 1 {
        return Err(invalid(
            "enclosure thermal inputs schema_version 1 required",
        ));
    }
    if inputs.power_sweep && inputs.radiation {
        return Err(invalid(
            "power_sweep superposes solves linearly in power and requires radiation off (radiation is nonlinear)",
        ));
    }
    let names: BTreeSet<&str> = geometry.names().into_iter().collect();
    for region in inputs.materials.keys() {
        if !names.contains(region.as_str()) {
            return Err(invalid(format!(
                "material for region {region} which is not an approved meshed region"
            )));
        }
    }
    if let Some(region) = names.iter().find(|n| !inputs.materials.contains_key(**n)) {
        return Err(invalid(format!("region {region} has no material")));
    }
    let library = Library::builtin()?;
    let materials = inputs
        .materials
        .iter()
        .map(|(region, input)| Ok((region.clone(), material(region, input, &library)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut provenance = BTreeMap::new();
    let mut synthetic = materials.values().any(|m| m.synthetic);
    let mut scalar = |name: &str, input: &PhysicalInput<Quantity>, dimension: &str| {
        let (value, source, flag) = known(input, name, dimension)?;
        provenance.insert(name.to_string(), source);
        synthetic |= flag;
        Ok::<f64, crate::Error>(value)
    };
    let heat_source_power_w = scalar("heat_source_power", &inputs.heat_source_power, "power")?;
    let ambient_k = scalar(
        "ambient_temperature",
        &inputs.ambient_temperature,
        "temperature",
    )?;
    let film_w_m2_k = scalar(
        "film_coefficient",
        &inputs.film_coefficient,
        "heat_transfer_coefficient",
    )?;
    let internal_film_w_m2_k = scalar(
        "internal_film_coefficient",
        &inputs.internal_film_coefficient,
        "heat_transfer_coefficient",
    )?;
    let solar = match &inputs.solar {
        Some(solar) => {
            let glazing = crate::conditions::Conditions::builtin()?.transparent_materials;
            let transparent_regions = match solar.transparent_solar {
                None => vec![],
                Some(_) => inputs
                    .exposed_regions
                    .iter()
                    .filter(|r| {
                        materials.get(r.as_str()).is_some_and(|m| {
                            m.source
                                .strip_prefix("library:")
                                .is_some_and(|m| glazing.iter().any(|g| g == m))
                        })
                    })
                    .cloned()
                    .collect(),
            };
            Some(ResolvedSolar {
                irradiance_w_m2: scalar("solar_irradiance", &solar.irradiance, "irradiance")?,
                direction: solar.direction,
                transparent_solar: solar.transparent_solar,
                transparent_regions,
            })
        }
        None => None,
    };
    let model = ThermalModel {
        synthetic,
        materials,
        heat_source_region: inputs.heat_source_region.clone(),
        heat_source_power_w,
        ambient_k,
        film_w_m2_k,
        exposed_regions: inputs.exposed_regions.clone(),
        internal_film_w_m2_k,
        solar,
        radiation: inputs.radiation,
        energy_balance_tolerance: inputs.energy_balance_tolerance,
        provenance,
    };
    model.validate(geometry)?;
    Ok(model)
}

impl ExecutionPlan {
    /// Version-7 (mesh) or version-8 (thermal) general-solid plan.
    pub fn cad_solids(recipe: CadSolidsRecipe, policy: String) -> Result<Self> {
        let thermal = recipe.thermal.is_some();
        let stage = if thermal { "fem-imported" } else { "mesh" };
        let mut plan = Self {
            schema_version: if thermal { 8 } else { 7 },
            case: None,
            fem: None,
            thermal: None,
            cad_source: None,
            imported_fem: None,
            wetting: None,
            contact: None,
            thermal_contact: None,
            freezing: None,
            spectral: None,
            atmosphere: None,
            atmospheric_transport: None,
            cad_variant: None,
            cad_sheet_variant: None,
            cad_transport: None,
            retained_cooling: None,
            cad_solids: Some(recipe),
            source: None,
            frames: None,
            filter: None,
            stages: vec![
                Stage {
                    id: stage.into(),
                    dependencies: vec![],
                    operation: if thermal {
                        StageOperation::FemImported
                    } else {
                        StageOperation::CadMesh
                    },
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 1,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec![stage.into()],
                    operation: StageOperation::Bundle,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 16 * 1024 * 1024,
                    vram_bytes: 0,
                },
            ],
            transfers: vec![],
            observation: ObservationPlan {
                metrics: if thermal {
                    vec![]
                } else {
                    vec!["geometry_correspondence".into()]
                },
                probes: vec![],
                retained_times_s: vec![],
                checkpoint_times_s: vec![],
                preview_times_s: vec![],
                max_artifact_bytes: 0,
                scientific_congestion: "fail".into(),
                preview_may_drop: false,
            },
            fleetix_revision: FLEETIX_REV.into(),
            fleetix_contract_digest: fleetix_digest(),
            policy,
        };
        crate::estimates::minimum(&plan)?.apply(&mut plan);
        plan.validate()?;
        Ok(plan)
    }
}

pub fn plan_mesh(
    store: &Store,
    request: SolidMeshRequest,
    policy: String,
) -> Result<ExecutionPlan> {
    let (_, source) = solids(store, &request.source_job, &request.regions, &request.mesh)?;
    ExecutionPlan::cad_solids(
        CadSolidsRecipe {
            source,
            thermal: None,
        },
        policy,
    )
}

pub fn plan_thermal(
    store: &Store,
    request: EnclosureThermalRequest,
    policy: String,
) -> Result<ExecutionPlan> {
    let job = store.job(&request.mesh_job)?;
    let mesh_plan = store.recorded_plan(&request.mesh_job)?;
    let Some(CadSolidsRecipe {
        source: meshed,
        thermal: None,
    }) = &mesh_plan.cad_solids
    else {
        return Err(invalid("a general-solid mesh job is required"));
    };
    if job.state != "succeeded" || job.exit_code != Some(0) {
        return Err(invalid("the approved mesh job has not succeeded"));
    }
    let (_, receipt) = crate::qualification::registered_json(
        store,
        &request.mesh_job,
        "stages/mesh/cad-mesh-receipt.json",
    )?
    .ok_or_else(|| invalid("registered mesh receipt required"))?;
    let mesh = store
        .artifact_record(&request.mesh_job, "stages/mesh/mesh.json")?
        .ok_or_else(|| invalid("registered mesh required"))?;
    if receipt["mesh_sha256"] != mesh.sha256 {
        return Err(invalid("registered mesh differs from its receipt"));
    }
    let names: Vec<String> = meshed
        .geometry
        .names()
        .into_iter()
        .map(String::from)
        .collect();
    let (_, source) = solids(store, &meshed.job_id, &names, &meshed.geometry.mesh)?;
    if digest(&source)? != digest(meshed)? {
        return Err(invalid("the mesh job's CAD source changed since meshing"));
    }
    let thermal = resolve(&request.spec, &source.geometry)?;
    ExecutionPlan::cad_solids(
        CadSolidsRecipe {
            source,
            thermal: Some(EnclosureThermalSpec {
                schema_version: 1,
                mesh_job: request.mesh_job,
                mesh_plan_digest: mesh_plan.id()?,
                mesh_sha256: mesh.sha256,
                thermal,
            }),
        },
        policy,
    )
}

/// Exactly the `cad_solids` field on a v7 (mesh) or v8 (thermal) plan.
pub(crate) fn envelope(plan: &ExecutionPlan) -> bool {
    plan.cad_solids
        .as_ref()
        .is_some_and(|recipe| plan.schema_version == if recipe.thermal.is_some() { 8 } else { 7 })
        && plan.case.is_none()
        && plan.source.is_none()
        && plan.frames.is_none()
        && plan.filter.is_none()
        && plan.fem.is_none()
        && plan.thermal.is_none()
        && plan.cad_source.is_none()
        && plan.imported_fem.is_none()
        && plan.wetting.is_none()
        && plan.contact.is_none()
        && plan.thermal_contact.is_none()
        && plan.freezing.is_none()
        && plan.spectral.is_none()
        && plan.atmosphere.is_none()
        && plan.atmospheric_transport.is_none()
        && plan.cad_variant.is_none()
        && plan.cad_sheet_variant.is_none()
        && plan.cad_transport.is_none()
        && plan.retained_cooling.is_none()
}

pub(crate) fn validate_plan(plan: &ExecutionPlan) -> Result<()> {
    let recipe = plan
        .cad_solids
        .as_ref()
        .ok_or_else(|| invalid("general-solid recipe required"))?;
    recipe.source.validate()?;
    let thermal = recipe.thermal.is_some();
    if let Some(spec) = &recipe.thermal {
        spec.validate(&recipe.source)?;
    }
    let stage = if thermal { "fem-imported" } else { "mesh" };
    if !envelope(plan)
        || plan.policy == "ci"
        || plan.stages.len() != 2
        || plan.stages[0].id != stage
        || !plan.stages[0].dependencies.is_empty()
        || plan.stages[0].operation
            != if thermal {
                StageOperation::FemImported
            } else {
                StageOperation::CadMesh
            }
        || plan.stages[1].id != "bundle"
        || plan.stages[1].dependencies != [stage]
        || plan.stages[1].operation != StageOperation::Bundle
        || plan
            .stages
            .iter()
            .any(|s| s.gpu != GpuRequirement::CpuOnly || s.selection.is_some() || s.vram_bytes != 0)
        || !plan.transfers.is_empty()
        || !plan.observation.retained_times_s.is_empty()
        || !plan.observation.checkpoint_times_s.is_empty()
        || !plan.observation.preview_times_s.is_empty()
        || !plan.observation.probes.is_empty()
        || plan.observation.preview_may_drop
        || if thermal {
            !plan.observation.metrics.is_empty()
        } else {
            plan.observation.metrics != ["geometry_correspondence"]
        }
    {
        return Err(invalid(
            "exact independent source-bound general-solid CPU mesh/thermal and bundle DAG required",
        ));
    }
    Ok(())
}

fn recipe(plan: &ExecutionPlan) -> Result<&CadSolidsRecipe> {
    plan.cad_solids
        .as_ref()
        .ok_or_else(|| invalid("approved general-solid recipe required"))
}

pub fn staged_bytes(source: &CadSolids) -> Result<u64> {
    source
        .breps
        .iter()
        .map(|b| b.bytes)
        .chain([source.manifest.bytes, source.region_evidence.bytes])
        .try_fold(0u64, |sum, b| sum.checked_add(b))
        .ok_or_else(|| invalid("general-solid staging budget overflow"))
}

#[derive(Serialize)]
pub struct ThermalDescriptor<'a> {
    schema_version: u32,
    geometry: &'a SolidsGeometry,
    mesh_sha256: &'a str,
    thermal: &'a ThermalModel,
}

pub fn mesh_descriptor(plan: &ExecutionPlan) -> Result<&SolidsGeometry> {
    let recipe = recipe(plan)?;
    recipe.source.validate()?;
    Ok(&recipe.source.geometry)
}

pub fn thermal_descriptor(plan: &ExecutionPlan) -> Result<ThermalDescriptor<'_>> {
    let recipe = recipe(plan)?;
    let spec = recipe
        .thermal
        .as_ref()
        .ok_or_else(|| invalid("approved enclosure thermal recipe required"))?;
    spec.validate(&recipe.source)?;
    Ok(ThermalDescriptor {
        schema_version: 2,
        geometry: &recipe.source.geometry,
        mesh_sha256: &spec.mesh_sha256,
        thermal: &spec.thermal,
    })
}

fn retained_paths(source: &CadSolids) -> Vec<&str> {
    source
        .breps
        .iter()
        .map(|b| b.path.as_str())
        .chain(["brep-manifest.json", "regions.json"])
        .collect()
}

fn verify_copies(root: &Path, source: &CadSolids) -> Result<()> {
    source.validate()?;
    let expected = source
        .breps
        .iter()
        .map(|b| (b.path.as_str(), &b.sha256, b.bytes))
        .chain([
            (
                "brep-manifest.json",
                &source.manifest.sha256,
                source.manifest.bytes,
            ),
            (
                "regions.json",
                &source.region_evidence.sha256,
                source.region_evidence.bytes,
            ),
        ]);
    for (path, sha, bytes) in expected {
        let observed = native_manifest(root, path, 64 * 1024 * 1024, "recheck retained solids")?;
        if &observed.sha256 != sha || observed.bytes != bytes {
            return Err(invalid(
                "retained CAD solids differ from the approved source",
            ));
        }
    }
    let names = fs::read_dir(root)?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<std::io::Result<BTreeSet<_>>>()?;
    if names
        != retained_paths(source)
            .into_iter()
            .map(std::ffi::OsString::from)
            .collect()
    {
        return Err(invalid("only exact retained CAD solid evidence permitted"));
    }
    Ok(())
}

/// Distinct-inode copies of every approved BREP before acknowledgment.
pub(crate) fn retain(store: &Store, id: &str, plan: &ExecutionPlan) -> Result<()> {
    let Some(recipe) = &plan.cad_solids else {
        return Ok(());
    };
    let expected = &recipe.source;
    let names: Vec<String> = expected
        .geometry
        .names()
        .into_iter()
        .map(String::from)
        .collect();
    let (root, observed) = solids(store, &expected.job_id, &names, &expected.geometry.mesh)?;
    if digest(&observed)? != digest(expected)? {
        return Err(invalid("approved CAD solids changed"));
    }
    let job_dir = store.job_dir(id)?;
    let destination = job_dir.join(RETAINED);
    if destination.exists() {
        return Err(invalid("immutable CAD solid destination already exists"));
    }
    let intent = commit_artifact(
        &job_dir,
        INTENT,
        &serde_json::to_vec(expected)?,
        "json",
        "durable general-solid staging intent; recovery verifies the original before deleting copies",
    )?;
    let staging = job_dir.join(format!(".retained-solids-{}", uuid::Uuid::new_v4()));
    private_dir(&staging)?;
    for path in retained_paths(expected) {
        let record = store
            .artifact_record(&expected.job_id, path)?
            .ok_or_else(|| invalid("original CAD solid record missing"))?;
        copy_verified(&root, &staging, &record)?;
    }
    verify_copies(&staging, expected)?;
    sync_directories(&staging)?;
    publish_directory(&staging, &destination)?;
    fs::File::open(&job_dir)?.sync_all()?;
    for path in retained_paths(expected) {
        let record = native_manifest(
            &job_dir,
            &format!("{RETAINED}/{path}"),
            64 * 1024 * 1024,
            "verified distinct-inode immutable CAD solid; original mm placement retained",
        )?;
        store.add_artifact(id, &record)?;
    }
    let provenance = serde_json::json!({"source":expected,"plan":store.recorded_plan(&expected.job_id)?,
        "host_profile":store.job_profile(&expected.job_id)?,"execution_binding":store.execution_binding(&expected.job_id)?,
        "execution_authorization":store.execution_authorization(&expected.job_id)?});
    store.add_artifact(
        id,
        &commit_artifact(
            &job_dir,
            "cad-solids-execution.json",
            &serde_json::to_vec_pretty(&provenance)?,
            "json",
            "original authorized CAD solids execution; no upgraded qualification",
        )?,
    )?;
    store.add_artifact(id, &intent)?;
    Ok(())
}

/// The verified retained-solid directory of a submitted job.
pub fn registered(store: &Store, id: &str, plan: &ExecutionPlan) -> Result<PathBuf> {
    let expected = &recipe(plan)?.source;
    let root = store.job_dir(id)?;
    let source: CadSolids =
        serde_json::from_slice(&read_bounded(&safe_path(&root, INTENT)?, MAX_MESSAGE)?)?;
    if digest(&source)? != digest(expected)? {
        return Err(invalid(
            "retained CAD solid intent differs from approved plan",
        ));
    }
    let copies = safe_path(&root, RETAINED)?;
    verify_copies(&copies, expected)?;
    for path in retained_paths(expected) {
        let record = store
            .artifact_record(id, &format!("{RETAINED}/{path}"))?
            .ok_or_else(|| invalid("retained CAD solid registration missing"))?;
        let observed = native_manifest(&root, &record.path, 64 * 1024 * 1024, "recheck")?;
        if record.sha256 != observed.sha256 || record.bytes != observed.bytes {
            return Err(invalid("retained CAD solid registry changed"));
        }
    }
    Ok(copies)
}

pub(crate) fn recover_orphan(store: &Store, id: &str) -> Result<()> {
    let root = safe_path(&store.root, &format!("artifacts/{id}"))?;
    let intent = safe_path(&root, INTENT)?;
    if !intent.exists() {
        return Ok(());
    }
    let expected: CadSolids = serde_json::from_slice(&read_bounded(&intent, MAX_MESSAGE)?)?;
    let names: Vec<String> = expected
        .geometry
        .names()
        .into_iter()
        .map(String::from)
        .collect();
    if let Ok((_, original)) = solids(store, &expected.job_id, &names, &expected.geometry.mesh)
        && digest(&original)? == digest(&expected)?
    {
        fs::remove_dir_all(&root)?;
        fs::File::open(root.parent().ok_or_else(|| invalid("CAD orphan parent"))?)?.sync_all()?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeshFile {
    schema_version: u32,
    synthetic: bool,
    coordinate_unit: String,
    element_type: String,
    nodes: BTreeMap<u64, [f64; 3]>,
    elements: BTreeMap<u64, Vec<u64>>,
    regions: BTreeMap<String, Vec<u64>>,
    boundary_faces: Vec<(u64, u8, String)>,
    classification: String,
}

/// Independently recomputed mesh facts.
#[derive(Debug)]
pub struct MeshFacts {
    pub nodes: usize,
    pub elements: usize,
    pub element_type: String,
    pub region_volumes_m3: BTreeMap<String, f64>,
    pub max_relative_volume_error: f64,
    pub min_shape_ratio: f64,
    pub interfaces_m2: BTreeMap<String, f64>,
    pub ambient_m2: BTreeMap<String, f64>,
    pub cavity_m2: BTreeMap<String, f64>,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl MeshFile {
    fn point(&self, id: u64) -> [f64; 3] {
        self.nodes[&id]
    }
    fn volume(&self, element: &[u64]) -> f64 {
        let [a, b, c, d] = [0, 1, 2, 3].map(|i| self.point(element[i]));
        dot(sub(b, a), cross(sub(c, a), sub(d, a))) / 6.
    }
    /// Area and unit outward normal of 0-based face `face`.
    fn face(&self, element: &[u64], face: usize) -> ([f64; 3], f64, [f64; 3]) {
        let [a, b, c] = FACES[face].map(|i| self.point(element[i]));
        let opposite = self.point(element[6 - FACES[face].iter().sum::<usize>()]);
        let n = cross(sub(b, a), sub(c, a));
        let length = dot(n, n).sqrt();
        let sign = if dot(n, sub(opposite, a)) > 0. {
            -1.
        } else {
            1.
        };
        (a, length / 2., n.map(|v| sign * v / length))
    }
    /// Temperatures at the face's three edge midpoints.
    fn face_values(&self, element: &[u64], face: usize, t: &BTreeMap<u64, f64>) -> [f64; 3] {
        if element.len() == 10 {
            FACE_MIDS[face].map(|m| t[&element[m]])
        } else {
            let [a, b, c] = FACES[face].map(|i| t[&element[i]]);
            [(a + b) / 2., (b + c) / 2., (c + a) / 2.]
        }
    }
}

fn parse_mesh(geometry: &SolidsGeometry, bytes: &[u8]) -> Result<(MeshFile, MeshFacts)> {
    geometry.validate()?;
    let mesh: MeshFile = serde_json::from_slice(bytes)?;
    let (element_type, width) = match geometry.mesh.element_order {
        1 => ("C3D4", 4),
        _ => ("C3D10", 10),
    };
    let names: BTreeSet<&str> = geometry.names().into_iter().collect();
    if mesh.schema_version != 1
        || mesh.synthetic != geometry.synthetic
        || mesh.coordinate_unit != "m"
        || mesh.element_type != element_type
        || mesh.nodes.is_empty()
        || mesh.elements.is_empty()
        || mesh.elements.len() as u64 > geometry.mesh.max_elements
        || mesh.nodes.contains_key(&0)
        || mesh.elements.contains_key(&0)
        || mesh
            .regions
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != names
        || mesh.classification.is_empty()
        || mesh.nodes.values().flatten().any(|v| !v.is_finite())
    {
        return Err(invalid(
            "complete bounded general-solid tetrahedral mesh evidence required",
        ));
    }
    let mut region_of = BTreeMap::new();
    for (name, ids) in &mesh.regions {
        for id in ids {
            if region_of.insert(*id, name.clone()).is_some() {
                return Err(invalid("each element belongs to exactly one region"));
            }
        }
    }
    if region_of.len() != mesh.elements.len() || region_of.keys().ne(mesh.elements.keys()) {
        return Err(invalid("regions must partition the elements"));
    }
    let mut volumes: BTreeMap<String, f64> = names.iter().map(|n| (n.to_string(), 0.)).collect();
    let mut min_ratio = f64::INFINITY;
    let mut owners: BTreeMap<[u64; 3], Vec<(u64, u8)>> = BTreeMap::new();
    for (id, element) in &mesh.elements {
        if element.len() != width
            || element.iter().collect::<BTreeSet<_>>().len() != width
            || element.iter().any(|n| !mesh.nodes.contains_key(n))
        {
            return Err(invalid("unique registered nodes per tetrahedron required"));
        }
        let volume = mesh.volume(element);
        if !(volume.is_finite() && volume > 0.) {
            return Err(invalid("inverted or degenerate tetrahedron"));
        }
        let points: Vec<[f64; 3]> = element.iter().map(|n| mesh.point(*n)).collect();
        let rms = (EDGES
            .iter()
            .map(|[a, b]| {
                let d = sub(points[*a], points[*b]);
                dot(d, d)
            })
            .sum::<f64>()
            / 6.)
            .sqrt();
        min_ratio = min_ratio.min(6. * 2f64.sqrt() * volume / rms.powi(3));
        if width == 10 {
            for ([a, b], mid) in EDGES.iter().zip(&points[4..]) {
                let centre = [0, 1, 2].map(|k| (points[*a][k] + points[*b][k]) / 2.);
                let d = sub(centre, *mid);
                let e = sub(points[*a], points[*b]);
                if dot(d, d) > 0.0625 * dot(e, e) {
                    return Err(invalid("C3D10 midside node ordering changed"));
                }
            }
        }
        *volumes
            .get_mut(&region_of[id])
            .ok_or_else(|| invalid("region"))? += volume;
        for (index, corners) in FACES.iter().enumerate() {
            let mut key = corners.map(|i| element[i]);
            key.sort_unstable();
            owners.entry(key).or_default().push((*id, index as u8 + 1));
        }
    }
    if min_ratio < geometry.mesh.min_quality {
        return Err(invalid("mesh quality below the approved gate"));
    }
    let mut boundary = BTreeSet::new();
    let mut interfaces = BTreeMap::new();
    for owner in owners.values() {
        match owner.as_slice() {
            [single] => {
                boundary.insert(*single);
            }
            [(a, fa), (b, _)] if region_of[a] != region_of[b] => {
                let mut pair = [region_of[a].as_str(), region_of[b].as_str()];
                pair.sort_unstable();
                let (_, area, _) = mesh.face(&mesh.elements[a], usize::from(*fa) - 1);
                *interfaces.entry(pair.join("|")).or_insert(0.) += area;
            }
            [_, _] => {}
            _ => return Err(invalid("non-manifold mesh face")),
        }
    }
    let listed: BTreeSet<(u64, u8)> = mesh
        .boundary_faces
        .iter()
        .map(|(e, f, _)| (*e, *f))
        .collect();
    let mut ambient = BTreeMap::new();
    let mut cavity = BTreeMap::new();
    for (eid, face, label) in &mesh.boundary_faces {
        let (_, area, _) = mesh.face(&mesh.elements[eid], usize::from(*face) - 1);
        let target = match label.as_str() {
            "ambient" => &mut ambient,
            "cavity" => &mut cavity,
            _ => return Err(invalid("boundary faces are ambient or cavity")),
        };
        *target.entry(region_of[eid].clone()).or_insert(0.) += area;
    }
    if listed != boundary || listed.len() != mesh.boundary_faces.len() || ambient.is_empty() {
        return Err(invalid(
            "boundary faces must be exactly the unshared element faces, with an ambient exterior",
        ));
    }
    let mut max_error: f64 = 0.;
    for region in &geometry.regions {
        let error = (volumes[&region.name] - region.volume_m3).abs() / region.volume_m3;
        if error.is_nan() || error > geometry.mesh.volume_relative_tolerance {
            return Err(invalid(format!(
                "region {} mesh volume differs from the approved CAD volume",
                region.name
            )));
        }
        max_error = max_error.max(error);
    }
    let facts = MeshFacts {
        nodes: mesh.nodes.len(),
        elements: mesh.elements.len(),
        element_type: element_type.into(),
        region_volumes_m3: volumes,
        max_relative_volume_error: max_error,
        min_shape_ratio: min_ratio,
        interfaces_m2: interfaces,
        ambient_m2: ambient,
        cavity_m2: cavity,
    };
    Ok((mesh, facts))
}

/// Independent mesh checks: element budget, orientation, quality, per-region
/// volume against CAD, exact exterior faces and conformal interfaces.
pub fn verify_mesh(geometry: &SolidsGeometry, bytes: &[u8]) -> Result<MeshFacts> {
    parse_mesh(geometry, bytes).map(|(_, facts)| facts)
}

fn close(value: &Value, expected: f64, relative: f64) -> bool {
    value
        .as_f64()
        .is_some_and(|v| (v - expected).abs() <= relative * expected.abs().max(1e-300))
}

fn common_receipt(receipt: &Value, adapter: &str, policy: &str) -> bool {
    let checks = &receipt["sandbox"]["checks"];
    receipt["schema_version"] == 1
        && receipt["adapter"] == adapter
        && receipt["backend"] == "cpu"
        && receipt["executed"] == true
        && receipt["software_fallback"] == false
        && receipt["precision"] == "float64"
        && receipt["gmsh_version"] == "4.15.2"
        && receipt["gmsh_source_sha256"] == GMSH_SOURCE
        && receipt["gap_healing"] == false
        && receipt["physical_validation"] == "unqualified"
        && receipt["sandbox"]["policy"] == policy
        && checks
            .as_object()
            .is_some_and(|c| c.len() == SANDBOX_CHECKS.len())
        && SANDBOX_CHECKS.iter().all(|k| checks[k] == true)
}

fn mesh_receipt_matches(geometry: &SolidsGeometry, facts: &MeshFacts, receipt: &Value) -> bool {
    receipt["element_type"] == facts.element_type
        && receipt["nodes"] == facts.nodes as u64
        && receipt["elements"] == facts.elements as u64
        && geometry.regions.iter().all(|r| {
            let entry = &receipt["regions"][&r.name];
            entry["brep_sha256"] == r.brep_sha256.as_str()
                && entry["cad_volume_m3"] == r.volume_m3
                && close(
                    &entry["mesh_volume_m3"],
                    facts.region_volumes_m3[&r.name],
                    1e-9,
                )
        })
        && receipt["regions"].as_object().map(|m| m.len()) == Some(geometry.regions.len())
        && close(
            &receipt["quality"]["min_shape_ratio"],
            facts.min_shape_ratio,
            1e-9,
        )
        && receipt["quality"]["gmsh_min_sicn"]
            .as_f64()
            .is_some_and(|v| v > 0. && v <= 1.)
        && receipt["interfaces_m2"].as_object().map(|m| m.len()) == Some(facts.interfaces_m2.len())
        && facts
            .interfaces_m2
            .iter()
            .all(|(k, v)| close(&receipt["interfaces_m2"][k], *v, 1e-9))
}

pub fn verify_mesh_receipt(
    plan: &ExecutionPlan,
    root: &Path,
    receipt: &Value,
) -> Result<NumericalEvidence> {
    let geometry = mesh_descriptor(plan)?;
    let record = native_manifest(
        root,
        "mesh.json",
        MAX_MESH_BYTES,
        "verify closed native mesh",
    )?;
    let bytes = read_bounded(&safe_path(root, "mesh.json")?, MAX_MESH_BYTES)?;
    let facts = verify_mesh(geometry, &bytes)?;
    if !common_receipt(receipt, "Gmsh", crate::execution::CAD_MESH_SANDBOX_POLICY)
        || receipt["formulation"] != MESH_FORMULATION
        || receipt["request_sha256"] != digest(geometry)?
        || receipt["mesh_sha256"] != record.sha256
        || receipt["synthetic"] != geometry.synthetic
        || receipt["geometry_provenance"] != geometry.geometry_provenance
        || receipt["source_unit"] != "mm"
        || receipt["coordinate_unit"] != "m"
        || receipt["scale_to_m"] != 0.001
        || receipt["placement_translation_unit"] != "mm"
        || receipt["fragmented"] != true
        || !mesh_receipt_matches(geometry, &facts, receipt)
    {
        return Err(invalid(
            "general-solid mesh receipt differs from approved source, independent mesh facts or isolated policy",
        ));
    }
    Ok(NumericalEvidence {
        reference: "approved CAD solid volumes; independently recomputed orientation, quality and exterior faces".into(),
        scope: "geometric correspondence only; no solver, field or physical validation".into(),
        error_kind: "max_relative_region_volume_error".into(),
        error: facts.max_relative_volume_error,
        tolerance: geometry.mesh.volume_relative_tolerance,
    })
}

/// Final nodal temperatures of the last `temperatures for set NALL` block.
pub fn read_temperatures(text: &str) -> Result<BTreeMap<u64, f64>> {
    let mut block: Option<(f64, BTreeMap<u64, f64>)> = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("temperatures for set NALL and time") {
            let time: f64 = rest
                .trim()
                .replace(['D', 'd'], "E")
                .parse()
                .map_err(|_| invalid("time"))?;
            block = Some((time, BTreeMap::new()));
        } else if line.contains(" for set ") {
            return Err(invalid("unexpected native field section"));
        } else if let Some((_, values)) = &mut block {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let [id, value] = parts.as_slice() else {
                continue;
            };
            let (Ok(id), Ok(value)) = (
                id.parse::<u64>(),
                value.replace(['D', 'd'], "E").parse::<f64>(),
            ) else {
                continue;
            };
            if values.insert(id, value).is_some() {
                return Err(invalid("duplicate native temperature"));
            }
        }
    }
    match block {
        Some((time, values)) if (time - 1.).abs() <= 1e-12 && !values.is_empty() => Ok(values),
        _ => Err(invalid("one final steady temperature block required")),
    }
}

/// Recomputed energy terms and region statistics of a native field.
#[derive(Debug)]
pub struct ThermalFacts {
    pub heat_source_w: f64,
    pub solar_absorbed_w: f64,
    /// Transparent-glazing incident solar deposited in the internal air node.
    pub solar_cavity_w: f64,
    pub film_w: f64,
    pub radiation_w: f64,
    pub internal_air_residual_w: f64,
    pub relative_error: f64,
    pub regions: BTreeMap<String, [f64; 3]>,
    pub max_k: f64,
}

fn thermal_facts(
    mesh: &MeshFile,
    model: &ThermalModel,
    temperatures: &BTreeMap<u64, f64>,
    internal_air_k: f64,
) -> Result<ThermalFacts> {
    if temperatures.keys().ne(mesh.nodes.keys())
        || temperatures.values().any(|t| !(t.is_finite() && *t > 0.))
    {
        return Err(invalid(
            "complete finite absolute native temperatures required",
        ));
    }
    let region_of: BTreeMap<u64, &str> = mesh
        .regions
        .iter()
        .flat_map(|(n, ids)| ids.iter().map(move |id| (*id, n.as_str())))
        .collect();
    let exposed: BTreeSet<&str> = model.exposed_regions.iter().map(String::as_str).collect();
    let ta = model.ambient_k;
    let (mut film, mut radiation, mut solar, mut air, mut cavity_solar) = (0., 0., 0., 0., 0.);
    for (eid, face, label) in &mesh.boundary_faces {
        let element = &mesh.elements[eid];
        let index = usize::from(*face) - 1;
        let (_, area, normal) = mesh.face(element, index);
        let values = mesh.face_values(element, index, temperatures);
        let region = region_of[eid];
        if label == "cavity" {
            air += model.internal_film_w_m2_k * area / 3.
                * values.iter().map(|t| t - internal_air_k).sum::<f64>();
        } else if exposed.contains(region) {
            let material = &model.materials[region];
            film += model.film_w_m2_k * area / 3. * values.iter().map(|t| t - ta).sum::<f64>();
            if model.radiation {
                radiation += material.emissivity.unwrap_or(f64::NAN) * SIGMA * area / 3.
                    * values.iter().map(|t| t.powi(4) - ta.powi(4)).sum::<f64>();
            }
            if let Some(sun) = &model.solar {
                let cosine = dot(normal, sun.direction);
                if cosine > 0. {
                    let incident = sun.irradiance_w_m2 * cosine * area;
                    if sun.transparent_regions.iter().any(|r| r == region) {
                        cavity_solar += incident;
                    } else {
                        solar += material.solar_absorptance.unwrap_or(f64::NAN) * incident;
                    }
                }
            }
        }
    }
    let mut regions = BTreeMap::new();
    for (name, ids) in &mesh.regions {
        let (mut weighted, mut volume) = (0., 0.);
        let (mut high, mut low) = (f64::NEG_INFINITY, f64::INFINITY);
        for id in ids {
            let element = &mesh.elements[id];
            let v = mesh.volume(element);
            weighted += v * element[..4].iter().map(|n| temperatures[n]).sum::<f64>() / 4.;
            volume += v;
            for n in element {
                high = high.max(temperatures[n]);
                low = low.min(temperatures[n]);
            }
        }
        regions.insert(name.clone(), [high, low, weighted / volume]);
    }
    let heat_source_w = model.heat_source_power_w;
    let heat_in = heat_source_w + solar + cavity_solar;
    Ok(ThermalFacts {
        heat_source_w,
        solar_absorbed_w: solar,
        solar_cavity_w: cavity_solar,
        film_w: film,
        radiation_w: radiation,
        // Net heat into the air node: cavity faces plus the deposited solar.
        internal_air_residual_w: air + cavity_solar,
        relative_error: (heat_in - film - radiation).abs() / heat_in,
        max_k: temperatures
            .values()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max),
        regions,
    })
}

/// Verify a native enclosure thermal stage: the approved mesh, the native
/// field, recomputed region statistics and the steady energy-balance gate.
pub fn verify_thermal_receipt(
    plan: &ExecutionPlan,
    root: &Path,
    receipt: &Value,
) -> Result<NumericalEvidence> {
    let descriptor = thermal_descriptor(plan)?;
    let (geometry, model) = (descriptor.geometry, descriptor.thermal);
    let mesh_record = native_manifest(root, "mesh.json", MAX_MESH_BYTES, "verify enclosure mesh")?;
    let field = native_manifest(
        root,
        "reference.dat",
        MAX_FIELD_BYTES,
        "verify enclosure field",
    )?;
    if mesh_record.sha256 != descriptor.mesh_sha256 {
        return Err(invalid(
            "re-meshed solids differ from the approved mesh job",
        ));
    }
    let (mesh, facts) = parse_mesh(
        geometry,
        &read_bounded(&safe_path(root, "mesh.json")?, MAX_MESH_BYTES)?,
    )?;
    let text = read_bounded(&safe_path(root, "reference.dat")?, MAX_FIELD_BYTES)?;
    let temperatures = read_temperatures(
        std::str::from_utf8(&text).map_err(|_| invalid("UTF-8 native field required"))?,
    )?;
    let has_cavity = mesh.boundary_faces.iter().any(|(_, _, l)| l == "cavity");
    let internal_air_k = match (&receipt["internal_air_k"], has_cavity) {
        (Value::Null, false) => model.ambient_k,
        (Value::Number(value), true) => value.as_f64().unwrap_or(f64::NAN),
        _ => {
            return Err(invalid(
                "internal air temperature must match the cavity model",
            ));
        }
    };
    let thermal = thermal_facts(&mesh, model, &temperatures, internal_air_k)?;
    let energy = &receipt["energy_balance"];
    let tolerance = model.energy_balance_tolerance;
    let heat_in = thermal.heat_source_w + thermal.solar_absorbed_w + thermal.solar_cavity_w;
    let transparent = model
        .solar
        .as_ref()
        .is_some_and(|s| s.transparent_solar.is_some());
    if (transparent && !has_cavity && thermal.solar_cavity_w > 0.)
        || !close(&energy["solar_cavity_w"], thermal.solar_cavity_w, 1e-9)
        || (receipt["applicability"]["transparent_solar"] == TRANSPARENT_SOLAR_STATEMENT)
            != transparent
    {
        return Err(invalid(
            "transparent glazing solar must be the declared conservative cavity deposit, labelled in the receipt applicability",
        ));
    }
    if !common_receipt(
        receipt,
        "CalculiX",
        crate::execution::FEM_IMPORTED_SANDBOX_POLICY,
    ) || receipt["factorization"] != "SPOOLES"
        || receipt["synthetic"] != (model.synthetic || geometry.synthetic)
        || receipt["formulation"] != THERMAL_FORMULATION
        || receipt["calculix_version"] != "2.23"
        || receipt["calculix_source_sha256"] != CALCULIX_SOURCE
        || receipt["request_sha256"] != digest(&descriptor)?
        || receipt["mesh_sha256"] != mesh_record.sha256
        || receipt["native_field_sha256"] != field.sha256
        || receipt["element_type"] != facts.element_type
        || receipt["nodes"] != facts.nodes as u64
        || receipt["elements"] != facts.elements as u64
        || receipt["temperature_unit"] != "K"
        || !receipt["applicability"].is_object()
        || receipt["regions"].as_object().map(|m| m.len()) != Some(thermal.regions.len())
        || thermal.regions.iter().any(|(name, [high, low, mean])| {
            let entry = &receipt["regions"][name];
            !close(&entry["max_k"], *high, 1e-12)
                || !close(&entry["min_k"], *low, 1e-12)
                || !close(&entry["mean_k"], *mean, 1e-9)
        })
        || !close(
            &receipt["global_max"]["temperature_k"],
            thermal.max_k,
            1e-12,
        )
        || !close(&energy["heat_in_w"], heat_in, 1e-9)
        || !close(&energy["film_w"], thermal.film_w, 1e-6)
        || (model.radiation && !close(&energy["radiation_w"], thermal.radiation_w, 1e-6))
        || energy["tolerance"] != tolerance
        || thermal.relative_error.is_nan()
        || thermal.relative_error > tolerance
        || (thermal.internal_air_residual_w.abs() / heat_in).is_nan()
        || thermal.internal_air_residual_w.abs() / heat_in > tolerance
        || !energy["relative_error"]
            .as_f64()
            .is_some_and(|e| e <= tolerance)
    {
        return Err(invalid(
            "enclosure thermal receipt differs from the approved model, native field, recomputed statistics or energy-balance gate",
        ));
    }
    Ok(NumericalEvidence {
        reference: "steady global energy balance: heat source + absorbed solar (+ transparent-glazing solar deposited in the internal air) against film + radiation, recomputed from the native field".into(),
        scope: format!(
            "steady solid conduction with declared boundary inputs and one effective internal air node; no internal convection, condensation or physical validation{}",
            if transparent {
                format!("; {TRANSPARENT_SOLAR_STATEMENT}")
            } else {
                String::new()
            }
        ),
        error_kind: "relative_energy_balance_error".into(),
        error: thermal.relative_error,
        tolerance,
    })
}
