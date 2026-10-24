//! Explicit temperature-dependent thermal data with bounded interpolation.
use crate::{Result, contracts::invalid, science::Quantity};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(tag = "availability", rename_all = "snake_case", deny_unknown_fields)]
pub enum PhysicalInput<T> {
    Known {
        value: T,
        provenance: String,
        synthetic: bool,
    },
    Missing {
        reason: String,
    },
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for PhysicalInput<T> {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        // Keep generic (including borrowed) inputs typed rather than buffering
        // their numbers through serde's internally tagged Content representation.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, bound(deserialize = "T: Deserialize<'de>"))]
        struct Fields<T> {
            availability: String,
            #[serde(default, deserialize_with = "present")]
            value: Option<T>,
            #[serde(default, deserialize_with = "present")]
            provenance: Option<String>,
            #[serde(default, deserialize_with = "present")]
            synthetic: Option<bool>,
            #[serde(default, deserialize_with = "present")]
            reason: Option<String>,
        }
        fn present<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
        where
            D: serde::Deserializer<'de>,
            T: Deserialize<'de>,
        {
            T::deserialize(deserializer).map(Some)
        }
        let fields = Fields::<T>::deserialize(deserializer)?;
        match fields.availability.as_str() {
            "known" => {
                if fields.reason.is_some() {
                    return Err(serde::de::Error::unknown_field(
                        "reason",
                        &["availability", "value", "provenance", "synthetic"],
                    ));
                }
                Ok(Self::Known {
                    value: fields
                        .value
                        .ok_or_else(|| serde::de::Error::missing_field("value"))?,
                    provenance: fields
                        .provenance
                        .ok_or_else(|| serde::de::Error::missing_field("provenance"))?,
                    synthetic: fields
                        .synthetic
                        .ok_or_else(|| serde::de::Error::missing_field("synthetic"))?,
                })
            }
            "missing" => {
                if fields.value.is_some()
                    || fields.provenance.is_some()
                    || fields.synthetic.is_some()
                {
                    return Err(serde::de::Error::custom("unexpected known input field"));
                }
                Ok(Self::Missing {
                    reason: fields
                        .reason
                        .ok_or_else(|| serde::de::Error::missing_field("reason"))?,
                })
            }
            availability => Err(serde::de::Error::unknown_variant(
                availability,
                &["known", "missing"],
            )),
        }
    }
}

impl<T> PhysicalInput<T> {
    pub fn is_synthetic(&self) -> bool {
        matches!(
            self,
            Self::Known {
                synthetic: true,
                ..
            }
        )
    }
    pub fn validate(&self) -> Result<()> {
        let record = match self {
            Self::Known { provenance, .. } => provenance,
            Self::Missing { reason } => reason,
        };
        if record.trim().is_empty() || record.len() > 4096 {
            return Err(invalid(
                "explicit bounded physical-data provenance or missing-input reason required",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PropertyKnot {
    pub temperature: Quantity,
    pub value: Quantity,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemperatureCurve {
    pub interpolation: String,
    pub knots: Vec<PropertyKnot>,
}

impl TemperatureCurve {
    pub fn validate(&self, dimension: &str) -> Result<()> {
        if self.interpolation != "piecewise_linear" || !(2..=1024).contains(&self.knots.len()) {
            return Err(invalid(
                "bounded explicit piecewise-linear thermal property curve required",
            ));
        }
        let mut previous = -1.;
        for knot in &self.knots {
            let temperature = knot.temperature.si("temperature")?;
            let value = knot.value.si(dimension)?;
            if temperature <= 0.
                || temperature <= previous
                || value < 0.
                || (value == 0. && dimension != "heat_transfer_coefficient")
            {
                return Err(invalid(
                    "ordered positive absolute temperatures and thermal property values required",
                ));
            }
            previous = temperature;
        }
        Ok(())
    }
    pub fn value_si(&self, temperature_k: f64, dimension: &str) -> Result<f64> {
        self.validate(dimension)?;
        if !temperature_k.is_finite() {
            return Err(invalid(
                "finite absolute interpolation temperature required",
            ));
        }
        for pair in self.knots.windows(2) {
            let left = pair[0].temperature.si("temperature")?;
            let right = pair[1].temperature.si("temperature")?;
            if (left..=right).contains(&temperature_k) {
                let fraction = (temperature_k - left) / (right - left);
                let value = (1. - fraction) * pair[0].value.si(dimension)?
                    + fraction * pair[1].value.si(dimension)?;
                if value.is_finite()
                    && (value > 0. || (value == 0. && dimension == "heat_transfer_coefficient"))
                {
                    return Ok(value);
                }
                return Err(invalid("thermal property interpolation overflow"));
            }
        }
        Err(invalid(
            "temperature outside declared material data; extrapolation is forbidden",
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThermalMaterial {
    pub density: PhysicalInput<Quantity>,
    pub conductivity: PhysicalInput<TemperatureCurve>,
    pub specific_heat: PhysicalInput<TemperatureCurve>,
}

impl ThermalMaterial {
    /// Numeric/coverage checks preserve unknowns; returning missing fields does
    /// not authorize execution or supply synthetic defaults.
    pub fn validate(&self, minimum_k: f64, maximum_k: f64) -> Result<Vec<String>> {
        if !minimum_k.is_finite()
            || !maximum_k.is_finite()
            || minimum_k <= 0.
            || maximum_k < minimum_k
        {
            return Err(invalid(
                "positive bounded declared temperature domain required",
            ));
        }
        self.density.validate()?;
        let mut missing = Vec::new();
        match &self.density {
            PhysicalInput::Known { value, .. } if value.si("density")? <= 0. => {
                return Err(invalid("positive thermal material density required"));
            }
            PhysicalInput::Missing { .. } => missing.push("density".into()),
            _ => {}
        }
        for (name, dimension, input) in [
            ("conductivity", "thermal_conductivity", &self.conductivity),
            ("specific_heat", "specific_heat", &self.specific_heat),
        ] {
            input.validate()?;
            match input {
                PhysicalInput::Known { value, .. } => {
                    value.value_si(minimum_k, dimension)?;
                    value.value_si(maximum_k, dimension)?;
                }
                PhysicalInput::Missing { .. } => missing.push(name.into()),
            }
        }
        Ok(missing)
    }
}
