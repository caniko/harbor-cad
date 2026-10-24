//! Cold-restart input verification. Valid inputs do not establish a native
//! transient solve, component boot reliability or physical validation.
use crate::{
    Result,
    contracts::{digest, invalid},
    materials::{PhysicalInput, TemperatureCurve, ThermalMaterial},
    science::{Quantity, dew_point},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PowerKnot {
    pub time: Quantity,
    pub power: Quantity,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AmbientKnot {
    pub time: Quantity,
    pub temperature: Quantity,
}

crate::json::tagged_enum! {
"assessment";
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(tag = "assessment", rename_all = "snake_case", deny_unknown_fields)]
pub enum MoistureRisk {
    Missing {
        reason: String,
    },
    Inapplicable {
        justification: String,
    },
    DewPointScreening {
        air_temperature: Quantity,
        relative_humidity: f64,
        minimum_surface_temperature: Quantity,
        provenance: String,
    },
}
}

impl MoistureRisk {
    pub fn inspect(&self) -> Result<serde_json::Value> {
        match self {
            Self::Missing { reason } => {
                bounded_text(reason)?;
                Ok(serde_json::json!({"status":"missing_inputs","reason":reason}))
            }
            Self::Inapplicable { justification } => {
                bounded_text(justification)?;
                Ok(serde_json::json!({"status":"inapplicable","justification":justification}))
            }
            Self::DewPointScreening {
                air_temperature,
                relative_humidity,
                minimum_surface_temperature,
                provenance,
            } => {
                bounded_text(provenance)?;
                let air = air_temperature.si("temperature")?;
                let surface = minimum_surface_temperature.si("temperature")?;
                if surface <= 0. {
                    return Err(invalid("positive absolute surface temperature required"));
                }
                let dew = dew_point(air, *relative_humidity)?;
                if surface < 273.15 {
                    return Ok(
                        serde_json::json!({"status":"unsupported_screening","reason":"subzero surfaces require a separately justified ice/frost model","provenance":provenance}),
                    );
                }
                Ok(
                    serde_json::json!({"status":"screening","model":"Magnus over water, air 0..50 degC","dew_point_k":dew,
                    "minimum_surface_temperature_k":surface,"margin_k":surface-dew,"condensation_risk":surface<=dew,
                    "provenance":provenance,"limitations":["screening only","no condensate mass or moisture transport","no frost or ingress inference"]}),
                )
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ColdRestartSpec {
    pub schema_version: u32,
    pub synthetic: bool,
    pub geometry_sha256: String,
    pub thermal_region: String,
    pub material: ThermalMaterial,
    pub minimum_valid_temperature: Quantity,
    pub maximum_valid_temperature: Quantity,
    pub initial_temperature: Quantity,
    pub duration: Quantity,
    pub history_interpolation: String,
    pub ambient_history: PhysicalInput<Vec<AmbientKnot>>,
    pub heater_history: PhysicalInput<Vec<PowerKnot>>,
    pub convection_coefficient: PhysicalInput<TemperatureCurve>,
    pub observation_times: Vec<Quantity>,
    pub numerical_tolerance: f64,
    pub moisture: MoistureRisk,
}

fn bounded_text(text: &str) -> Result<()> {
    if text.trim().is_empty() || text.len() > 4096 {
        return Err(invalid("explicit bounded provenance/reason required"));
    }
    Ok(())
}
fn times(times: &[Quantity], duration: f64, endpoints: bool) -> Result<Vec<f64>> {
    if times.is_empty() || times.len() > 1024 {
        return Err(invalid("bounded explicit recipe history required"));
    }
    let result: Vec<_> = times.iter().map(|q| q.si("time")).collect::<Result<_>>()?;
    if result.iter().any(|t| *t < 0. || *t > duration)
        || result.windows(2).any(|p| p[0] >= p[1])
        || (endpoints && (result.len() < 2 || result[0] != 0. || result.last() != Some(&duration)))
    {
        return Err(invalid(
            "ordered history must cover the approved duration; no extrapolation",
        ));
    }
    Ok(result)
}

impl ColdRestartSpec {
    pub fn inspect(&self) -> Result<serde_json::Value> {
        let low = self.minimum_valid_temperature.si("temperature")?;
        let high = self.maximum_valid_temperature.si("temperature")?;
        let initial = self.initial_temperature.si("temperature")?;
        let duration = self.duration.si("time")?;
        if self.schema_version != 1
            || self.geometry_sha256.len() != 64
            || !self
                .geometry_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.thermal_region.is_empty()
            || self.thermal_region.len() > 64
            || !self
                .thermal_region
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || low <= 0.
            || high <= low
            || !(low..=high).contains(&initial)
            || duration <= 0.
            || self.history_interpolation != "piecewise_linear"
            || !self.numerical_tolerance.is_finite()
            || !(0. ..1.).contains(&self.numerical_tolerance)
        {
            return Err(invalid(
                "versioned exact geometry, temperature domain and cold-restart numeric policy required",
            ));
        }
        let retained = times(&self.observation_times, duration, false)?;
        let mut missing: Vec<_> = self
            .material
            .validate(low, high)?
            .into_iter()
            .map(|p| format!("material.{p}"))
            .collect();
        self.ambient_history.validate()?;
        self.heater_history.validate()?;
        self.convection_coefficient.validate()?;
        match &self.ambient_history {
            PhysicalInput::Missing { .. } => missing.push("ambient_history".into()),
            PhysicalInput::Known { value, .. } => {
                times(
                    &value.iter().map(|p| p.time.clone()).collect::<Vec<_>>(),
                    duration,
                    true,
                )?;
                for step in value {
                    if !(low..=high).contains(&step.temperature.si("temperature")?) {
                        return Err(invalid(
                            "ambient history outside declared material temperature domain",
                        ));
                    }
                }
            }
        }
        let heater_energy = match &self.heater_history {
            PhysicalInput::Missing { .. } => {
                missing.push("heater_history".into());
                None
            }
            PhysicalInput::Known { value, .. } => {
                let times = times(
                    &value.iter().map(|p| p.time.clone()).collect::<Vec<_>>(),
                    duration,
                    true,
                )?;
                let powers: Vec<_> = value
                    .iter()
                    .map(|p| p.power.si("power"))
                    .collect::<Result<_>>()?;
                if powers.iter().any(|p| *p < 0.) {
                    return Err(invalid(
                        "heater input must specify nonnegative power; cooling requires its own boundary model",
                    ));
                }
                let energy: f64 = times
                    .windows(2)
                    .zip(powers.windows(2))
                    .map(|(t, p)| (t[1] - t[0]) * (p[0] / 2. + p[1] / 2.))
                    .sum();
                if !energy.is_finite() {
                    return Err(invalid("heater energy integral overflow"));
                }
                Some(energy)
            }
        };
        match &self.convection_coefficient {
            PhysicalInput::Missing { .. } => missing.push("convection_coefficient".into()),
            PhysicalInput::Known { value, .. } => {
                value.value_si(low, "heat_transfer_coefficient")?;
                value.value_si(high, "heat_transfer_coefficient")?;
            }
        }
        let moisture = self.moisture.inspect()?;
        if matches!(
            moisture["status"].as_str(),
            Some("missing_inputs" | "unsupported_screening")
        ) {
            missing.push("moisture_risk".into());
        }
        let synthetic_inputs: Vec<_> = [
            ("material.density", self.material.density.is_synthetic()),
            (
                "material.conductivity",
                self.material.conductivity.is_synthetic(),
            ),
            (
                "material.specific_heat",
                self.material.specific_heat.is_synthetic(),
            ),
            ("ambient_history", self.ambient_history.is_synthetic()),
            ("heater_history", self.heater_history.is_synthetic()),
            (
                "convection_coefficient",
                self.convection_coefficient.is_synthetic(),
            ),
        ]
        .into_iter()
        .filter_map(|(name, synthetic)| synthetic.then_some(name))
        .collect();
        Ok(
            serde_json::json!({"valid":true,"schema_version":1,"science_id":digest(self)?,"inputs_complete":missing.is_empty(),"missing_inputs":missing,
            "duration_s":duration,"initial_temperature_k":initial,"temperature_domain_k":[low,high],"observation_times_s":retained,
            "prescribed_heater_energy_j":heater_energy,"moisture_risk":moisture,"synthetic":self.synthetic || !synthetic_inputs.is_empty(),"synthetic_inputs":synthetic_inputs,
            "execution":"not_requested","numerical_verification":"not_run","convergence":"not_run","physical_validation":"unqualified",
            "limitations":["geometric region must be verified after native recompute","property extrapolation forbidden during native solve",
            "prescribed heater energy is not retained heat","no boot-reliability or sealing inference"]}),
        )
    }
}
