//! Sourced material/process data with estimate-only cost, manufacturability and
//! service-margin screens. Estimates are not quotes; missing data stays missing.
use crate::{
    Error, Result,
    contracts::{invalid, token},
    materials::PhysicalInput,
    science::Quantity,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Stated assumption, not a sourced value: allowance for offcuts, sprues,
/// support/powder loss and rejected parts on top of the bought material.
/// Every estimate echoes it in `assumptions`.
pub const MATERIAL_SCRAP_FACTOR: f64 = 1.1;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuantityRange {
    pub low: Quantity,
    pub high: Quantity,
}

impl QuantityRange {
    pub fn si(&self, dimension: &str) -> Result<(f64, f64)> {
        let (low, high) = (self.low.si(dimension)?, self.high.si(dimension)?);
        if low > high {
            return Err(invalid("range low exceeds high"));
        }
        Ok((low, high))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoneyRange {
    pub low: f64,
    pub high: f64,
    pub currency: String,
    pub basis: String,
    pub year: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub id: String,
    pub name: String,
    pub density: PhysicalInput<QuantityRange>,
    pub thermal_conductivity: PhysicalInput<QuantityRange>,
    pub specific_heat: PhysicalInput<QuantityRange>,
    pub youngs_modulus: PhysicalInput<QuantityRange>,
    pub poisson_ratio: PhysicalInput<QuantityRange>,
    pub yield_strength: PhysicalInput<QuantityRange>,
    pub tensile_strength: PhysicalInput<QuantityRange>,
    pub thermal_expansion: PhysicalInput<QuantityRange>,
    pub max_service_temperature: PhysicalInput<QuantityRange>,
    pub min_service_temperature: PhysicalInput<QuantityRange>,
    pub emissivity: PhysicalInput<QuantityRange>,
    pub solar_absorptance: PhysicalInput<QuantityRange>,
    pub compression_set: PhysicalInput<QuantityRange>,
    pub hardness_shore_a: PhysicalInput<QuantityRange>,
    pub uv_resistance: PhysicalInput<String>,
    pub material_cost: PhysicalInput<MoneyRange>,
}

impl Material {
    /// (field, dimension, value) for every numeric range property.
    pub fn quantities(&self) -> [(&'static str, &'static str, &PhysicalInput<QuantityRange>); 14] {
        [
            ("density", "density", &self.density),
            (
                "thermal_conductivity",
                "thermal_conductivity",
                &self.thermal_conductivity,
            ),
            ("specific_heat", "specific_heat", &self.specific_heat),
            ("youngs_modulus", "pressure", &self.youngs_modulus),
            ("poisson_ratio", "dimensionless", &self.poisson_ratio),
            ("yield_strength", "pressure", &self.yield_strength),
            ("tensile_strength", "pressure", &self.tensile_strength),
            (
                "thermal_expansion",
                "thermal_expansion",
                &self.thermal_expansion,
            ),
            (
                "max_service_temperature",
                "temperature",
                &self.max_service_temperature,
            ),
            (
                "min_service_temperature",
                "temperature",
                &self.min_service_temperature,
            ),
            ("emissivity", "dimensionless", &self.emissivity),
            (
                "solar_absorptance",
                "dimensionless",
                &self.solar_absorptance,
            ),
            ("compression_set", "dimensionless", &self.compression_set),
            ("hardness_shore_a", "dimensionless", &self.hardness_shore_a),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimeBasis {
    /// Subtractive: stock is the bounding box; machine time follows removed volume.
    RemovedVolume,
    /// Additive: machine time follows part volume.
    PartVolume,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Process {
    pub id: String,
    pub name: String,
    pub compatible_materials: Vec<String>,
    pub min_wall_thickness: PhysicalInput<QuantityRange>,
    pub max_envelope: PhysicalInput<[Quantity; 3]>,
    pub typical_tolerance: PhysicalInput<QuantityRange>,
    pub setup_cost: PhysicalInput<MoneyRange>,
    /// `null`: the process uses no part-specific tooling.
    pub tooling_cost: Option<PhysicalInput<MoneyRange>>,
    pub tool_life_parts: Option<PhysicalInput<QuantityRange>>,
    pub rate: PhysicalInput<MoneyRange>,
    pub rate_includes_material: bool,
    /// Volume processed per machine hour; only for `per_hour` rates.
    pub throughput: Option<PhysicalInput<QuantityRange>>,
    pub time_basis: Option<TimeBasis>,
    pub machine_time_model: String,
    pub lead_time: PhysicalInput<QuantityRange>,
    pub notes: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterialsFile {
    schema_version: u32,
    materials: Vec<Material>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessesFile {
    schema_version: u32,
    processes: Vec<Process>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Library {
    pub materials: Vec<Material>,
    pub processes: Vec<Process>,
}

fn context<T>(owner: &str, field: &str, result: Result<T>) -> Result<T> {
    result.map_err(|error| match error {
        Error::Invalid(message) => invalid(format!("{owner}.{field}: {message}")),
        other => other,
    })
}

fn check_range(input: &PhysicalInput<QuantityRange>, dimension: &str) -> Result<()> {
    input.validate()?;
    if let PhysicalInput::Known { value, .. } = input {
        let (low, _) = value.si(dimension)?;
        if (dimension == "temperature" && low <= 0.) || low < 0. {
            return Err(invalid("non-physical negative range"));
        }
    }
    Ok(())
}

fn check_money(input: &PhysicalInput<MoneyRange>, bases: &[&str]) -> Result<()> {
    input.validate()?;
    if let PhysicalInput::Known { value, .. } = input
        && (!value.low.is_finite()
            || !value.high.is_finite()
            || value.low < 0.
            || value.low > value.high
            || value.currency != "USD"
            || !bases.contains(&value.basis.as_str())
            || !(2000..=2100).contains(&value.year))
    {
        return Err(invalid(format!(
            "finite 0 <= low <= high USD with basis in {bases:?} and a price year required"
        )));
    }
    Ok(())
}

impl Library {
    pub fn builtin() -> Result<Self> {
        Self::load(
            include_str!("../data/materials.json"),
            include_str!("../data/processes.json"),
        )
    }

    pub fn load(materials_json: &str, processes_json: &str) -> Result<Self> {
        let materials: MaterialsFile = serde_json::from_str(materials_json)?;
        let processes: ProcessesFile = serde_json::from_str(processes_json)?;
        if materials.schema_version != 1 || processes.schema_version != 1 {
            return Err(invalid("material/process data schema_version 1 required"));
        }
        let library = Self {
            materials: materials.materials,
            processes: processes.processes,
        };
        library.validate()?;
        Ok(library)
    }

    fn validate(&self) -> Result<()> {
        let mut ids = BTreeSet::new();
        for id in self
            .materials
            .iter()
            .map(|m| &m.id)
            .chain(self.processes.iter().map(|p| &p.id))
        {
            if !token(id) || !ids.insert(id.as_str()) {
                return Err(invalid(format!("unique token id required: {id}")));
            }
        }
        for m in &self.materials {
            for (field, dimension, input) in m.quantities() {
                context(&m.id, field, check_range(input, dimension))?;
            }
            context(&m.id, "uv_resistance", m.uv_resistance.validate())?;
            context(
                &m.id,
                "material_cost",
                check_money(&m.material_cost, &["per_kg"]),
            )?;
        }
        for p in &self.processes {
            let owner = p.id.as_str();
            if p.compatible_materials.is_empty()
                || p.compatible_materials
                    .iter()
                    .any(|id| self.material(id).is_err())
            {
                return Err(invalid(format!(
                    "{owner}: compatible_materials must name known materials"
                )));
            }
            context(
                owner,
                "min_wall_thickness",
                check_range(&p.min_wall_thickness, "length"),
            )?;
            context(
                owner,
                "typical_tolerance",
                check_range(&p.typical_tolerance, "length"),
            )?;
            context(owner, "lead_time", check_range(&p.lead_time, "time"))?;
            context(owner, "max_envelope", p.max_envelope.validate())?;
            if let PhysicalInput::Known { value, .. } = &p.max_envelope {
                for q in value {
                    if context(owner, "max_envelope", q.si("length"))? <= 0. {
                        return Err(invalid(format!("{owner}.max_envelope: positive required")));
                    }
                }
            }
            context(
                owner,
                "setup_cost",
                check_money(&p.setup_cost, &["per_order"]),
            )?;
            context(
                owner,
                "rate",
                check_money(&p.rate, &["per_hour", "per_cm3", "per_part"]),
            )?;
            match (&p.tooling_cost, &p.tool_life_parts) {
                (None, None) => {}
                (Some(cost), Some(life)) => {
                    context(owner, "tooling_cost", check_money(cost, &["per_tool"]))?;
                    context(owner, "tool_life_parts", check_range(life, "dimensionless"))?;
                    if let PhysicalInput::Known { value, .. } = life
                        && value.si("dimensionless")?.0 < 1.
                    {
                        return Err(invalid(format!("{owner}.tool_life_parts: >= 1 required")));
                    }
                }
                _ => {
                    return Err(invalid(format!(
                        "{owner}: tooling_cost and tool_life_parts are both null or both present"
                    )));
                }
            }
            let hourly =
                matches!(&p.rate, PhysicalInput::Known { value, .. } if value.basis == "per_hour");
            match (&p.throughput, p.time_basis) {
                (Some(throughput), Some(_)) => {
                    context(owner, "throughput", check_range(throughput, "volume_rate"))?;
                    if let PhysicalInput::Known { value, .. } = throughput
                        && value.si("volume_rate")?.0 <= 0.
                    {
                        return Err(invalid(format!("{owner}.throughput: positive required")));
                    }
                }
                (None, None) if !hourly => {}
                _ => {
                    return Err(invalid(format!(
                        "{owner}: per_hour rates need throughput and time_basis; other bases neither"
                    )));
                }
            }
            if p.machine_time_model.trim().is_empty() || p.notes.trim().is_empty() {
                return Err(invalid(format!(
                    "{owner}: machine_time_model and notes required"
                )));
            }
        }
        Ok(())
    }

    pub fn material(&self, id: &str) -> Result<&Material> {
        self.materials
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| invalid(format!("unknown material {id}")))
    }

    pub fn process(&self, id: &str) -> Result<&Process> {
        self.processes
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| invalid(format!("unknown process {id}")))
    }

    /// Bounded listing; full records via `materials show`.
    pub fn summary(&self) -> serde_json::Value {
        let materials: Vec<_> = self
            .materials
            .iter()
            .map(|m| serde_json::json!({"id": m.id, "name": m.name}))
            .collect();
        let processes: Vec<_> = self
            .processes
            .iter()
            .map(|p| {
                let rate_basis = match &p.rate {
                    PhysicalInput::Known { value, .. } => value.basis.as_str(),
                    PhysicalInput::Missing { .. } => "missing",
                };
                serde_json::json!({"id": p.id, "name": p.name,
                    "compatible_materials": p.compatible_materials, "rate_basis": rate_basis})
            })
            .collect();
        serde_json::json!({"materials": materials, "processes": processes,
            "scope": "sourced typical datasheet/quote-guide ranges; estimates, not supplier quotes or qualified properties"})
    }
}

/// Geometry facts a design loop fills from CAD receipts (SI units).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeometrySummary {
    pub volume_m3: f64,
    pub bbox_m: [f64; 3],
    pub min_wall_m: Option<f64>,
    pub surface_area_m2: Option<f64>,
}

impl GeometrySummary {
    fn validate(&self) -> Result<f64> {
        let bbox = self.bbox_m.iter().product::<f64>();
        let positive = |v: f64| v.is_finite() && v > 0.;
        if !positive(self.volume_m3)
            || !self.bbox_m.iter().all(|&v| positive(v))
            || self.volume_m3 > bbox * (1. + 1e-9)
            || self.min_wall_m.is_some_and(|v| !positive(v))
            || self.surface_area_m2.is_some_and(|v| !positive(v))
        {
            return Err(invalid(
                "finite positive geometry with volume inside its bounding box required",
            ));
        }
        Ok(bbox)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CostTerm {
    pub name: String,
    pub low: Option<f64>,
    pub high: Option<f64>,
    pub basis: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CostEstimate {
    pub material_id: String,
    pub process_id: String,
    pub quantity: u32,
    pub currency: String,
    /// Per-unit totals; `None` whenever any needed input is missing.
    pub low: Option<f64>,
    pub high: Option<f64>,
    pub breakdown: Vec<CostTerm>,
    pub missing: Vec<String>,
    pub price_years: Vec<u16>,
    pub assumptions: Vec<String>,
}

#[derive(Default)]
struct Terms {
    breakdown: Vec<CostTerm>,
    missing: Vec<String>,
    years: BTreeSet<u16>,
}

impl Terms {
    fn money<'b>(
        &mut self,
        owner: &str,
        field: &str,
        input: &'b PhysicalInput<MoneyRange>,
    ) -> Option<&'b MoneyRange> {
        match input {
            PhysicalInput::Known { value, .. } => {
                self.years.insert(value.year);
                Some(value)
            }
            PhysicalInput::Missing { .. } => {
                self.missing.push(format!("{owner}.{field}"));
                None
            }
        }
    }
    fn range(
        &mut self,
        owner: &str,
        field: &str,
        dimension: &str,
        input: &PhysicalInput<QuantityRange>,
    ) -> Result<Option<(f64, f64)>> {
        match input {
            PhysicalInput::Known { value, .. } => Ok(Some(value.si(dimension)?)),
            PhysicalInput::Missing { .. } => {
                self.missing.push(format!("{owner}.{field}"));
                Ok(None)
            }
        }
    }
    fn push(&mut self, name: &str, value: Option<(f64, f64)>, basis: String) {
        self.breakdown.push(CostTerm {
            name: name.into(),
            low: value.map(|v| v.0),
            high: value.map(|v| v.1),
            basis,
        });
    }
}

pub fn unit_cost(
    library: &Library,
    geometry: &GeometrySummary,
    material_id: &str,
    process_id: &str,
    quantity: u32,
) -> Result<CostEstimate> {
    let bbox_m3 = geometry.validate()?;
    if !(1..=1_000_000).contains(&quantity) {
        return Err(invalid("quantity must be 1..=1000000"));
    }
    let material = library.material(material_id)?;
    let process = library.process(process_id)?;
    if !process
        .compatible_materials
        .iter()
        .any(|m| m == material_id)
    {
        return Err(invalid(format!(
            "{process_id} is not declared compatible with {material_id}"
        )));
    }
    let q = f64::from(quantity);
    let subtractive = process.time_basis == Some(TimeBasis::RemovedVolume);
    let mut t = Terms::default();

    if process.rate_includes_material {
        t.push(
            "material",
            Some((0., 0.)),
            "included in process rate".into(),
        );
    } else {
        let stock_m3 = if subtractive {
            bbox_m3
        } else {
            geometry.volume_m3
        };
        let density = t.range(material_id, "density", "density", &material.density)?;
        let cost = t.money(material_id, "material_cost", &material.material_cost);
        let value = density.zip(cost).map(|((dl, dh), c)| {
            let mass = stock_m3 * MATERIAL_SCRAP_FACTOR;
            (mass * dl * c.low, mass * dh * c.high)
        });
        let stock = if subtractive {
            "bounding-box stock"
        } else {
            "part volume"
        };
        t.push(
            "material",
            value,
            format!("{stock} x density x USD/kg x scrap factor {MATERIAL_SCRAP_FACTOR}"),
        );
    }

    let rate = t.money(process_id, "rate", &process.rate);
    let process_value = match rate {
        None => None,
        Some(r) => match r.basis.as_str() {
            // ponytail: per_part rates ignore part size inside the sourced
            // spread; upgrade path: per-geometry quote-guide regression.
            "per_part" => Some((r.low, r.high)),
            "per_cm3" => {
                let cm3 = geometry.volume_m3 * 1e6;
                Some((cm3 * r.low, cm3 * r.high))
            }
            // ponytail: machine time = processed volume / sourced volumetric
            // throughput; ignores feature count, finishing passes, nesting and
            // orientation. Upgrade path: toolpath/slicer time from CAM.
            _ => {
                let throughput = match process.throughput.as_ref() {
                    Some(input) => t.range(process_id, "throughput", "volume_rate", input)?,
                    None => None,
                };
                let volume_m3 = if subtractive {
                    bbox_m3 - geometry.volume_m3
                } else {
                    geometry.volume_m3
                };
                throughput.map(|(lo, hi)| {
                    // throughput is m3/s in SI; rates are per hour.
                    let hours = |rate_si: f64| volume_m3 / rate_si / 3600.;
                    (r.low * hours(hi), r.high * hours(lo))
                })
            }
        },
    };
    t.push(
        "process",
        process_value,
        rate.map_or("missing rate".into(), |r| {
            format!("{} ({})", r.basis, process.machine_time_model)
        }),
    );

    let setup = t
        .money(process_id, "setup_cost", &process.setup_cost)
        .map(|s| (s.low / q, s.high / q));
    t.push(
        "setup",
        setup,
        format!("per_order setup amortized over {quantity}"),
    );

    if let (Some(cost), Some(life)) = (&process.tooling_cost, &process.tool_life_parts) {
        let cost = t.money(process_id, "tooling_cost", cost);
        let life = t.range(process_id, "tool_life_parts", "dimensionless", life)?;
        let value = cost.zip(life).map(|(c, (life_lo, life_hi))| {
            let tools = |life: f64| (q / life.floor()).ceil();
            (c.low * tools(life_hi) / q, c.high * tools(life_lo) / q)
        });
        t.push(
            "tooling",
            value,
            format!("ceil({quantity} / tool life) tools amortized over {quantity}"),
        );
    }

    let total = |pick: fn(&CostTerm) -> Option<f64>| {
        t.missing
            .is_empty()
            .then(|| t.breakdown.iter().map(pick).sum::<Option<f64>>())
            .flatten()
    };
    let (low, high) = (total(|c| c.low), total(|c| c.high));
    Ok(CostEstimate {
        material_id: material_id.into(),
        process_id: process_id.into(),
        quantity,
        currency: "USD".into(),
        low,
        high,
        missing: t.missing,
        breakdown: t.breakdown,
        price_years: t.years.into_iter().collect(),
        assumptions: vec![
            format!("material scrap factor {MATERIAL_SCRAP_FACTOR} (assumption, not sourced)"),
            "estimate from typical published ranges; not a supplier quote; excludes finishing, inserts, assembly, inspection, shipping and tax".into(),
        ],
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    Compatibility,
    Envelope,
    MinWall,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Outside every sourced limit.
    Violation,
    /// Inside the sourced spread between optimistic and conservative limits.
    Marginal,
    /// Required data is missing; not a pass.
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Violation {
    pub check: Check,
    pub severity: Severity,
    pub detail: String,
}

pub fn manufacturability(
    library: &Library,
    geometry: &GeometrySummary,
    material_id: &str,
    process_id: &str,
) -> Result<Vec<Violation>> {
    geometry.validate()?;
    library.material(material_id)?;
    let process = library.process(process_id)?;
    let mut found = Vec::new();
    let mut add = |check, severity, detail: String| {
        found.push(Violation {
            check,
            severity,
            detail,
        })
    };
    if !process
        .compatible_materials
        .iter()
        .any(|m| m == material_id)
    {
        add(
            Check::Compatibility,
            Severity::Violation,
            format!("{process_id} does not process {material_id}"),
        );
    }
    match &process.max_envelope {
        PhysicalInput::Known { value, .. } => {
            let mut envelope = value
                .iter()
                .map(|q| q.si("length"))
                .collect::<Result<Vec<_>>>()?;
            let mut part = geometry.bbox_m.to_vec();
            envelope.sort_by(f64::total_cmp);
            part.sort_by(f64::total_cmp);
            if part.iter().zip(&envelope).any(|(p, e)| p > e) {
                add(
                    Check::Envelope,
                    Severity::Violation,
                    format!("sorted bbox {part:?} m exceeds sorted envelope {envelope:?} m"),
                );
            }
        }
        PhysicalInput::Missing { reason } => add(
            Check::Envelope,
            Severity::Unknown,
            format!("process envelope missing: {reason}"),
        ),
    }
    match (geometry.min_wall_m, &process.min_wall_thickness) {
        (None, _) => add(
            Check::MinWall,
            Severity::Unknown,
            "geometry min_wall_m not provided".into(),
        ),
        (_, PhysicalInput::Missing { reason }) => add(
            Check::MinWall,
            Severity::Unknown,
            format!("process min wall missing: {reason}"),
        ),
        (Some(wall), PhysicalInput::Known { value, .. }) => {
            let (low, high) = value.si("length")?;
            if wall < low {
                add(
                    Check::MinWall,
                    Severity::Violation,
                    format!("wall {wall} m below process minimum {low}..{high} m"),
                );
            } else if wall < high {
                add(
                    Check::MinWall,
                    Severity::Marginal,
                    format!("wall {wall} m inside process minimum spread {low}..{high} m"),
                );
            }
        }
    }
    Ok(found)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MarginCheck {
    MaxTemperature,
    MinTemperature,
    Stress,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Margin {
    pub check: MarginCheck,
    /// K for temperatures, Pa for stress.
    pub demand: f64,
    /// Conservative end of the sourced limit (stress: yield low / safety factor).
    pub capacity: Option<f64>,
    /// K headroom for temperatures; capacity/demand - 1 for stress.
    pub margin: Option<f64>,
    pub passes: Option<bool>,
    pub detail: String,
}

pub fn service_margins(
    library: &Library,
    material_id: &str,
    max_temp_k: f64,
    min_temp_k: f64,
    max_von_mises_pa: Option<f64>,
    safety_factor: f64,
) -> Result<Vec<Margin>> {
    let material = library.material(material_id)?;
    if !max_temp_k.is_finite()
        || !min_temp_k.is_finite()
        || min_temp_k <= 0.
        || min_temp_k > max_temp_k
        || !safety_factor.is_finite()
        || safety_factor < 1.
        || max_von_mises_pa.is_some_and(|s| !s.is_finite() || s < 0.)
    {
        return Err(invalid(
            "0 < min <= max absolute temperature, finite stress >= 0 and safety factor >= 1 required",
        ));
    }
    let known = |input: &PhysicalInput<QuantityRange>, dimension| match input {
        PhysicalInput::Known { value, .. } => value.si(dimension).map(Some),
        PhysicalInput::Missing { .. } => Ok(None),
    };
    let margin =
        |check, demand, capacity: Option<f64>, headroom: &dyn Fn(f64) -> f64, detail: &str| {
            let margin = capacity.map(headroom);
            Margin {
                check,
                demand,
                capacity,
                margin,
                passes: margin.map(|m| m >= 0.),
                detail: if capacity.is_some() {
                    detail.into()
                } else {
                    format!("{detail}; data missing, not a pass")
                },
            }
        };
    let max = known(&material.max_service_temperature, "temperature")?.map(|r| r.0);
    let min = known(&material.min_service_temperature, "temperature")?.map(|r| r.1);
    let mut margins = vec![
        margin(
            MarginCheck::MaxTemperature,
            max_temp_k,
            max,
            &|c| c - max_temp_k,
            "low end of max service temperature",
        ),
        margin(
            MarginCheck::MinTemperature,
            min_temp_k,
            min,
            &|c| min_temp_k - c,
            "high end of min service temperature",
        ),
    ];
    if let Some(stress) = max_von_mises_pa {
        let capacity = known(&material.yield_strength, "pressure")?.map(|r| r.0 / safety_factor);
        margins.push(margin(
            MarginCheck::Stress,
            stress,
            capacity,
            &|c| {
                if stress == 0. {
                    f64::INFINITY
                } else {
                    c / stress - 1.
                }
            },
            "low end of yield strength / safety factor",
        ));
    }
    Ok(margins)
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CostRequest {
    pub geometry: GeometrySummary,
    pub material_id: String,
    pub process_id: String,
    pub quantity: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CostReport {
    pub estimate: CostEstimate,
    pub manufacturability: Vec<Violation>,
}

pub fn evaluate(library: &Library, request: &CostRequest) -> Result<CostReport> {
    Ok(CostReport {
        manufacturability: manufacturability(
            library,
            &request.geometry,
            &request.material_id,
            &request.process_id,
        )?,
        estimate: unit_cost(
            library,
            &request.geometry,
            &request.material_id,
            &request.process_id,
            request.quantity,
        )?,
    })
}
