//! Cited outdoor design conditions (ETSI EN 300 019-1-4 climatic classes),
//! external film coefficients and surface finishes. Standards classes are
//! design severities, not site weather; missing data stays missing.
use crate::{
    Error, Result,
    contracts::{invalid, token},
    manufacturing::{Library, QuantityRange},
    materials::PhysicalInput,
    science::Quantity,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClimateClass {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub low_air_temperature: PhysicalInput<QuantityRange>,
    pub high_air_temperature: PhysicalInput<QuantityRange>,
    pub low_relative_humidity: PhysicalInput<QuantityRange>,
    pub high_relative_humidity: PhysicalInput<QuantityRange>,
    pub solar_irradiance: PhysicalInput<QuantityRange>,
    pub wind_speed: PhysicalInput<QuantityRange>,
    /// Film at the class's design wind speed, used by the cold point.
    pub wind_film: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Film {
    pub id: String,
    pub name: String,
    /// `null`: natural convection in still air.
    pub wind_speed: Option<Quantity>,
    pub coefficient: PhysicalInput<QuantityRange>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Finish {
    pub id: String,
    pub name: String,
    pub compatible_materials: Vec<String>,
    /// `missing` falls back to the material's own sourced value.
    pub solar_absorptance: PhysicalInput<QuantityRange>,
    pub emissivity: PhysicalInput<QuantityRange>,
}

/// A cited conservative electronics default (not product data), e.g. a
/// component temperature limit or an effective board conductivity.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ElectronicsDefault {
    pub id: String,
    pub name: String,
    /// `temperature`, `thermal_conductivity` or `dimensionless` (e.g. an emissivity).
    pub dimension: String,
    pub value: PhysicalInput<QuantityRange>,
    /// A condition the product must meet for this default to hold (e.g. a
    /// board construction); `design export` lists it for the winner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConditionsFile {
    schema_version: u32,
    classes: Vec<ClimateClass>,
    films: Vec<Film>,
    internal_films: Vec<Film>,
    electronics: Vec<ElectronicsDefault>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FinishesFile {
    schema_version: u32,
    transparent_materials: Vec<String>,
    finishes: Vec<Finish>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Point {
    /// High ambient, peak solar, low (still-air) film.
    Hot,
    /// Low ambient, no solar, high (design-wind) film.
    Cold,
}

/// An SI value with the citation trail it was derived from.
#[derive(Clone, Debug, Serialize)]
pub struct Sourced {
    pub value: f64,
    pub unit: &'static str,
    pub provenance: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesignPoint {
    pub ambient_temperature: Sourced,
    pub solar_irradiance: Sourced,
    pub film_coefficient: Sourced,
}

pub struct Conditions {
    pub classes: Vec<ClimateClass>,
    pub films: Vec<Film>,
    /// Natural convection on cavity surfaces inside closed enclosures.
    pub internal_films: Vec<Film>,
    /// Cited conservative electronics defaults (not product data).
    pub electronics: Vec<ElectronicsDefault>,
    pub finishes: Vec<Finish>,
    /// Glazing that transmits solar: no finish, no absorbed-solar fallback.
    pub transparent_materials: Vec<String>,
    library: Library,
}

fn context<T>(owner: &str, field: &str, result: Result<T>) -> Result<T> {
    result.map_err(|error| match error {
        Error::Invalid(message) => invalid(format!("{owner}.{field}: {message}")),
        other => other,
    })
}

/// Cited, non-synthetic range inside `[min, max]` (SI); returns it when known.
fn check(
    input: &PhysicalInput<QuantityRange>,
    dimension: &str,
    min: f64,
    max: f64,
) -> Result<Option<(f64, f64)>> {
    input.validate()?;
    let PhysicalInput::Known {
        value,
        provenance,
        synthetic,
    } = input
    else {
        return Ok(None);
    };
    if *synthetic || !provenance.contains("https://") || !provenance.contains("accessed") {
        return Err(invalid(
            "non-synthetic value citing a URL and access date required",
        ));
    }
    let (low, high) = value.si(dimension)?;
    if low < min || high > max {
        return Err(invalid(format!(
            "{dimension} range outside {min}..={max} (SI)"
        )));
    }
    Ok(Some((low, high)))
}

fn si_unit(dimension: &str) -> &'static str {
    match dimension {
        "temperature" => "K",
        "irradiance" => "W/m2",
        "heat_transfer_coefficient" => "W/(m2*K)",
        "thermal_conductivity" => "W/(m*K)",
        _ => "1",
    }
}

fn end(
    input: &PhysicalInput<QuantityRange>,
    dimension: &str,
    high: bool,
    what: &str,
) -> Result<Sourced> {
    match input {
        PhysicalInput::Known {
            value, provenance, ..
        } => {
            let (low, upper) = value.si(dimension)?;
            Ok(Sourced {
                value: if high { upper } else { low },
                unit: si_unit(dimension),
                provenance: format!(
                    "{what} ({} end): {provenance}",
                    if high { "high" } else { "low" }
                ),
            })
        }
        PhysicalInput::Missing { reason } => Err(invalid(format!("{what} is missing: {reason}"))),
    }
}

impl Conditions {
    pub fn builtin() -> Result<Self> {
        Self::load(
            include_str!("../data/conditions.json"),
            include_str!("../data/finishes.json"),
            Library::builtin()?,
        )
    }

    pub fn load(conditions_json: &str, finishes_json: &str, library: Library) -> Result<Self> {
        let conditions: ConditionsFile = serde_json::from_str(conditions_json)?;
        let finishes: FinishesFile = serde_json::from_str(finishes_json)?;
        if conditions.schema_version != 1 || finishes.schema_version != 1 {
            return Err(invalid(
                "conditions/finishes data schema_version 1 required",
            ));
        }
        let loaded = Self {
            classes: conditions.classes,
            films: conditions.films,
            internal_films: conditions.internal_films,
            electronics: conditions.electronics,
            finishes: finishes.finishes,
            transparent_materials: finishes.transparent_materials,
            library,
        };
        loaded.validate()?;
        Ok(loaded)
    }

    fn validate(&self) -> Result<()> {
        let mut ids = BTreeSet::new();
        let named = self
            .classes
            .iter()
            .map(|c| (&c.id, &c.name))
            .chain(self.films.iter().map(|f| (&f.id, &f.name)))
            .chain(self.internal_films.iter().map(|f| (&f.id, &f.name)))
            .chain(self.electronics.iter().map(|e| (&e.id, &e.name)))
            .chain(self.finishes.iter().map(|f| (&f.id, &f.name)));
        for (id, name) in named {
            if !token(id) || !ids.insert(id.as_str()) || name.trim().is_empty() {
                return Err(invalid(format!("unique token id and name required: {id}")));
            }
        }
        if self.films.iter().filter(|f| f.wind_speed.is_none()).count() != 1 {
            return Err(invalid(
                "exactly one still-air (null wind_speed) film required",
            ));
        }
        if self.internal_films.iter().any(|f| f.wind_speed.is_some()) {
            return Err(invalid(
                "internal (closed-cavity) films are natural convection: null wind_speed",
            ));
        }
        for f in self.films.iter().chain(&self.internal_films) {
            let id = f.id.as_str();
            context(
                id,
                "coefficient",
                check(&f.coefficient, "heat_transfer_coefficient", 0.1, 1e4),
            )?;
            if let Some(speed) = &f.wind_speed
                && context(id, "wind_speed", speed.si("velocity"))? <= 0.
            {
                return Err(invalid(format!("{id}.wind_speed: positive required")));
            }
        }
        for c in &self.classes {
            let id = c.id.as_str();
            let temperature =
                |field, input| context(id, field, check(input, "temperature", 150., 400.));
            let humidity = |field, input| context(id, field, check(input, "dimensionless", 0., 1.));
            let low_t = temperature("low_air_temperature", &c.low_air_temperature)?;
            let high_t = temperature("high_air_temperature", &c.high_air_temperature)?;
            let low_rh = humidity("low_relative_humidity", &c.low_relative_humidity)?;
            let high_rh = humidity("high_relative_humidity", &c.high_relative_humidity)?;
            context(
                id,
                "solar_irradiance",
                check(&c.solar_irradiance, "irradiance", 0., 2000.),
            )?;
            let wind = context(id, "wind_speed", check(&c.wind_speed, "velocity", 0., 150.))?;
            for (low, high, what) in [
                (low_t, high_t, "air temperature"),
                (low_rh, high_rh, "relative humidity"),
            ] {
                if let (Some(low), Some(high)) = (low, high)
                    && low.1 > high.0
                {
                    return Err(invalid(format!("{id}: low {what} exceeds high")));
                }
            }
            let film = self.film(&c.wind_film)?;
            let film_speed = film
                .wind_speed
                .as_ref()
                .map(|s| s.si("velocity"))
                .transpose()?;
            if c.scope.trim().is_empty()
                || film_speed.is_none()
                || wind.is_some_and(|w| Some(w.1) != film_speed)
            {
                return Err(invalid(format!(
                    "{id}: scope and a wind_film at the class design wind speed required"
                )));
            }
        }
        for e in &self.electronics {
            let (min, max) = match e.dimension.as_str() {
                "temperature" => (150., 500.),
                "thermal_conductivity" => (0.01, 1e3),
                "dimensionless" => (f64::MIN_POSITIVE, 1.),
                _ => {
                    return Err(invalid(format!(
                        "{}: dimension temperature, thermal_conductivity or dimensionless required",
                        e.id
                    )));
                }
            };
            context(&e.id, "value", check(&e.value, &e.dimension, min, max))?;
            if e.requirement
                .as_ref()
                .is_some_and(|r| r.trim().is_empty() || r.len() > 1024)
            {
                return Err(invalid(format!(
                    "{}: requirement must be bounded nonempty text",
                    e.id
                )));
            }
        }
        for id in &self.transparent_materials {
            self.library.material(id)?;
        }
        for f in &self.finishes {
            let id = f.id.as_str();
            if f.compatible_materials.is_empty()
                || f.compatible_materials.iter().any(|m| {
                    self.library.material(m).is_err() || self.transparent_materials.contains(m)
                })
            {
                return Err(invalid(format!(
                    "{id}: compatible_materials must name known opaque materials"
                )));
            }
            for (field, input) in [
                ("solar_absorptance", &f.solar_absorptance),
                ("emissivity", &f.emissivity),
            ] {
                context(id, field, check(input, "dimensionless", 0., 1.))?;
            }
        }
        Ok(())
    }

    pub fn class(&self, id: &str) -> Result<&ClimateClass> {
        self.classes
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| invalid(format!("unknown condition class {id}")))
    }

    pub fn film(&self, id: &str) -> Result<&Film> {
        self.films
            .iter()
            .chain(&self.internal_films)
            .find(|f| f.id == id)
            .ok_or_else(|| invalid(format!("unknown film {id}")))
    }

    pub fn finish(&self, id: &str) -> Result<&Finish> {
        self.finishes
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| invalid(format!("unknown finish {id}")))
    }

    /// Worst-case steady design points. Hot: high air temperature, peak solar
    /// and the low still-air film; cold: low air temperature, no solar and the
    /// high design-wind film.
    // ponytail: conservative constant h; upgrade to ΔT/wind-dependent correlation iteration
    pub fn design_point(&self, class: &str, point: Point) -> Result<DesignPoint> {
        let c = self.class(class)?;
        let label = |what: &str| format!("{class} {point:?} point {what}");
        Ok(match point {
            Point::Hot => {
                let still = self
                    .films
                    .iter()
                    .find(|f| f.wind_speed.is_none())
                    .ok_or_else(|| invalid("no still-air film"))?;
                DesignPoint {
                    ambient_temperature: end(
                        &c.high_air_temperature,
                        "temperature",
                        true,
                        &label("high_air_temperature"),
                    )?,
                    solar_irradiance: end(
                        &c.solar_irradiance,
                        "irradiance",
                        true,
                        &label("solar_irradiance"),
                    )?,
                    film_coefficient: end(
                        &still.coefficient,
                        "heat_transfer_coefficient",
                        false,
                        &label(&format!("film {}", still.id)),
                    )?,
                }
            }
            Point::Cold => DesignPoint {
                ambient_temperature: end(
                    &c.low_air_temperature,
                    "temperature",
                    false,
                    &label("low_air_temperature"),
                )?,
                solar_irradiance: Sourced {
                    value: 0.,
                    unit: "W/m2",
                    provenance: label(
                        "solar_irradiance: none (clear night); sky radiative cooling below air temperature is not represented",
                    ),
                },
                film_coefficient: end(
                    &self.film(&c.wind_film)?.coefficient,
                    "heat_transfer_coefficient",
                    true,
                    &label(&format!("film {}", c.wind_film)),
                )?,
            },
        })
    }

    /// (solar absorptance, emissivity) of an opaque material under a finish:
    /// a known finish value overrides the material's, a missing one falls
    /// back to it. Transparent glazing is refused: its solar load is
    /// transmittance, not surface absorption.
    pub fn surface_optics(
        &self,
        material_id: &str,
        finish_id: &str,
    ) -> Result<(PhysicalInput<QuantityRange>, PhysicalInput<QuantityRange>)> {
        let material = self.library.material(material_id)?;
        if self.transparent_materials.iter().any(|m| m == material_id) {
            return Err(invalid(format!(
                "{material_id} is transparent glazing: transmittance semantics, no finish or absorbed-solar fallback"
            )));
        }
        let finish = self.finish(finish_id)?;
        if !finish.compatible_materials.iter().any(|m| m == material_id) {
            return Err(invalid(format!(
                "finish {finish_id} is not declared compatible with {material_id}"
            )));
        }
        let pick = |f: &PhysicalInput<QuantityRange>, m: &PhysicalInput<QuantityRange>| match f {
            PhysicalInput::Known { .. } => f.clone(),
            PhysicalInput::Missing { .. } => m.clone(),
        };
        Ok((
            pick(&finish.solar_absorptance, &material.solar_absorptance),
            pick(&finish.emissivity, &material.emissivity),
        ))
    }

    /// The product requirement under which a sourced key's value holds, if
    /// its data entry declares one (`electronics:<id>.<end>` only).
    pub fn requirement(&self, key: &str) -> Option<&str> {
        let (id, _) = key.strip_prefix("electronics:")?.split_once('.')?;
        self.electronics
            .iter()
            .find(|e| e.id == id)?
            .requirement
            .as_deref()
    }

    /// `condition:<class>.<hot|cold>.<ambient_temperature|solar_irradiance|film_coefficient>`,
    /// `finish:<id>.<solar_absorptance|emissivity>` (conservative ends:
    /// absorptance high, emissivity low) or `film:<id>.<low|high>` (the
    /// caller names the end that is conservative for its use), and
    /// `electronics:<id>.<low|high>` likewise.
    pub fn lookup(&self, key: &str) -> Result<Sourced> {
        let bad = || {
            invalid(format!(
                "sourced key {key} must be condition:<class>.<hot|cold>.<ambient_temperature|solar_irradiance|film_coefficient> , finish:<id>.<solar_absorptance|emissivity>, film:<id>.<low|high> or electronics:<id>.<low|high>"
            ))
        };
        if let Some(rest) = key.strip_prefix("condition:") {
            let parts: Vec<&str> = rest.split('.').collect();
            let [class, point, field] = parts[..] else {
                return Err(bad());
            };
            let point = match point {
                "hot" => Point::Hot,
                "cold" => Point::Cold,
                _ => return Err(bad()),
            };
            let p = self.design_point(class, point)?;
            return match field {
                "ambient_temperature" => Ok(p.ambient_temperature),
                "solar_irradiance" => Ok(p.solar_irradiance),
                "film_coefficient" => Ok(p.film_coefficient),
                _ => Err(bad()),
            };
        }
        let named_end = |prefix: &str| {
            key.strip_prefix(prefix)
                .and_then(|r| r.split_once('.'))
                .map(|(id, end_name)| match end_name {
                    "low" => Ok((id, false)),
                    "high" => Ok((id, true)),
                    _ => Err(bad()),
                })
                .transpose()
        };
        if let Some((id, high)) = named_end("film:")? {
            let what = format!("film {id}");
            return end(
                &self.film(id)?.coefficient,
                "heat_transfer_coefficient",
                high,
                &what,
            );
        }
        if let Some((id, high)) = named_end("electronics:")? {
            let e = self
                .electronics
                .iter()
                .find(|e| e.id == id)
                .ok_or_else(|| invalid(format!("unknown electronics default {id}")))?;
            return end(&e.value, &e.dimension, high, &format!("electronics {id}"));
        }
        let (id, field) = key
            .strip_prefix("finish:")
            .and_then(|rest| rest.split_once('.'))
            .ok_or_else(bad)?;
        let finish = self.finish(id)?;
        let what = format!("finish {id} {field}");
        match field {
            "solar_absorptance" => end(&finish.solar_absorptance, "dimensionless", true, &what),
            "emissivity" => end(&finish.emissivity, "dimensionless", false, &what),
            _ => Err(bad()),
        }
    }
}
