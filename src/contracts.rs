use crate::{Error, Result, science::Quantity};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const PROTOCOL_VERSION: u32 = 1;
pub const FLEETIX_REV: &str = "2230d9ee804a66d94424a91919182e4fcca13ab2";
pub const MAX_MESSAGE: u64 = 65536;

pub fn digest<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
pub fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
pub fn token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub source: String,
    pub sha256: Option<String>,
    pub synthetic: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub name: String,
    pub provenance: Provenance,
    pub density: Quantity,
    pub humidity_fraction: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhysicsApplicability {
    pub formulation: String,
    pub dimensionality: u8,
    pub precision: String,
    pub validated_reynolds_max: Option<f64>,
    pub exclusions: Vec<String>,
    pub numerical_tolerance: f64,
    pub physical_validation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Claim {
    Velocity,
    Cooling,
    ResolvedIngress,
    SnowDeposition,
    IceDamage,
    MaterialLifetime,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Presentation {
    pub camera: [f64; 3],
    pub width: u32,
    pub height: u32,
    pub field: String,
    pub range: [f64; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseSpec {
    pub schema_version: u32,
    pub name: String,
    pub geometry: Provenance,
    pub regions: Vec<String>,
    pub material: Material,
    pub applicability: PhysicsApplicability,
    pub length: Quantity,
    pub channel_height: Quantity,
    pub kinematic_viscosity: Quantity,
    pub acceleration: Quantity,
    pub geometry_tolerance: Quantity,
    pub resolution: u32,
    pub max_time_s: f64,
    pub claims: Vec<Claim>,
    pub presentation: Presentation,
}
impl CaseSpec {
    pub fn reference() -> Self {
        Self {
            schema_version: 1,
            name: "synthetic-channel".into(),
            geometry: Provenance {
                source: "synthetic parallel plates".into(),
                sha256: None,
                synthetic: true,
            },
            regions: vec!["fluid".into(), "wall".into()],
            material: Material {
                name: "synthetic Newtonian fluid".into(),
                provenance: Provenance {
                    source: "analytical fixture".into(),
                    sha256: None,
                    synthetic: true,
                },
                density: Quantity {
                    value: 1.,
                    unit: "kg/m3".into(),
                },
                humidity_fraction: None,
            },
            applicability: PhysicsApplicability {
                formulation: "steady_incompressible_channel".into(),
                dimensionality: 3,
                precision: "float64".into(),
                validated_reynolds_max: None,
                exclusions: vec!["thermal coupling".into(), "wetting and ingress".into()],
                numerical_tolerance: 1e-12,
                physical_validation: "unqualified".into(),
            },
            length: Quantity {
                value: 1.,
                unit: "m".into(),
            },
            channel_height: Quantity {
                value: 0.01,
                unit: "m".into(),
            },
            kinematic_viscosity: Quantity {
                value: 1e-5,
                unit: "m2/s".into(),
            },
            acceleration: Quantity {
                value: 0.1,
                unit: "m/s2".into(),
            },
            geometry_tolerance: Quantity {
                value: 1e-5,
                unit: "m".into(),
            },
            resolution: 33,
            max_time_s: 1.,
            claims: vec![Claim::Velocity],
            presentation: Presentation {
                camera: [1., 1., 1.],
                width: 640,
                height: 480,
                field: "velocity".into(),
                range: [0., 0.125],
            },
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 || !token(&self.name) {
            return Err(invalid("case version/name"));
        }
        for (q, dimension) in [
            (&self.length, "length"),
            (&self.channel_height, "length"),
            (&self.kinematic_viscosity, "kinematic_viscosity"),
            (&self.material.density, "density"),
        ] {
            if q.si(dimension)? <= 0. {
                return Err(invalid(format!("positive {dimension} required")));
            }
        }
        self.acceleration.si("acceleration")?;
        if self.geometry_tolerance.si("length")? <= 0. {
            return Err(invalid("positive explicit CAD chord tolerance required"));
        }
        if !(3..=1_000_000).contains(&self.resolution)
            || !self.max_time_s.is_finite()
            || self.max_time_s <= 0.
        {
            return Err(invalid("resolution/time outside explicit limits"));
        }
        if self.regions.is_empty()
            || self.regions.iter().any(|r| !token(r))
            || self.regions.iter().collect::<BTreeSet<_>>().len() != self.regions.len()
        {
            return Err(invalid("regions must be unique, named and unambiguous"));
        }
        if let Some(h) = self.material.humidity_fraction
            && (!h.is_finite() || !(0.0..=1.).contains(&h))
        {
            return Err(invalid("humidity fraction"));
        }
        if self.applicability.precision != "float64"
            || self.applicability.dimensionality != 3
            || !self.applicability.numerical_tolerance.is_finite()
            || self.applicability.numerical_tolerance <= 0.
        {
            return Err(invalid(
                "unsupported dimensionality, precision or tolerance",
            ));
        }
        if self.applicability.physical_validation != "unqualified" {
            return Err(invalid(
                "physical validation must come from evidence, not a case assertion",
            ));
        }
        for claim in &self.claims {
            match claim {
                Claim::Velocity => {}
                Claim::Cooling => {
                    return Err(invalid(
                        "velocity alone cannot define cooling; heat-flux or correlation inputs required",
                    ));
                }
                Claim::ResolvedIngress => {
                    return Err(invalid("no validated resolved gap/wetting model"));
                }
                Claim::SnowDeposition => return Err(invalid("prescribed snow is not deposition")),
                Claim::IceDamage => return Err(invalid("latent heat is not ice damage")),
                Claim::MaterialLifetime => {
                    return Err(invalid("dose is not calibrated material lifetime"));
                }
            }
        }
        if self.presentation.width == 0
            || self.presentation.height == 0
            || self.presentation.width > 4096
            || self.presentation.height > 4096
            || self
                .presentation
                .camera
                .iter()
                .chain(self.presentation.range.iter())
                .any(|v| !v.is_finite())
            || self.presentation.range[0] >= self.presentation.range[1]
        {
            return Err(invalid("presentation budget or fixed comparison range"));
        }
        Ok(())
    }
    pub fn science_id(&self) -> Result<String> {
        let mut value = serde_json::to_value(self)?;
        value
            .as_object_mut()
            .ok_or_else(|| invalid("case serialization"))?
            .remove("presentation");
        digest(&value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Compute,
    Render,
    Media,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GpuSelection {
    pub role: Role,
    pub backend: String,
    pub pci: String,
    pub backend_uuid: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct B1Selections {
    pub compute: GpuSelection,
    pub render: GpuSelection,
    pub media: GpuSelection,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GpuRequirement {
    Required,
    Preferred,
    CpuOnly,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StageOperation {
    ChannelReference,
    CadFixture,
    CadInspect,
    CadMesh,
    FemImported,
    Openlb,
    NumericalFilter,
    FemReference,
    ThermalReference,
    ThermalProjection,
    WettingReference,
    FreezingReference,
    SpectralReference,
    AtmosphericReference,
    AtmosphericTransport,
    ContactReference,
    Render,
    Video,
    Bundle,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    pub id: String,
    pub dependencies: Vec<String>,
    pub operation: StageOperation,
    pub gpu: GpuRequirement,
    pub selection: Option<GpuSelection>,
    pub ram_bytes: u64,
    pub vram_bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlan {
    pub metrics: Vec<String>,
    pub probes: Vec<[f64; 3]>,
    pub retained_times_s: Vec<f64>,
    pub checkpoint_times_s: Vec<f64>,
    pub preview_times_s: Vec<f64>,
    pub max_artifact_bytes: u64,
    pub scientific_congestion: String,
    pub preview_may_drop: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransferSpec {
    pub source_region: String,
    pub destination_region: String,
    pub source_quantity: String,
    pub destination_quantity: String,
    pub unit: String,
    pub orientation: [f64; 3],
    pub interpolation: String,
    pub maximum_relative_conservation_error: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RetainedSource {
    pub job_id: String,
    pub plan_digest: String,
    pub execution_binding_digest: String,
    pub authorization_digest: String,
    pub snapshot_sha256: String,
    pub artifact_id: String,
    pub science_id: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresentationRequest {
    pub source_job: String,
    pub times_s: Vec<f64>,
    pub presentation: Presentation,
    pub render: GpuSelection,
    pub media: Option<GpuSelection>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameSource {
    pub job_id: String,
    pub plan_digest: String,
    pub execution_binding_digest: String,
    pub authorization_digest: String,
    pub sequence_sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VideoRequest {
    pub source_job: String,
    pub media: GpuSelection,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GradientField {
    Velocity,
    Pressure,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilterSpec {
    pub time_s: f64,
    pub field: GradientField,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilterRequest {
    pub source_job: String,
    pub filter: FilterSpec,
    pub compute: GpuSelection,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPlan {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case: Option<CaseSpec>,
    pub stages: Vec<Stage>,
    pub transfers: Vec<TransferSpec>,
    pub observation: ObservationPlan,
    pub fleetix_revision: String,
    pub fleetix_contract_digest: String,
    pub policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<RetainedSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<FrameSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<FilterSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fem: Option<crate::fem::FemReferenceSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal: Option<crate::thermal::ThermalReferenceSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cad_source: Option<crate::cad_source::CadSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_fem: Option<crate::fem_imported::ImportedFemSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wetting: Option<crate::wetting::WettingReferenceSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<crate::contact::ContactReferenceSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_contact: Option<crate::thermal_contact::ThermalContactSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freezing: Option<crate::freezing::FreezingReferenceSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectral: Option<crate::radiation::SpectralReferenceSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atmosphere: Option<crate::atmosphere::AtmosphericReferenceSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atmospheric_transport: Option<crate::atmospheric_transport::AtmosphericTransportSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cad_variant: Option<crate::cad_variant::CadVariantSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cad_transport: Option<crate::cad_transport::CadSpectralTransportSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retained_cooling: Option<crate::cooling_execution::CoolingExecutionSpec>,
}

// Decode versions explicitly: v1's field set remains strict, and v2 requires
// an immutable source. The remote record shares the exact existing field types.
#[derive(Deserialize, JsonSchema)]
#[serde(remote = "ExecutionPlan", deny_unknown_fields)]
struct ExecutionPlanRecord {
    schema_version: u32,
    #[serde(default, deserialize_with = "channel_case")]
    case: Option<CaseSpec>,
    stages: Vec<Stage>,
    transfers: Vec<TransferSpec>,
    observation: ObservationPlan,
    fleetix_revision: String,
    fleetix_contract_digest: String,
    policy: String,
    #[serde(default, deserialize_with = "retained_source")]
    source: Option<RetainedSource>,
    #[serde(default, deserialize_with = "frame_source")]
    frames: Option<FrameSource>,
    #[serde(default, deserialize_with = "filter_spec")]
    filter: Option<FilterSpec>,
    #[serde(default, deserialize_with = "fem_spec")]
    fem: Option<crate::fem::FemReferenceSpec>,
    #[serde(default, deserialize_with = "thermal_spec")]
    thermal: Option<crate::thermal::ThermalReferenceSpec>,
    #[serde(default, deserialize_with = "cad_source")]
    cad_source: Option<crate::cad_source::CadSource>,
    #[serde(default, deserialize_with = "imported_fem")]
    imported_fem: Option<crate::fem_imported::ImportedFemSpec>,
    #[serde(default, deserialize_with = "wetting_spec")]
    wetting: Option<crate::wetting::WettingReferenceSpec>,
    #[serde(default, deserialize_with = "contact_spec")]
    contact: Option<crate::contact::ContactReferenceSpec>,
    #[serde(default, deserialize_with = "thermal_contact_spec")]
    thermal_contact: Option<crate::thermal_contact::ThermalContactSpec>,
    #[serde(default, deserialize_with = "freezing_spec")]
    freezing: Option<crate::freezing::FreezingReferenceSpec>,
    #[serde(default, deserialize_with = "spectral_spec")]
    spectral: Option<crate::radiation::SpectralReferenceSpec>,
    #[serde(default, deserialize_with = "atmospheric_spec")]
    atmosphere: Option<crate::atmosphere::AtmosphericReferenceSpec>,
    #[serde(default, deserialize_with = "atmospheric_transport_spec")]
    atmospheric_transport: Option<crate::atmospheric_transport::AtmosphericTransportSpec>,
    #[serde(default, deserialize_with = "cad_variant_spec")]
    cad_variant: Option<crate::cad_variant::CadVariantSpec>,
    #[serde(default, deserialize_with = "cad_transport_spec")]
    cad_transport: Option<crate::cad_transport::CadSpectralTransportSpec>,
    #[serde(default, deserialize_with = "cooling_execution_spec")]
    retained_cooling: Option<crate::cooling_execution::CoolingExecutionSpec>,
}

fn cooling_execution_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::cooling_execution::CoolingExecutionSpec>, D::Error> {
    crate::cooling_execution::CoolingExecutionSpec::deserialize(decoder).map(Some)
}

fn cad_transport_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::cad_transport::CadSpectralTransportSpec>, D::Error> {
    crate::cad_transport::CadSpectralTransportSpec::deserialize(decoder).map(Some)
}

fn atmospheric_transport_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::atmospheric_transport::AtmosphericTransportSpec>, D::Error> {
    crate::atmospheric_transport::AtmosphericTransportSpec::deserialize(decoder).map(Some)
}
fn cad_variant_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::cad_variant::CadVariantSpec>, D::Error> {
    crate::cad_variant::CadVariantSpec::deserialize(decoder).map(Some)
}

fn atmospheric_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::atmosphere::AtmosphericReferenceSpec>, D::Error> {
    crate::atmosphere::AtmosphericReferenceSpec::deserialize(decoder).map(Some)
}

fn spectral_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::radiation::SpectralReferenceSpec>, D::Error> {
    crate::radiation::SpectralReferenceSpec::deserialize(decoder).map(Some)
}

fn freezing_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::freezing::FreezingReferenceSpec>, D::Error> {
    crate::freezing::FreezingReferenceSpec::deserialize(decoder).map(Some)
}

fn thermal_contact_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::thermal_contact::ThermalContactSpec>, D::Error> {
    crate::thermal_contact::ThermalContactSpec::deserialize(decoder).map(Some)
}

fn wetting_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::wetting::WettingReferenceSpec>, D::Error> {
    crate::wetting::WettingReferenceSpec::deserialize(decoder).map(Some)
}
fn contact_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::contact::ContactReferenceSpec>, D::Error> {
    crate::contact::ContactReferenceSpec::deserialize(decoder).map(Some)
}

fn imported_fem<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::fem_imported::ImportedFemSpec>, D::Error> {
    crate::fem_imported::ImportedFemSpec::deserialize(decoder).map(Some)
}

fn cad_source<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::cad_source::CadSource>, D::Error> {
    crate::cad_source::CadSource::deserialize(decoder).map(Some)
}

fn channel_case<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<CaseSpec>, D::Error> {
    CaseSpec::deserialize(decoder).map(Some)
}
fn fem_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::fem::FemReferenceSpec>, D::Error> {
    crate::fem::FemReferenceSpec::deserialize(decoder).map(Some)
}
fn thermal_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<crate::thermal::ThermalReferenceSpec>, D::Error> {
    crate::thermal::ThermalReferenceSpec::deserialize(decoder).map(Some)
}

fn filter_spec<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<FilterSpec>, D::Error> {
    FilterSpec::deserialize(decoder).map(Some)
}

fn frame_source<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<FrameSource>, D::Error> {
    FrameSource::deserialize(decoder).map(Some)
}

fn retained_source<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<RetainedSource>, D::Error> {
    RetainedSource::deserialize(decoder).map(Some)
}

impl<'de> Deserialize<'de> for ExecutionPlan {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        use serde::de::Error;
        let plan = ExecutionPlanRecord::deserialize(decoder)?;
        if plan.retained_cooling.is_some() {
            if !plan.retained_cooling_envelope() {
                return Err(D::Error::custom(
                    "strict source-bound version-18 independently approved retained cooling required",
                ));
            }
            return Ok(plan);
        }
        if plan.cad_transport.is_some() {
            if !plan.cad_transport_envelope() {
                return Err(D::Error::custom(
                    "strict source-bound version-17 direct CAD optical transport required",
                ));
            }
            return Ok(plan);
        }
        if plan.cad_variant.is_some() {
            if !plan.cad_variant_envelope() {
                return Err(D::Error::custom(
                    "strict source-bound version-16 CAD variant required",
                ));
            }
            return Ok(plan);
        }
        if plan.atmospheric_transport.is_some() {
            if !plan.atmospheric_transport_envelope() {
                return Err(D::Error::custom(
                    "strict source-bound version-15 atmospheric transport recipe required",
                ));
            }
            return Ok(plan);
        }
        if plan.atmosphere.is_some() {
            if !plan.atmospheric_envelope() {
                return Err(D::Error::custom(
                    "strict independent version-14 molecular atmospheric recipe required",
                ));
            }
            return Ok(plan);
        }
        if plan.spectral.is_some() {
            if !plan.spectral_envelope() {
                return Err(D::Error::custom(
                    "strict independent version-13 directional spectral recipe required",
                ));
            }
            return Ok(plan);
        }
        if plan.freezing.is_some() {
            if !plan.freezing_envelope() {
                return Err(D::Error::custom(
                    "strict independent version-12 freezing recipe required",
                ));
            }
            return Ok(plan);
        }
        if plan.thermal_contact.is_some() {
            if !plan.thermal_contact_envelope() {
                return Err(D::Error::custom(
                    "strict independent version-11 thermal contact recipe required",
                ));
            }
            return Ok(plan);
        }
        if plan.contact.is_some() {
            if !plan.contact_envelope() {
                return Err(D::Error::custom(
                    "strict independent version-10 contact recipe required",
                ));
            }
            return Ok(plan);
        }
        if plan.wetting.is_some() {
            if !plan.wetting_envelope() {
                return Err(D::Error::custom(
                    "strict independent version-9 wetting recipe required",
                ));
            }
            return Ok(plan);
        }
        match (
            plan.schema_version,
            &plan.source,
            &plan.frames,
            &plan.filter,
            &plan.case,
            &plan.fem,
            &plan.thermal,
            &plan.cad_source,
            &plan.imported_fem,
        ) {
            (1, None, None, None, Some(_), None, None, None, None)
            | (2, Some(_), None, None, Some(_), None, None, None, None)
            | (3, Some(_), Some(_), None, Some(_), None, None, None, None)
            | (4, Some(_), None, Some(_), Some(_), None, None, None, None)
            | (5, None, None, None, None, Some(_), None, None, None)
            | (6, None, None, None, None, None, Some(_), None, None)
            | (7, None, None, None, None, None, None, Some(_), None)
            | (8, None, None, None, None, None, None, Some(_), Some(_)) => {}
            _ => {
                return Err(D::Error::custom(
                    "strict versioned v1-v8 scientific recipe and original source envelope required",
                ));
            }
        }
        Ok(plan)
    }
}

impl JsonSchema for ExecutionPlan {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ExecutionPlan".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let mut schema = ExecutionPlanRecord::json_schema(generator);
        if let Some(properties) = schema.get_mut("properties") {
            properties["schema_version"]["enum"] = serde_json::json!([
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18
            ]);
        }
        schema.insert("allOf".into(), serde_json::json!([
            {"if":{"properties":{"schema_version":{"const":1}}},"then":{"not":{"anyOf":[{"required":["source"]},{"required":["frames"]},{"required":["filter"]}]}}},
            {"if":{"properties":{"schema_version":{"const":2}}},"then":{"required":["source"],"properties":{"source":{"type":"object"}},"not":{"anyOf":[{"required":["frames"]},{"required":["filter"]}]}}},
            {"if":{"properties":{"schema_version":{"const":3}}},"then":{"required":["source","frames"],"properties":{"source":{"type":"object"},"frames":{"type":"object"}},"not":{"required":["filter"]}}},
            {"if":{"properties":{"schema_version":{"const":4}}},"then":{"required":["source","filter"],"properties":{"source":{"type":"object"},"filter":{"type":"object"}},"not":{"required":["frames"]}}},
            {"if":{"properties":{"schema_version":{"const":5}}},"then":{"required":["fem"],"properties":{"fem":{"type":"object"}},"not":{"anyOf":[{"required":["case"]},{"required":["source"]},{"required":["frames"]},{"required":["filter"]},{"required":["thermal"]}]}}},
            {"if":{"properties":{"schema_version":{"const":6}}},"then":{"required":["thermal"],"properties":{"thermal":{"type":"object"}},"not":{"anyOf":[{"required":["case"]},{"required":["source"]},{"required":["frames"]},{"required":["filter"]},{"required":["fem"]}]}},"else":{"not":{"required":["thermal"]}}},
            {"if":{"properties":{"schema_version":{"enum":[1,2,3,4]}}},"then":{"required":["case"],"properties":{"case":{"type":"object"}},"not":{"required":["fem"]}}},
            {"if":{"properties":{"schema_version":{"enum":[7,8]}}},"then":{"required":["cad_source"],"properties":{"cad_source":{"type":"object"}},"not":{"anyOf":[{"required":["case"]},{"required":["source"]},{"required":["frames"]},{"required":["filter"]},{"required":["fem"]},{"required":["thermal"]}]}},"else":{"not":{"required":["cad_source"]}}},
            {"if":{"properties":{"schema_version":{"const":8}}},"then":{"required":["imported_fem"],"properties":{"imported_fem":{"type":"object"}}},"else":{"not":{"required":["imported_fem"]}}},
            {"if":{"properties":{"schema_version":{"const":9}}},"then":{"required":["wetting"],"properties":{"wetting":{"type":"object"}},"not":{"anyOf":[{"required":["case"]},{"required":["source"]},{"required":["frames"]},{"required":["filter"]},{"required":["fem"]},{"required":["thermal"]},{"required":["cad_source"]},{"required":["imported_fem"]},{"required":["contact"]}]}},"else":{"not":{"required":["wetting"]}}},
            {"if":{"properties":{"schema_version":{"const":10}}},"then":{"required":["contact"],"properties":{"contact":{"type":"object"}},"not":{"anyOf":[{"required":["case"]},{"required":["source"]},{"required":["frames"]},{"required":["filter"]},{"required":["fem"]},{"required":["thermal"]},{"required":["cad_source"]},{"required":["imported_fem"]},{"required":["wetting"]}]}},"else":{"not":{"required":["contact"]}}},
            {"if":{"properties":{"schema_version":{"const":11}}},"then":{"required":["thermal_contact"],"properties":{"thermal_contact":{"type":"object"}},"not":{"anyOf":[{"required":["case"]},{"required":["source"]},{"required":["frames"]},{"required":["filter"]},{"required":["fem"]},{"required":["thermal"]},{"required":["cad_source"]},{"required":["imported_fem"]},{"required":["wetting"]},{"required":["contact"]}]}},"else":{"not":{"required":["thermal_contact"]}}},
            {"if":{"properties":{"schema_version":{"const":12}}},"then":{"required":["freezing"],"properties":{"freezing":{"type":"object"}},"not":{"anyOf":[{"required":["case"]},{"required":["source"]},{"required":["frames"]},{"required":["filter"]},{"required":["fem"]},{"required":["thermal"]},{"required":["cad_source"]},{"required":["imported_fem"]},{"required":["wetting"]},{"required":["contact"]},{"required":["thermal_contact"]}]}},"else":{"not":{"required":["freezing"]}}}
        ]));
        let spectral_schema = serde_json::json!(
            {"if":{"properties":{"schema_version":{"const":13}}},"then":{"required":["spectral"],"properties":{"spectral":{"type":"object"}},"not":{"anyOf":[{"required":["case"]},{"required":["source"]},{"required":["frames"]},{"required":["filter"]},{"required":["fem"]},{"required":["thermal"]},{"required":["cad_source"]},{"required":["imported_fem"]},{"required":["wetting"]},{"required":["contact"]},{"required":["thermal_contact"]},{"required":["freezing"]}]}},"else":{"not":{"required":["spectral"]}}}
        );
        if let Some(conditions) = schema
            .get_mut("allOf")
            .and_then(serde_json::Value::as_array_mut)
        {
            conditions.push(spectral_schema);
            conditions.push(serde_json::json!(
                {"if":{"properties":{"schema_version":{"const":14}}},"then":{"required":["atmosphere"],"properties":{"atmosphere":{"type":"object"}},"not":{"anyOf":[{"required":["case"]},{"required":["source"]},{"required":["frames"]},{"required":["filter"]},{"required":["fem"]},{"required":["thermal"]},{"required":["cad_source"]},{"required":["imported_fem"]},{"required":["wetting"]},{"required":["contact"]},{"required":["thermal_contact"]},{"required":["freezing"]},{"required":["spectral"]}]}},"else":{"not":{"required":["atmosphere"]}}}
            ));
            conditions.push(serde_json::json!(
                {"if":{"properties":{"schema_version":{"const":15}}},"then":{"required":["atmospheric_transport"],"properties":{"atmospheric_transport":{"type":"object"}},"not":{"anyOf":[{"required":["case"]},{"required":["source"]},{"required":["frames"]},{"required":["filter"]},{"required":["fem"]},{"required":["thermal"]},{"required":["cad_source"]},{"required":["imported_fem"]},{"required":["wetting"]},{"required":["contact"]},{"required":["thermal_contact"]},{"required":["freezing"]},{"required":["spectral"]},{"required":["atmosphere"]}]}},"else":{"not":{"required":["atmospheric_transport"]}}}
            ));
            conditions.push(serde_json::json!({"if":{"properties":{"schema_version":{"const":16}}},"then":{"required":["cad_variant","case"],"properties":{"cad_variant":{"type":"object"},"case":{"type":"object"}},"not":{"anyOf":[{"required":["source"]},{"required":["frames"]},{"required":["filter"]},{"required":["fem"]},{"required":["thermal"]},{"required":["cad_source"]},{"required":["imported_fem"]},{"required":["wetting"]},{"required":["contact"]},{"required":["thermal_contact"]},{"required":["freezing"]},{"required":["spectral"]},{"required":["atmosphere"]},{"required":["atmospheric_transport"]}]}},"else":{"not":{"required":["cad_variant"]}}}));
        }
        if let Some(conditions) = schema
            .get_mut("allOf")
            .and_then(serde_json::Value::as_array_mut)
        {
            let prohibited = [
                "case",
                "source",
                "frames",
                "filter",
                "fem",
                "thermal",
                "cad_source",
                "imported_fem",
                "wetting",
                "contact",
                "thermal_contact",
                "freezing",
                "spectral",
                "atmosphere",
                "atmospheric_transport",
                "cad_variant",
            ]
            .map(|key| serde_json::json!({"required":[key]}));
            conditions.push(serde_json::json!({"if":{"properties":{"schema_version":{"const":17}}},"then":{"required":["cad_transport"],"properties":{"cad_transport":{"type":"object"}},"not":{"anyOf":prohibited}},"else":{"not":{"required":["cad_transport"]}}}));
            let prohibited = prohibited
                .into_iter()
                .chain(std::iter::once(
                    serde_json::json!({"required":["cad_transport"]}),
                ))
                .collect::<Vec<_>>();
            conditions.push(serde_json::json!({"if":{"properties":{"schema_version":{"const":18}}},"then":{"required":["retained_cooling"],"properties":{"retained_cooling":{"type":"object"}},"not":{"anyOf":prohibited}},"else":{"not":{"required":["retained_cooling"]}}}));
        }
        schema
    }
}
impl ExecutionPlan {
    fn retained_cooling_envelope(&self) -> bool {
        self.schema_version == 18
            && self.retained_cooling.is_some()
            && self.case.is_none()
            && self.source.is_none()
            && self.frames.is_none()
            && self.filter.is_none()
            && self.fem.is_none()
            && self.thermal.is_none()
            && self.cad_source.is_none()
            && self.imported_fem.is_none()
            && self.wetting.is_none()
            && self.contact.is_none()
            && self.thermal_contact.is_none()
            && self.freezing.is_none()
            && self.spectral.is_none()
            && self.atmosphere.is_none()
            && self.atmospheric_transport.is_none()
            && self.cad_variant.is_none()
            && self.cad_transport.is_none()
    }
    fn cad_transport_envelope(&self) -> bool {
        self.schema_version == 17
            && self.cad_transport.is_some()
            && self.case.is_none()
            && self.source.is_none()
            && self.frames.is_none()
            && self.filter.is_none()
            && self.fem.is_none()
            && self.thermal.is_none()
            && self.cad_source.is_none()
            && self.imported_fem.is_none()
            && self.wetting.is_none()
            && self.contact.is_none()
            && self.thermal_contact.is_none()
            && self.freezing.is_none()
            && self.spectral.is_none()
            && self.atmosphere.is_none()
            && self.atmospheric_transport.is_none()
            && self.cad_variant.is_none()
    }
    fn cad_variant_envelope(&self) -> bool {
        self.schema_version == 16
            && self.cad_variant.is_some()
            && self.case.is_some()
            && self.source.is_none()
            && self.frames.is_none()
            && self.filter.is_none()
            && self.fem.is_none()
            && self.thermal.is_none()
            && self.cad_source.is_none()
            && self.imported_fem.is_none()
            && self.wetting.is_none()
            && self.contact.is_none()
            && self.thermal_contact.is_none()
            && self.freezing.is_none()
            && self.spectral.is_none()
            && self.atmosphere.is_none()
            && self.atmospheric_transport.is_none()
    }
    fn atmospheric_transport_envelope(&self) -> bool {
        self.schema_version == 15
            && self.atmospheric_transport.is_some()
            && self.atmosphere.is_none()
            && self.spectral.is_none()
            && self.freezing.is_none()
            && self.thermal_contact.is_none()
            && self.contact.is_none()
            && self.wetting.is_none()
            && self.case.is_none()
            && self.fem.is_none()
            && self.thermal.is_none()
            && self.source.is_none()
            && self.frames.is_none()
            && self.filter.is_none()
            && self.cad_source.is_none()
            && self.imported_fem.is_none()
    }
    fn atmospheric_envelope(&self) -> bool {
        self.schema_version == 14
            && self.atmospheric_transport.is_none()
            && self.atmosphere.is_some()
            && self.spectral.is_none()
            && self.freezing.is_none()
            && self.thermal_contact.is_none()
            && self.contact.is_none()
            && self.wetting.is_none()
            && self.case.is_none()
            && self.fem.is_none()
            && self.thermal.is_none()
            && self.source.is_none()
            && self.frames.is_none()
            && self.filter.is_none()
            && self.cad_source.is_none()
            && self.imported_fem.is_none()
    }
    fn spectral_envelope(&self) -> bool {
        self.schema_version == 13
            && self.atmospheric_transport.is_none()
            && self.atmosphere.is_none()
            && self.spectral.is_some()
            && self.freezing.is_none()
            && self.thermal_contact.is_none()
            && self.contact.is_none()
            && self.wetting.is_none()
            && self.case.is_none()
            && self.fem.is_none()
            && self.thermal.is_none()
            && self.source.is_none()
            && self.frames.is_none()
            && self.filter.is_none()
            && self.cad_source.is_none()
            && self.imported_fem.is_none()
    }
    fn freezing_envelope(&self) -> bool {
        self.schema_version == 12
            && self.atmospheric_transport.is_none()
            && self.atmosphere.is_none()
            && self.spectral.is_none()
            && self.freezing.is_some()
            && self.thermal_contact.is_none()
            && self.contact.is_none()
            && self.wetting.is_none()
            && self.case.is_none()
            && self.fem.is_none()
            && self.thermal.is_none()
            && self.source.is_none()
            && self.frames.is_none()
            && self.filter.is_none()
            && self.cad_source.is_none()
            && self.imported_fem.is_none()
    }
    fn thermal_contact_envelope(&self) -> bool {
        self.schema_version == 11
            && self.atmospheric_transport.is_none()
            && self.atmosphere.is_none()
            && self.spectral.is_none()
            && self.freezing.is_none()
            && self.thermal_contact.is_some()
            && self.contact.is_none()
            && self.wetting.is_none()
            && self.case.is_none()
            && self.fem.is_none()
            && self.thermal.is_none()
            && self.source.is_none()
            && self.frames.is_none()
            && self.filter.is_none()
            && self.cad_source.is_none()
            && self.imported_fem.is_none()
    }
    fn contact_envelope(&self) -> bool {
        self.schema_version == 10
            && self.atmospheric_transport.is_none()
            && self.atmosphere.is_none()
            && self.spectral.is_none()
            && self.freezing.is_none()
            && self.thermal_contact.is_none()
            && self.contact.is_some()
            && self.wetting.is_none()
            && self.case.is_none()
            && self.fem.is_none()
            && self.thermal.is_none()
            && self.source.is_none()
            && self.frames.is_none()
            && self.filter.is_none()
            && self.cad_source.is_none()
            && self.imported_fem.is_none()
    }
    fn wetting_envelope(&self) -> bool {
        self.schema_version == 9
            && self.atmospheric_transport.is_none()
            && self.atmosphere.is_none()
            && self.spectral.is_none()
            && self.freezing.is_none()
            && self.thermal_contact.is_none()
            && self.contact.is_none()
            && self.wetting.is_some()
            && self.case.is_none()
            && self.fem.is_none()
            && self.thermal.is_none()
            && self.source.is_none()
            && self.frames.is_none()
            && self.filter.is_none()
            && self.cad_source.is_none()
            && self.imported_fem.is_none()
    }
    pub fn channel_case(&self) -> Result<&CaseSpec> {
        self.case
            .as_ref()
            .ok_or_else(|| invalid("fluid/CAD case required for this operation"))
    }
    pub fn science_id(&self) -> Result<String> {
        if let Some(spec) = &self.retained_cooling {
            if !self.retained_cooling_envelope() {
                return Err(invalid(
                    "independent v18 source-bound retained cooling required",
                ));
            }
            return digest(spec);
        }
        if let Some(spec) = &self.cad_transport {
            if !self.cad_transport_envelope() {
                return Err(invalid(
                    "independent v17 direct CAD optical recipe required",
                ));
            }
            return digest(spec);
        }
        if let Some(spec) = &self.cad_variant {
            if !self.cad_variant_envelope() {
                return Err(invalid("independent v16 controlled CAD variant required"));
            }
            return digest(&(self.channel_case()?.science_id()?, spec));
        }
        if let Some(spec) = &self.atmospheric_transport {
            if !self.atmospheric_transport_envelope() {
                return Err(invalid(
                    "one source-bound atmospheric transport scientific recipe required",
                ));
            }
            return digest(spec);
        }
        if let Some(spec) = &self.atmosphere {
            if !self.atmospheric_envelope() {
                return Err(invalid(
                    "one independent atmospheric scientific recipe required",
                ));
            }
            return digest(spec);
        }
        if let Some(spec) = &self.spectral {
            if !self.spectral_envelope() {
                return Err(invalid(
                    "one independent spectral scientific recipe required",
                ));
            }
            return digest(spec);
        }
        if let Some(spec) = &self.freezing {
            if !self.freezing_envelope() {
                return Err(invalid("one independent scientific recipe required"));
            }
            return digest(spec);
        }
        if let Some(spec) = &self.thermal_contact {
            if !self.thermal_contact_envelope() {
                return Err(invalid("one independent scientific recipe required"));
            }
            return digest(spec);
        }
        if let Some(contact) = &self.contact {
            if !self.contact_envelope() {
                return Err(invalid("one independent scientific recipe required"));
            }
            return digest(contact);
        }
        if let Some(wetting) = &self.wetting {
            if !self.wetting_envelope() {
                return Err(invalid("one independent scientific recipe required"));
            }
            return digest(wetting);
        }
        match (
            &self.case,
            &self.fem,
            &self.thermal,
            &self.cad_source,
            &self.imported_fem,
        ) {
            (Some(case), None, None, None, None) => case.science_id(),
            (None, Some(fem), None, None, None) => digest(fem),
            (None, None, Some(thermal), None, None) => digest(thermal),
            (None, None, None, Some(source), None) => digest(source),
            (None, None, None, Some(source), Some(fem)) => digest(&(source, fem)),
            _ => Err(invalid("one scientific recipe required")),
        }
    }
    pub fn numerical_filter(
        original: &Self,
        source: RetainedSource,
        request: FilterRequest,
        policy: String,
    ) -> Result<Self> {
        if source.job_id != request.source_job
            || source.plan_digest != original.id()?
            || source.science_id != original.science_id()?
            || !original
                .observation
                .retained_times_s
                .contains(&request.filter.time_s)
            || request.compute.role != Role::Compute
            || request.compute.backend != "hip"
            || request.compute.backend_uuid.is_none()
        {
            return Err(invalid(
                "numerical filter requires exact retained source and HIP compute selection",
            ));
        }
        let mut plan = Self {
            schema_version: 4,
            case: original.case.clone(),
            source: Some(source),
            frames: None,
            filter: Some(request.filter.clone()),
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
            cad_transport: None,
            retained_cooling: None,
            stages: vec![
                Stage {
                    id: "filter".into(),
                    dependencies: vec![],
                    operation: StageOperation::NumericalFilter,
                    gpu: GpuRequirement::Required,
                    selection: Some(request.compute),
                    ram_bytes: 1,
                    vram_bytes: 1,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec!["filter".into()],
                    operation: StageOperation::Bundle,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 16 * 1024 * 1024,
                    vram_bytes: 0,
                },
            ],
            transfers: vec![],
            observation: ObservationPlan {
                metrics: vec![],
                probes: vec![],
                retained_times_s: vec![request.filter.time_s],
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
    pub fn video(
        render: &Self,
        source: RetainedSource,
        frames: FrameSource,
        request: VideoRequest,
        policy: String,
    ) -> Result<Self> {
        if frames.job_id != request.source_job
            || frames.plan_digest != render.id()?
            || render.science_id()? != source.science_id
            || !render
                .stages
                .iter()
                .any(|s| matches!(s.operation, StageOperation::Render))
            || request.media.role != Role::Media
        {
            return Err(invalid(
                "video requires approved rendered-frame science and an independent media role",
            ));
        }
        let mut plan = Self {
            schema_version: 3,
            case: render.case.clone(),
            source: Some(source),
            frames: Some(frames),
            filter: None,
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
            cad_transport: None,
            retained_cooling: None,
            stages: vec![
                Stage {
                    id: "video".into(),
                    dependencies: vec![],
                    operation: StageOperation::Video,
                    gpu: GpuRequirement::Required,
                    selection: Some(request.media),
                    ram_bytes: 1,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec!["video".into()],
                    operation: StageOperation::Bundle,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 16 * 1024 * 1024,
                    vram_bytes: 0,
                },
            ],
            transfers: vec![],
            observation: ObservationPlan {
                metrics: vec![],
                probes: vec![],
                retained_times_s: render.observation.retained_times_s.clone(),
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
    pub fn presentation(
        original: &Self,
        source: RetainedSource,
        request: PresentationRequest,
        policy: String,
    ) -> Result<Self> {
        if source.job_id != request.source_job
            || source.plan_digest != original.id()?
            || source.science_id != original.science_id()?
            || request
                .times_s
                .iter()
                .any(|t| !original.observation.retained_times_s.contains(t))
            || request.render.role != Role::Render
            || request
                .media
                .as_ref()
                .is_some_and(|s| s.role != Role::Media)
        {
            return Err(invalid(
                "presentation source, retained times or operation roles differ",
            ));
        }
        let mut case = original.channel_case()?.clone();
        case.presentation = request.presentation;
        let mut stages = vec![Stage {
            id: "render".into(),
            dependencies: vec![],
            operation: StageOperation::Render,
            gpu: GpuRequirement::Required,
            selection: Some(request.render),
            ram_bytes: 1,
            vram_bytes: 0,
        }];
        if let Some(media) = request.media {
            stages.push(Stage {
                id: "video".into(),
                dependencies: vec!["render".into()],
                operation: StageOperation::Video,
                gpu: GpuRequirement::Required,
                selection: Some(media),
                ram_bytes: 1,
                vram_bytes: 0,
            });
        }
        stages.push(Stage {
            id: "bundle".into(),
            dependencies: vec![if stages.len() == 1 { "render" } else { "video" }.into()],
            operation: StageOperation::Bundle,
            gpu: GpuRequirement::CpuOnly,
            selection: None,
            ram_bytes: 16 * 1024 * 1024,
            vram_bytes: 0,
        });
        let mut plan = Self {
            schema_version: 2,
            case: Some(case),
            stages,
            transfers: vec![],
            observation: ObservationPlan {
                metrics: vec![],
                probes: vec![],
                retained_times_s: request.times_s,
                checkpoint_times_s: vec![],
                preview_times_s: vec![],
                max_artifact_bytes: 0,
                scientific_congestion: "fail".into(),
                preview_may_drop: false,
            },
            fleetix_revision: FLEETIX_REV.into(),
            fleetix_contract_digest: fleetix_digest(),
            policy,
            source: Some(source),
            frames: None,
            filter: None,
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
            cad_transport: None,
            retained_cooling: None,
        };
        crate::estimates::minimum(&plan)?.apply(&mut plan);
        plan.validate()?;
        Ok(plan)
    }
    pub fn reference(case: CaseSpec) -> Result<Self> {
        let ram_bytes = 16 * 1024 * 1024 + u64::from(case.resolution) * 160;
        let max_artifact_bytes = 1048576u64.max(u64::from(case.resolution) * 128);
        let plan = Self {
            schema_version: 1,
            case: Some(case),
            stages: vec![Stage {
                id: "reference".into(),
                dependencies: vec![],
                operation: StageOperation::ChannelReference,
                gpu: GpuRequirement::CpuOnly,
                selection: None,
                ram_bytes,
                vram_bytes: 0,
            }],
            transfers: vec![],
            observation: ObservationPlan {
                metrics: vec!["mean_velocity".into()],
                probes: vec![],
                retained_times_s: vec![0.],
                checkpoint_times_s: vec![],
                preview_times_s: vec![],
                max_artifact_bytes,
                scientific_congestion: "fail".into(),
                preview_may_drop: false,
            },
            fleetix_revision: FLEETIX_REV.into(),
            fleetix_contract_digest: fleetix_digest(),
            policy: "ci".into(),
            source: None,
            frames: None,
            filter: None,
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
            cad_transport: None,
            retained_cooling: None,
        };
        plan.validate()?;
        Ok(plan)
    }
    /// Inspect approved source CAD in the native importer; no solver is implied.
    pub fn cad_inspection(case: CaseSpec, policy: String, max_artifact_bytes: u64) -> Result<Self> {
        if policy == "ci" {
            return Err(invalid("native CAD inspection requires systemd policy"));
        }
        let plan = Self {
            schema_version: 1,
            case: Some(case),
            stages: vec![
                Stage {
                    id: "cad".into(),
                    dependencies: vec![],
                    operation: StageOperation::CadInspect,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 1024 * 1024 * 1024,
                    vram_bytes: 0,
                },
                Stage {
                    id: "bundle".into(),
                    dependencies: vec!["cad".into()],
                    operation: StageOperation::Bundle,
                    gpu: GpuRequirement::CpuOnly,
                    selection: None,
                    ram_bytes: 16 * 1024 * 1024,
                    vram_bytes: 0,
                },
            ],
            transfers: vec![],
            observation: ObservationPlan {
                metrics: vec!["geometry_regions".into()],
                probes: vec![],
                retained_times_s: vec![],
                checkpoint_times_s: vec![],
                preview_times_s: vec![],
                max_artifact_bytes,
                scientific_congestion: "fail".into(),
                preview_may_drop: false,
            },
            fleetix_revision: FLEETIX_REV.into(),
            fleetix_contract_digest: fleetix_digest(),
            policy,
            source: None,
            frames: None,
            filter: None,
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
            cad_transport: None,
            retained_cooling: None,
        };
        plan.validate()?;
        Ok(plan)
    }
    pub fn openlb_reference(case: CaseSpec, policy: String) -> Result<Self> {
        case.validate()?;
        if !case.geometry.synthetic
            || case.applicability.formulation != "periodic_forced_channel"
            || policy == "ci"
        {
            return Err(invalid(
                "native CPU reference requires synthetic periodic channel and systemd policy",
            ));
        }
        let stages = vec![
            Stage {
                id: "cad".into(),
                dependencies: vec![],
                operation: StageOperation::CadFixture,
                gpu: GpuRequirement::CpuOnly,
                selection: None,
                ram_bytes: 1024 * 1024 * 1024,
                vram_bytes: 0,
            },
            Stage {
                id: "flow".into(),
                dependencies: vec!["cad".into()],
                operation: StageOperation::Openlb,
                gpu: GpuRequirement::CpuOnly,
                selection: None,
                ram_bytes: 0,
                vram_bytes: 0,
            },
            Stage {
                id: "bundle".into(),
                dependencies: vec!["flow".into()],
                operation: StageOperation::Bundle,
                gpu: GpuRequirement::CpuOnly,
                selection: None,
                ram_bytes: 16 * 1024 * 1024,
                vram_bytes: 0,
            },
        ];
        let end = case.max_time_s;
        let mut plan = Self {
            schema_version: 1,
            case: Some(case),
            stages,
            transfers: vec![],
            observation: ObservationPlan {
                metrics: vec!["channel_relative_l2_error".into()],
                probes: vec![],
                retained_times_s: vec![0., end / 2., end],
                checkpoint_times_s: vec![],
                preview_times_s: vec![],
                max_artifact_bytes: 0,
                scientific_congestion: "fail".into(),
                preview_may_drop: false,
            },
            fleetix_revision: FLEETIX_REV.into(),
            fleetix_contract_digest: fleetix_digest(),
            policy,
            source: None,
            frames: None,
            filter: None,
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
            cad_transport: None,
            retained_cooling: None,
        };
        crate::estimates::minimum(&plan)?.apply(&mut plan);
        plan.validate()?;
        Ok(plan)
    }
    pub fn id(&self) -> Result<String> {
        digest(self)
    }
    pub fn peak_ram(&self) -> u64 {
        self.stages.iter().map(|s| s.ram_bytes).max().unwrap_or(0)
    }
    pub fn disk_reservation(&self) -> Result<u64> {
        let copies = if self.stages.iter().any(|s| {
            !matches!(
                s.operation,
                StageOperation::ChannelReference | StageOperation::Bundle
            )
        }) {
            2
        } else {
            1
        };
        self.observation
            .max_artifact_bytes
            .checked_mul(copies)
            .ok_or_else(|| invalid("disk reservation overflow"))
    }
    pub fn b1(case: CaseSpec, selections: B1Selections, policy: String) -> Result<Self> {
        if selections.compute.role != Role::Compute
            || !["hip", "cuda"].contains(&selections.compute.backend.as_str())
            || selections
                .compute
                .backend_uuid
                .as_ref()
                .is_none_or(|id| !token(id))
            || selections.render.role != Role::Render
            || selections.render.backend != "egl"
            || selections.media.role != Role::Media
            || selections.media.backend != "vaapi"
        {
            return Err(invalid(
                "B1 requires independent HIP or CUDA UUID/PCI, EGL and VAAPI selections",
            ));
        }
        let mut plan = Self::openlb_reference(case, policy)?;
        plan.stages.pop();
        let flow = &mut plan.stages[1];
        flow.gpu = GpuRequirement::Required;
        flow.selection = Some(selections.compute);
        flow.vram_bytes = flow.ram_bytes;
        let pixels = u64::from(plan.channel_case()?.presentation.width)
            * u64::from(plan.channel_case()?.presentation.height);
        plan.stages.extend([
            Stage {
                id: "render".into(),
                dependencies: vec!["flow".into()],
                operation: StageOperation::Render,
                gpu: GpuRequirement::Required,
                selection: Some(selections.render),
                ram_bytes: 1024 * 1024 * 1024,
                vram_bytes: 128 * 1024 * 1024 + pixels * 32,
            },
            Stage {
                id: "video".into(),
                dependencies: vec!["render".into()],
                operation: StageOperation::Video,
                gpu: GpuRequirement::Required,
                selection: Some(selections.media),
                ram_bytes: 512 * 1024 * 1024,
                vram_bytes: 128 * 1024 * 1024 + pixels * 16,
            },
            Stage {
                id: "bundle".into(),
                dependencies: vec!["video".into()],
                operation: StageOperation::Bundle,
                gpu: GpuRequirement::CpuOnly,
                selection: None,
                ram_bytes: 16 * 1024 * 1024,
                vram_bytes: 0,
            },
        ]);
        crate::estimates::minimum(&plan)?.apply(&mut plan);
        plan.validate()?;
        Ok(plan)
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(case) = &self.case {
            case.validate()?;
        }
        if let Some(fem) = &self.fem {
            fem.validate()?;
        }
        if (self.retained_cooling.is_some() && !self.retained_cooling_envelope())
            || (self.cad_transport.is_some() && !self.cad_transport_envelope())
            || (self.cad_variant.is_some() && !self.cad_variant_envelope())
            || !(self.retained_cooling_envelope()
                || self.cad_transport_envelope()
                || self.cad_variant_envelope()
                || self.atmospheric_transport_envelope()
                || self.atmospheric_envelope()
                || self.spectral_envelope()
                || self.freezing_envelope()
                || self.thermal_contact_envelope()
                || self.contact_envelope()
                || self.wetting_envelope()
                || (self.atmospheric_transport.is_none()
                    && self.atmosphere.is_none()
                    && self.spectral.is_none()
                    && self.freezing.is_none()
                    && self.thermal_contact.is_none()
                    && self.contact.is_none()
                    && self.wetting.is_none()
                    && matches!(
                        (
                            self.schema_version,
                            &self.source,
                            &self.frames,
                            &self.filter,
                            &self.case,
                            &self.fem,
                            &self.thermal,
                            &self.cad_source,
                            &self.imported_fem,
                        ),
                        (1, None, None, None, Some(_), None, None, None, None)
                            | (2, Some(_), None, None, Some(_), None, None, None, None)
                            | (3, Some(_), Some(_), None, Some(_), None, None, None, None)
                            | (4, Some(_), None, Some(_), Some(_), None, None, None, None)
                            | (5, None, None, None, None, Some(_), None, None, None)
                            | (6, None, None, None, None, None, Some(_), None, None)
                            | (7, None, None, None, None, None, None, Some(_), None)
                            | (8, None, None, None, None, None, None, Some(_), Some(_))
                    )))
            || self.fleetix_revision != FLEETIX_REV
            || self.fleetix_contract_digest != fleetix_digest()
        {
            return Err(invalid("schema/Fleetix source or contract drift"));
        }
        if !["ci", "prototype", "production", "research"].contains(&self.policy.as_str()) {
            return Err(invalid("policy"));
        }
        if let Some(spec) = &self.thermal_contact {
            spec.validate_plan(self)?;
        }
        if let Some(spec) = &self.retained_cooling {
            spec.validate_plan(self)?;
        }
        if let Some(spec) = &self.freezing {
            spec.validate_plan(self)?;
        }
        if let Some(spec) = &self.spectral {
            spec.validate_plan(self)?;
        }
        if let Some(spec) = &self.atmosphere {
            spec.validate_plan(self)?;
        }
        if let Some(spec) = &self.atmospheric_transport {
            spec.validate_plan(self)?;
        }
        if let Some(spec) = &self.cad_transport {
            spec.validate_plan(self)?;
        }
        if let Some(spec) = &self.cad_variant {
            spec.validate()?;
            let case = self.channel_case()?;
            if self.policy == "ci"
                || self.stages.len() != 2
                || self.stages[0].id != "cad"
                || self.stages[0].operation != StageOperation::CadInspect
                || !self.stages[0].dependencies.is_empty()
                || self.stages[1].id != "bundle"
                || self.stages[1].operation != StageOperation::Bundle
                || self.stages[1].dependencies != ["cad"]
                || self.stages.iter().any(|s| {
                    s.gpu != GpuRequirement::CpuOnly || s.selection.is_some() || s.vram_bytes != 0
                })
                || !self.transfers.is_empty()
                || self.observation.metrics != ["geometry_regions"]
                || !self.observation.probes.is_empty()
                || !self.observation.retained_times_s.is_empty()
                || !self.observation.preview_times_s.is_empty()
                || !self.observation.checkpoint_times_s.is_empty()
                || self.observation.preview_may_drop
                || case.regions != [spec.request.region_name.clone()]
                || case.geometry.synthetic != spec.source.geometry.synthetic
                || case.geometry.source != "source-document.FCStd"
                || case.geometry.sha256.as_deref() != Some(spec.document.sha256.as_str())
                || case.geometry_tolerance.si("length")?
                    != spec.request.geometry_tolerance.si("length")?
            {
                return Err(invalid(
                    "strict source-bound controlled CPU CAD-copy variant DAG and original region context required",
                ));
            }
        }
        if let Some(source) = &self.cad_source {
            source.validate()?;
            let imported = self.imported_fem.is_some();
            if let Some(spec) = &self.imported_fem {
                spec.validate(source)?;
            }
            if self.policy == "ci"
                || self.stages.len() != 2
                || self.stages[0].id != if imported { "fem-imported" } else { "mesh" }
                || !self.stages[0].dependencies.is_empty()
                || !matches!(
                    (&self.stages[0].operation, imported),
                    (StageOperation::CadMesh, false) | (StageOperation::FemImported, true)
                )
                || self.stages[1].id != "bundle"
                || self.stages[1].dependencies != [self.stages[0].id.clone()]
                || !matches!(self.stages[1].operation, StageOperation::Bundle)
                || !self.observation.retained_times_s.is_empty()
                || !self.observation.checkpoint_times_s.is_empty()
                || !self.observation.preview_times_s.is_empty()
                || !self.observation.probes.is_empty()
                || self.observation.preview_may_drop
                || if imported {
                    !self.observation.metrics.is_empty()
                } else {
                    self.observation.metrics != ["geometry_correspondence"]
                }
            {
                return Err(invalid(
                    "exact independent source-bound imported CAD mesh and bundle DAG required",
                ));
            }
        }
        if self.fem.is_some() {
            let operations: Vec<_> = self.stages.iter().map(|s| &s.operation).collect();
            if self.policy == "ci"
                || !matches!(
                    operations.as_slice(),
                    [StageOperation::FemReference, StageOperation::Bundle]
                )
                || !self.stages[0].dependencies.is_empty()
                || self.stages[1].dependencies != [self.stages[0].id.clone()]
                || !self.observation.metrics.is_empty()
                || !self.observation.probes.is_empty()
                || !self.observation.retained_times_s.is_empty()
                || !self.observation.checkpoint_times_s.is_empty()
                || !self.observation.preview_times_s.is_empty()
                || self.observation.preview_may_drop
            {
                return Err(invalid(
                    "static independent FEM CPU DAG required; no fluid case, physical-time observations or implicit coupling",
                ));
            }
        }
        if let Some(thermal) = &self.thermal {
            thermal.validate()?;
            let operations: Vec<_> = self.stages.iter().map(|s| &s.operation).collect();
            if self.policy == "ci"
                || !matches!(
                    operations.as_slice(),
                    [StageOperation::ThermalReference, StageOperation::Bundle]
                )
                || !self.stages[0].dependencies.is_empty()
                || self.stages[1].dependencies != [self.stages[0].id.clone()]
                || !self.observation.metrics.is_empty()
                || !self.observation.probes.is_empty()
                || self.observation.retained_times_s != thermal.observation_times_s
                || !self.observation.checkpoint_times_s.is_empty()
                || !self.observation.preview_times_s.is_empty()
                || self.observation.preview_may_drop
            {
                return Err(invalid(
                    "exact independent transient CPU thermal DAG and physical observations required",
                ));
            }
        }
        if let Some(wetting) = &self.wetting {
            wetting.validate()?;
            let operations: Vec<_> = self.stages.iter().map(|s| &s.operation).collect();
            if self.policy == "ci"
                || !matches!(
                    operations.as_slice(),
                    [StageOperation::WettingReference, StageOperation::Bundle]
                )
                || self.stages[0].id != "wetting"
                || !self.stages[0].dependencies.is_empty()
                || self.stages[1].id != "bundle"
                || self.stages[1].dependencies != ["wetting"]
                || !self.observation.metrics.is_empty()
                || !self.observation.probes.is_empty()
                || self.observation.retained_times_s != wetting.times_s()
                || !self.observation.checkpoint_times_s.is_empty()
                || !self.observation.preview_times_s.is_empty()
                || self.observation.preview_may_drop
            {
                return Err(invalid(
                    "exact independent CPU planar wetting DAG and retained native observations required",
                ));
            }
        }
        if let Some(contact) = &self.contact {
            contact.validate()?;
            if self.policy == "ci"
                || self.stages.len() != 2
                || self.stages[0].id != "contact"
                || self.stages[1].id != "bundle"
                || self.stages[0].operation != StageOperation::ContactReference
                || self.stages[1].operation != StageOperation::Bundle
                || !self.stages[0].dependencies.is_empty()
                || self.stages[1].dependencies != ["contact"]
                || !self.transfers.is_empty()
                || !self.observation.metrics.is_empty()
                || !self.observation.probes.is_empty()
                || !self.observation.retained_times_s.is_empty()
                || !self.observation.checkpoint_times_s.is_empty()
                || !self.observation.preview_times_s.is_empty()
                || self.observation.preview_may_drop
            {
                return Err(invalid(
                    "exact independent CPU two-static-state contact DAG required",
                ));
            }
        }
        if let Some(source) = &self.source {
            let hashes = [
                &source.plan_digest,
                &source.execution_binding_digest,
                &source.authorization_digest,
                &source.snapshot_sha256,
                &source.artifact_id,
                &source.science_id,
            ];
            if self.policy == "ci"
                || uuid::Uuid::parse_str(&source.job_id).is_err()
                || hashes.iter().any(|s| {
                    s.len() != 64
                        || !s
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
                || source.science_id != self.science_id()?
                || source.bytes == 0
                || source.bytes > self.observation.max_artifact_bytes
                || self.channel_case()?.presentation.field != "velocity"
                || self.observation.retained_times_s.is_empty()
                || !self.observation.metrics.is_empty()
                || !self.observation.probes.is_empty()
                || !self.observation.preview_times_s.is_empty()
                || !self.observation.checkpoint_times_s.is_empty()
                || self.observation.preview_may_drop
            {
                return Err(invalid(
                    "presentation requires exact source science, retained times and bounded supported fields",
                ));
            }
            let operations: Vec<_> = self.stages.iter().map(|s| &s.operation).collect();
            let render = matches!(
                operations.as_slice(),
                [StageOperation::Render, StageOperation::Bundle]
                    | [
                        StageOperation::Render,
                        StageOperation::Video,
                        StageOperation::Bundle
                    ]
            );
            let video = matches!(
                operations.as_slice(),
                [StageOperation::Video, StageOperation::Bundle]
            );
            let numerical = matches!(
                operations.as_slice(),
                [StageOperation::NumericalFilter, StageOperation::Bundle]
            );
            if (self.filter.is_some() && !numerical)
                || (self.filter.is_none() && self.frames.is_none() && !render)
                || (self.frames.is_some() && !video)
            {
                return Err(invalid(
                    "source-bound plans require their exact render, video or numerical-filter DAG",
                ));
            }
            if let Some(filter) = &self.filter
                && (!filter.time_s.is_finite()
                    || self.observation.retained_times_s != vec![filter.time_s])
            {
                return Err(invalid(
                    "numerical filter requires one exact retained physical time",
                ));
            }
            if let Some(frames) = &self.frames {
                let hashes = [
                    &frames.plan_digest,
                    &frames.execution_binding_digest,
                    &frames.authorization_digest,
                    &frames.sequence_sha256,
                ];
                if uuid::Uuid::parse_str(&frames.job_id).is_err()
                    || frames.bytes == 0
                    || frames.bytes > self.observation.max_artifact_bytes
                    || hashes.iter().any(|h| {
                        h.len() != 64
                            || !h
                                .bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    })
                {
                    return Err(invalid(
                        "exact bounded frame-source execution identities required",
                    ));
                }
            }
            for (index, stage) in self.stages.iter().enumerate() {
                let dependencies = if index == 0 {
                    vec![]
                } else {
                    vec![self.stages[index - 1].id.clone()]
                };
                if stage.dependencies != dependencies
                    || stage.selection.as_ref().is_some_and(|s| {
                        if matches!(stage.operation, StageOperation::NumericalFilter) {
                            s.role != Role::Compute
                                || s.backend != "hip"
                                || s.backend_uuid.as_deref().is_none_or(|id| !token(id))
                        } else {
                            s.backend_uuid.is_some()
                                || s.backend
                                    != if s.role == Role::Render {
                                        "egl"
                                    } else {
                                        "vaapi"
                                    }
                        }
                    })
                {
                    return Err(invalid(
                        "presentation requires exact sequential dependencies and EGL/VAAPI roles",
                    ));
                }
            }
        }
        if self.stages.is_empty() || self.stages.len() > 32 {
            return Err(invalid("bounded nonempty DAG required"));
        }
        let mut seen = BTreeSet::new();
        let mut operations = BTreeSet::new();
        for stage in &self.stages {
            if stage.operation == StageOperation::AtmosphericTransport
                && self.atmospheric_transport.is_none()
            {
                return Err(invalid(
                    "atmospheric transport operation requires source-bound version-15 recipe",
                ));
            }
            if stage.operation == StageOperation::SpectralReference
                && self.spectral.is_none()
                && self.cad_transport.is_none()
            {
                return Err(invalid(
                    "spectral operation requires independent version-13 recipe",
                ));
            }
            if stage.operation == StageOperation::AtmosphericReference && self.atmosphere.is_none()
            {
                return Err(invalid(
                    "atmospheric operation requires independent version-14 recipe",
                ));
            }
            if stage.operation == StageOperation::FreezingReference
                && self.freezing.is_none()
                && self.retained_cooling.is_none()
            {
                return Err(invalid(
                    "freezing operation requires independent version-12 recipe",
                ));
            }
            if matches!(stage.operation, StageOperation::FemImported) && self.imported_fem.is_none()
            {
                return Err(invalid(
                    "imported FEM requires a source-bound version-8 recipe",
                ));
            }
            if matches!(stage.operation, StageOperation::CadMesh) && self.cad_source.is_none() {
                return Err(invalid(
                    "imported CAD mesh requires a source-bound version-7 recipe",
                ));
            }
            if matches!(stage.operation, StageOperation::ThermalReference)
                && self.thermal.is_none()
                && self.thermal_contact.is_none()
            {
                return Err(invalid(
                    "thermal operation requires an independent version-6 recipe",
                ));
            }
            if matches!(stage.operation, StageOperation::WettingReference) && self.wetting.is_none()
            {
                return Err(invalid(
                    "wetting operation requires an independent version-9 recipe",
                ));
            }
            if matches!(stage.operation, StageOperation::ContactReference)
                && self.contact.is_none()
                && self.thermal_contact.is_none()
            {
                return Err(invalid(
                    "contact operation requires an independent version-10 recipe",
                ));
            }
            if matches!(stage.operation, StageOperation::FemReference) && self.fem.is_none() {
                return Err(invalid("FEM operation requires a version-5 recipe"));
            }
            if stage.operation == StageOperation::ThermalProjection
                && self.thermal_contact.is_none()
            {
                return Err(invalid(
                    "temperature projection stage requires exact version-11 coupling",
                ));
            }
            if matches!(stage.operation, StageOperation::NumericalFilter) && self.filter.is_none() {
                return Err(invalid(
                    "numerical compute stage requires a source-bound version-4 filter specification",
                ));
            }
            let op_name = serde_json::to_string(&stage.operation)?;
            if !operations.insert(op_name)
                && !(self.thermal_contact.is_some()
                    && stage.operation == StageOperation::ThermalReference)
            {
                return Err(invalid(
                    "one stage per fixed-output adapter operation required",
                ));
            }
            if !token(&stage.id)
                || seen.contains(&stage.id)
                || stage.dependencies.iter().any(|d| !seen.contains(d))
            {
                return Err(invalid(
                    "DAG requires unique IDs and topologically ordered dependencies",
                ));
            }
            if stage.ram_bytes == 0 {
                return Err(invalid("explicit RAM estimate required"));
            }
            if matches!(stage.operation, StageOperation::ChannelReference)
                && self.channel_case()?.applicability.formulation != "steady_incompressible_channel"
            {
                return Err(invalid("analytical adapter formulation mismatch"));
            }
            if matches!(stage.operation, StageOperation::Openlb)
                && (!self.channel_case()?.geometry.synthetic
                    || self.channel_case()?.applicability.formulation != "periodic_forced_channel")
            {
                return Err(invalid(
                    "OpenLB adapter supports only the synthetic periodic channel",
                ));
            }
            if matches!(stage.operation, StageOperation::CadInspect)
                && (self
                    .channel_case()?
                    .geometry
                    .sha256
                    .as_ref()
                    .is_none_or(|s| {
                        s.len() != 64
                            || !s
                                .bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    })
                    || self.channel_case()?.geometry.source.is_empty()
                    || std::path::Path::new(&self.channel_case()?.geometry.source)
                        .components()
                        .any(|c| !matches!(c, std::path::Component::Normal(_))))
            {
                return Err(invalid(
                    "CAD inspection requires a scoped relative source and lowercase SHA-256 approval",
                ));
            }
            let expected_role = match stage.operation {
                StageOperation::Openlb if stage.gpu != GpuRequirement::CpuOnly => {
                    Some(Role::Compute)
                }
                StageOperation::Render => Some(Role::Render),
                StageOperation::Video => Some(Role::Media),
                StageOperation::NumericalFilter => Some(Role::Compute),
                _ => None,
            };
            if let Some(role) = expected_role {
                if stage.gpu != GpuRequirement::Required
                    || stage.selection.as_ref().map(|s| s.role) != Some(role)
                {
                    return Err(invalid(
                        "native B1 operations require explicit role/device selection; no fallback",
                    ));
                }
            } else if stage.gpu != GpuRequirement::CpuOnly
                || stage.selection.is_some()
                || stage.vram_bytes != 0
            {
                return Err(invalid("CPU operation cannot claim GPU execution"));
            }
            seen.insert(stage.id.clone());
        }
        if let Some(index) = self
            .stages
            .iter()
            .position(|s| matches!(s.operation, StageOperation::Bundle))
            && index != self.stages.len() - 1
        {
            return Err(invalid("bundle must be the final stage"));
        }
        if !self.transfers.is_empty() && self.thermal_contact.is_none() {
            return Err(Error::Unqualified(
                "coupled transfers require model-specific conservation qualification".into(),
            ));
        }
        if self
            .case
            .as_ref()
            .is_some_and(|c| self.observation.max_artifact_bytes < u64::from(c.resolution) * 128)
            || self.observation.max_artifact_bytes > 1_000_000_000_000
            || self.observation.scientific_congestion != "fail"
        {
            return Err(invalid("scientific output budget/congestion policy"));
        }
        if self.observation.probes.len() > 256
            || self
                .observation
                .probes
                .iter()
                .flatten()
                .any(|v| !v.is_finite())
        {
            return Err(invalid("bounded finite probes required"));
        }
        for times in [
            &self.observation.retained_times_s,
            &self.observation.checkpoint_times_s,
            &self.observation.preview_times_s,
        ] {
            if times.len() > 1024
                || times.iter().any(|t| {
                    !t.is_finite()
                        || *t < 0.
                        || self
                            .thermal
                            .as_ref()
                            .map(|s| s.duration_s)
                            .or_else(|| {
                                self.thermal_contact.as_ref().map(|s| {
                                    s.thermal.iter().map(|t| t.duration_s).fold(0., f64::max)
                                })
                            })
                            .or_else(|| {
                                self.wetting
                                    .as_ref()
                                    .map(|s| s.steps as f64 * s.physical_step_s())
                            })
                            .or_else(|| self.case.as_ref().map(|c| c.max_time_s))
                            .or_else(|| {
                                self.freezing
                                    .as_ref()
                                    .and_then(|s| s.scale().ok())
                                    .map(|s| s.duration_s)
                            })
                            .or_else(|| self.retained_cooling.as_ref().map(|s| s.duration_s()))
                            .is_none_or(|end| *t > end)
                })
                || times.windows(2).any(|p| p[0] >= p[1])
            {
                return Err(invalid("ordered, bounded observation times required"));
            }
        }
        if !self.observation.checkpoint_times_s.is_empty() {
            return Err(Error::Unqualified("no verified checkpoint adapter".into()));
        }
        if self
            .stages
            .iter()
            .any(|s| matches!(s.operation, StageOperation::Openlb))
        {
            self.validate_openlb_lattice()?;
        }
        crate::estimates::minimum(self)?.validate(self)?;
        Ok(())
    }
    fn validate_openlb_lattice(&self) -> Result<()> {
        let c = self.channel_case()?;
        let height = c.channel_height.si("length")?;
        let dx = height / f64::from(c.resolution);
        let viscosity = c.kinematic_viscosity.si("kinematic_viscosity")?;
        let velocity =
            c.acceleration.si("acceleration")?.abs() * height * height / (8. * viscosity);
        // Preserve the order in OpenLB's pinned converter constructor, including
        // Float64 tau subtraction, rather than replacing it with an ideal 0.1.
        let dt = ((0.8f64 - 0.5) / 3.) * (dx * dx) / viscosity;
        let mach = 3f64.sqrt() * (velocity / (dx / dt));
        if velocity == 0. || !mach.is_finite() || mach > 0.1 {
            return Err(invalid(
                "fixed BGK reference requires nonzero drive and lattice Mach <= 0.1; parameters preserved",
            ));
        }
        let nx = c.length.si("length")? / dx;
        if !nx.is_finite() || nx < 1. || (nx - nx.round()).abs() > 1e-8 {
            return Err(invalid(
                "integral periodic lattice extent required; geometry cannot be silently rounded",
            ));
        }
        // OpenLB 1.9 unitConverter.h getLatticeTime uses size_t(t/dt+0.5).
        let lattice_time = |time: f64| -> Result<u64> {
            let step = (time / dt + 0.5).floor();
            if !dt.is_finite()
                || dt <= 0.
                || !step.is_finite()
                || step < 0.
                || step >= u64::MAX as f64
            {
                return Err(invalid("lattice time conversion overflow"));
            }
            Ok(step as u64)
        };
        let end = lattice_time(c.max_time_s)?;
        if end == 0 || self.observation.retained_times_s.last() != Some(&c.max_time_s) {
            return Err(invalid(
                "native duration must reach a lattice step and final scientific state must be retained",
            ));
        }
        let mut prior = None;
        for time in &self.observation.retained_times_s {
            let step = lattice_time(*time)?;
            if step > end || prior == Some(step) {
                return Err(invalid(
                    "retained times collapse to one lattice step or exceed duration; reapprove explicitly",
                ));
            }
            prior = Some(step);
        }
        Ok(())
    }
}
pub fn fleetix_digest() -> String {
    format!(
        "{:x}",
        Sha256::digest(fleetix::gpu::CONTRACT_JSON.as_bytes())
    )
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactManifest {
    pub schema_version: u32,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub format: String,
    pub provenance: String,
    pub units: Option<String>,
    pub time_s: Option<f64>,
    pub association: Option<String>,
}
pub fn default_artifact_limit() -> u32 {
    20
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPage {
    pub items: Vec<ArtifactManifest>,
    pub total: u64,
    pub next_after: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidationReport {
    pub process: String,
    pub convergence: String,
    pub numerical_verification: String,
    pub physical_validation: String,
    pub limitations: Vec<String>,
    pub moisture_risk: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostExecutionProfile {
    pub schema_version: u32,
    pub policy: String,
    pub allowed_input_root: String,
    pub max_ram_bytes: u64,
    pub max_disk_bytes: u64,
    pub threads: u32,
    pub timeout_seconds: u32,
    pub native_runtime: Option<String>,
    pub service_mode: String,
}

crate::json::tagged_enum! {
"operation";
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    PrepareStudy {
        request: Box<crate::study::StudyRequest>,
    },
    SubmitStudy {
        request: Box<crate::study::StudyRequest>,
        idempotency_key: String,
    },
    StudyStatus {
        study_id: String,
    },
    Doctor {},
    BackendList {},
    Validate {
        case: Box<CaseSpec>,
    },
    Plan {
        case: Box<CaseSpec>,
    },
    PlanOpenlbReference {
        case: Box<CaseSpec>,
    },
    PlanCadInspection {
        case: Box<CaseSpec>,
        max_artifact_bytes: u64,
    },
    PlanCadMesh {
        request: Box<crate::cad_source::CadMeshRequest>,
    },
    PlanCadVariant {
        request: Box<crate::cad_variant::CadVariantRequest>,
    },
    PlanCadSpectralTransport {
        request: Box<crate::cad_transport::CadSpectralTransportRequest>,
    },
    CadOpticalResults {
        request: Box<crate::cad_transport_results::CadOpticalResultsRequest>,
    },
    PrepareCadSpectralScene {
        request: Box<crate::cad_spectral::CadSpectralSceneRequest>,
    },
    PlanFemImported {
        request: Box<crate::fem_imported::ImportedFemRequest>,
    },
    PlanFemReference {
        spec: Box<crate::fem::FemReferenceSpec>,
    },
    PlanThermalReference {
        spec: Box<crate::thermal::ThermalReferenceSpec>,
    },
    ValidateSnowReference {
        spec: Box<crate::snow::SnowReferenceSpec>,
    },
    ValidateSnowOpenings {
        spec: Box<crate::snow_openings::SnowOpeningRequest>,
    },
    ValidateSpectralReference {
        spec: Box<crate::radiation::SpectralReferenceSpec>,
    },
    PlanSpectralReference {
        spec: Box<crate::radiation::SpectralReferenceSpec>,
    },
    ValidateAtmosphericReference {
        spec: Box<crate::atmosphere::AtmosphericReferenceSpec>,
    },
    PlanAtmosphericReference {
        spec: Box<crate::atmosphere::AtmosphericReferenceSpec>,
    },
    ValidateSpectralReflectionReference {
        spec: Box<crate::radiation::SpectralReflectionSpec>,
    },
    PlanSnowReference {
        spec: Box<crate::snow::SnowReferenceSpec>,
    },
    PlanWettingReference {
        spec: Box<crate::wetting::WettingReferenceSpec>,
    },
    PlanContactReference {
        spec: Box<crate::contact::ContactReferenceSpec>,
    },
    PlanThermalContact {
        spec: Box<crate::thermal_contact::ThermalContactSpec>,
    },
    ValidateFreezingReference {
        spec: Box<crate::freezing::FreezingReferenceSpec>,
    },
    PlanFreezingReference {
        spec: Box<crate::freezing::FreezingReferenceSpec>,
    },
    PlanRetainedCooling {
        request: Box<crate::cooling_execution::CoolingExecutionRequest>,
    },
    ResultsSampleRetainedCooling {
        request: Box<crate::cooling_results::CoolingSampleRequest>,
    },
    PlanB1 {
        case: Box<CaseSpec>,
        selections: B1Selections,
    },
    PlanPresentation {
        request: Box<PresentationRequest>,
    },
    ValidateColdRestart {
        case: Box<crate::recipes::ColdRestartSpec>,
    },
    PlanVideo {
        request: Box<VideoRequest>,
    },
    PlanFilter {
        request: Box<FilterRequest>,
    },
    Submit {
        plan: Box<ExecutionPlan>,
        approved_digest: String,
        idempotency_key: String,
    },
    Status {
        job_id: String,
    },
    Logs {
        job_id: String,
        after: u64,
        limit: u32,
    },
    Cancel {
        job_id: String,
    },
    Artifacts {
        job_id: String,
        #[serde(default)]
        after: Option<String>,
        #[serde(default = "default_artifact_limit")]
        limit: u32,
    },
    Describe {
        job_id: String,
    },
    ResultsSample {
        request: Box<crate::results::SampleRequest>,
    },
    ResultsCompare {
        request: Box<crate::results::CompareRequest>,
    },
    ResultsSampleThermal {
        request: Box<crate::thermal_results::ThermalSampleRequest>,
    },
    ResultsCompareThermal {
        request: Box<crate::thermal_results::ThermalCompareRequest>,
    },
    ResultsSampleFreezing {
        request: Box<crate::freezing_results::FreezingSampleRequest>,
    },
    ResultsCompareFreezing {
        request: Box<crate::freezing_results::FreezingCompareRequest>,
    },
    ResultsMoisture {
        request: Box<crate::moisture_results::NativeMoistureRequest>,
    },
    ResultsTransferTemperature {
        request: Box<crate::thermal_transfer::ThermalProjectionRequest>,
    },
    ResultsRetainWetting {
        request: Box<crate::wetting_retention::WettingRetentionRequest>,
    },
    ResultsPrepareRetainedCooling {
        request: Box<crate::retained_cooling::RetainedCoolingRequest>,
    },
    ResultsTransferAtmosphere {
        request: Box<crate::atmosphere_transfer::AtmosphericTransferRequest>,
    },
    PlanAtmosphericTransport {
        request: Box<crate::atmosphere_transfer::AtmosphericTransferRequest>,
    },
    QualificationReport {
        job_id: String,
    },
    CadRegions {
        job_id: String,
    },
}
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub request: Operation,
}
pub fn schemas() -> serde_json::Value {
    let mut schemas = serde_json::json!({"CaseSpec": schemars::schema_for!(CaseSpec), "PhysicsApplicability": schemars::schema_for!(PhysicsApplicability),
        "ExecutionPlan": schemars::schema_for!(ExecutionPlan), "TransferSpec": schemars::schema_for!(TransferSpec),
        "HostExecutionProfile": schemars::schema_for!(HostExecutionProfile), "GpuSelection": schemars::schema_for!(GpuSelection),
        "ObservationPlan": schemars::schema_for!(ObservationPlan), "ArtifactManifest": schemars::schema_for!(ArtifactManifest),
        "B1Selections": schemars::schema_for!(B1Selections),
        "ArtifactPage": schemars::schema_for!(ArtifactPage),
        "ExecutionBinding": schemars::schema_for!(crate::execution::ExecutionBinding),
        "HostAuthority": schemars::schema_for!(crate::authority::HostAuthority),
        "ExecutionAuthorization": schemars::schema_for!(crate::authority::ExecutionAuthorization),
        "FieldSnapshot": schemars::schema_for!(crate::fields::FieldSnapshot),
        "RetainedSource": schemars::schema_for!(RetainedSource),
        "PresentationRequest": schemars::schema_for!(PresentationRequest),
        "FrameSource": schemars::schema_for!(FrameSource),
        "VideoRequest": schemars::schema_for!(VideoRequest),
        "FilterRequest": schemars::schema_for!(FilterRequest),
        "JobEvidenceReport": schemars::schema_for!(crate::qualification::JobEvidenceReport),
        "SampleRequest": schemars::schema_for!(crate::results::SampleRequest),
        "SampleReport": schemars::schema_for!(crate::results::SampleReport),
        "CompareRequest": schemars::schema_for!(crate::results::CompareRequest),
        "CompareReport": schemars::schema_for!(crate::results::CompareReport),
        "RegionReport": schemars::schema_for!(crate::cad::RegionReport),
        "CadSource": schemars::schema_for!(crate::cad_source::CadSource),
        "CadMeshRequest": schemars::schema_for!(crate::cad_source::CadMeshRequest),
        "CadSpectralSceneRequest": schemars::schema_for!(crate::cad_spectral::CadSpectralSceneRequest),
        "PreparedCadSpectralScene": schemars::schema_for!(crate::cad_spectral::PreparedCadSpectralScene),
        "ImportedFemRequest": schemars::schema_for!(crate::fem_imported::ImportedFemRequest),
        "ImportedFemSpec": schemars::schema_for!(crate::fem_imported::ImportedFemSpec),
        "FemReferenceSpec": schemars::schema_for!(crate::fem::FemReferenceSpec),
        "WettingReferenceSpec": schemars::schema_for!(crate::wetting::WettingReferenceSpec),
        "ThermalReferenceSpec": schemars::schema_for!(crate::thermal::ThermalReferenceSpec),
        "ConservativeTransfer": schemars::schema_for!(crate::transfers::ConservativeTransfer),
        "TransferReceipt": schemars::schema_for!(crate::transfers::TransferReceipt),
        "ThermalMaterial": schemars::schema_for!(crate::materials::ThermalMaterial),
        "ColdRestartSpec": schemars::schema_for!(crate::recipes::ColdRestartSpec),
        "ValidationReport": schemars::schema_for!(ValidationReport), "WorkerRequest": schemars::schema_for!(WorkerRequest)});
    // Keep individual expansions below the macro recursion bound as contracts grow.
    schemas["StudyRequest"] = serde_json::json!(schemars::schema_for!(crate::study::StudyRequest));
    schemas["PreparedStudy"] =
        serde_json::json!(schemars::schema_for!(crate::study::PreparedStudy));
    schemas["AtmosphericReferenceSpec"] = serde_json::json!(schemars::schema_for!(
        crate::atmosphere::AtmosphericReferenceSpec
    ));
    schemas["PreparedAtmosphericReference"] = serde_json::json!(schemars::schema_for!(
        crate::atmosphere::PreparedAtmosphericReference
    ));
    schemas["AtmosphericTransportSpec"] = serde_json::json!(schemars::schema_for!(
        crate::atmospheric_transport::AtmosphericTransportSpec
    ));
    schemas["CadSpectralTransportRequest"] = serde_json::json!(schemars::schema_for!(
        crate::cad_transport::CadSpectralTransportRequest
    ));
    schemas["CadSpectralTransportSpec"] = serde_json::json!(schemars::schema_for!(
        crate::cad_transport::CadSpectralTransportSpec
    ));
    schemas["CadOpticalResultsRequest"] = serde_json::json!(schemars::schema_for!(
        crate::cad_transport_results::CadOpticalResultsRequest
    ));
    schemas["CadOpticalResultsReport"] = serde_json::json!(schemars::schema_for!(
        crate::cad_transport_results::CadOpticalResultsReport
    ));
    schemas["SpectralReflectionSpec"] = serde_json::json!(schemars::schema_for!(
        crate::radiation::SpectralReflectionSpec
    ));
    schemas["PreparedSpectralReflection"] = serde_json::json!(schemars::schema_for!(
        crate::radiation::PreparedSpectralReflection
    ));
    schemas["SpectralReferenceSpec"] = serde_json::json!(schemars::schema_for!(
        crate::radiation::SpectralReferenceSpec
    ));
    schemas["PreparedSpectralReference"] = serde_json::json!(schemars::schema_for!(
        crate::radiation::PreparedSpectralReference
    ));
    schemas["WettingRetentionRequest"] = serde_json::json!(schemars::schema_for!(
        crate::wetting_retention::WettingRetentionRequest
    ));
    schemas["RetainedWettingReport"] = serde_json::json!(schemars::schema_for!(
        crate::wetting_retention::RetainedWettingReport
    ));
    schemas["RetainedCoolingRequest"] = serde_json::json!(schemars::schema_for!(
        crate::retained_cooling::RetainedCoolingRequest
    ));
    schemas["PreparedRetainedCooling"] = serde_json::json!(schemars::schema_for!(
        crate::retained_cooling::PreparedRetainedCooling
    ));
    schemas["CoolingExecutionRequest"] = serde_json::json!(schemars::schema_for!(
        crate::cooling_execution::CoolingExecutionRequest
    ));
    schemas["CoolingExecutionSpec"] = serde_json::json!(schemars::schema_for!(
        crate::cooling_execution::CoolingExecutionSpec
    ));
    schemas["CoolingSampleRequest"] = serde_json::json!(schemars::schema_for!(
        crate::cooling_results::CoolingSampleRequest
    ));
    schemas["AtmosphericTransferRequest"] = serde_json::json!(schemars::schema_for!(
        crate::atmosphere_transfer::AtmosphericTransferRequest
    ));
    schemas["AtmosphericTransferReport"] = serde_json::json!(schemars::schema_for!(
        crate::atmosphere_transfer::AtmosphericTransferReport
    ));
    schemas["SnowReferenceSpec"] =
        serde_json::json!(schemars::schema_for!(crate::snow::SnowReferenceSpec));
    schemas["PreparedSnowBoundary"] =
        serde_json::json!(schemars::schema_for!(crate::snow::PreparedSnowBoundary));
    schemas["SnowOpeningRequest"] = serde_json::json!(schemars::schema_for!(
        crate::snow_openings::SnowOpeningRequest
    ));
    schemas["PreparedSnowOpenings"] = serde_json::json!(schemars::schema_for!(
        crate::snow_openings::PreparedSnowOpenings
    ));
    schemas["FreezingSampleRequest"] = serde_json::json!(schemars::schema_for!(
        crate::freezing_results::FreezingSampleRequest
    ));
    schemas["FreezingSampleReport"] = serde_json::json!(schemars::schema_for!(
        crate::freezing_results::FreezingSampleReport
    ));
    schemas["FreezingCompareRequest"] = serde_json::json!(schemars::schema_for!(
        crate::freezing_results::FreezingCompareRequest
    ));
    schemas["FreezingCompareReport"] = serde_json::json!(schemars::schema_for!(
        crate::freezing_results::FreezingCompareReport
    ));
    schemas["ThermalSampleRequest"] = serde_json::json!(schemars::schema_for!(
        crate::thermal_results::ThermalSampleRequest
    ));
    schemas["ThermalSampleReport"] = serde_json::json!(schemars::schema_for!(
        crate::thermal_results::ThermalSampleReport
    ));
    schemas["ThermalCompareRequest"] = serde_json::json!(schemars::schema_for!(
        crate::thermal_results::ThermalCompareRequest
    ));
    schemas["ThermalCompareReport"] = serde_json::json!(schemars::schema_for!(
        crate::thermal_results::ThermalCompareReport
    ));
    schemas["NativeMoistureRequest"] = serde_json::json!(schemars::schema_for!(
        crate::moisture_results::NativeMoistureRequest
    ));
    schemas["NativeMoistureReport"] = serde_json::json!(schemars::schema_for!(
        crate::moisture_results::NativeMoistureReport
    ));
    schemas["ContactReferenceSpec"] =
        serde_json::json!(schemars::schema_for!(crate::contact::ContactReferenceSpec));
    schemas["ThermalProjectionRequest"] = serde_json::json!(schemars::schema_for!(
        crate::thermal_transfer::ThermalProjectionRequest
    ));
    schemas["ThermalProjectionReport"] = serde_json::json!(schemars::schema_for!(
        crate::thermal_transfer::ThermalProjectionReport
    ));
    schemas["FreezingReferenceSpec"] = serde_json::json!(schemars::schema_for!(
        crate::freezing::FreezingReferenceSpec
    ));
    schemas["FreezingScale"] =
        serde_json::json!(schemars::schema_for!(crate::freezing::FreezingScale));
    schemas["CadVariantRequest"] =
        serde_json::json!(schemars::schema_for!(crate::cad_variant::CadVariantRequest));
    schemas["CadVariantSpec"] =
        serde_json::json!(schemars::schema_for!(crate::cad_variant::CadVariantSpec));
    schemas
}
