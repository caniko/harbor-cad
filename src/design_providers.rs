//! Design-loop metric providers backed by the W1 manufacturing screens.
//!
//! Inputs come only from the envelope: the material/process from the
//! categorical `route` (`MATERIAL__PROCESS`) or `material` + `process`
//! parameters, and everything else from already-extracted metrics with these
//! conventional names (SI):
//!
//! - geometry: `volume_m3`, `xmin_m`, `xmax_m`, `ymin_m`, `ymax_m`, `zmin_m`,
//!   `zmax_m` (e.g. `cad-variant-recompute.json` `after.<region>.bounds_m`),
//!   optional `min_wall_m` and `surface_area_m2`;
//! - `quantity` (order size, an integer 1..=1e6);
//! - `max_temperature_k`, optional `min_temperature_k`;
//! - `max_von_mises_pa`, `stress_safety_factor`.
//!
//! A missing input, missing sourced data or an incompatible route is unknown
//! (`None`), never a default.
use crate::{
    Result,
    design::{MetricProvider, ParamValue, Params},
    manufacturing::{
        GeometrySummary, Library, MarginCheck, Severity, manufacturability, service_margins,
        unit_cost,
    },
    materials::PhysicalInput,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub enum Manufacturing {
    UnitCostLow,
    UnitCostHigh,
    Mass,
    Violations,
    TemperatureMargin,
    StressMargin,
}

pub static ALL: &[&dyn MetricProvider] = &[
    &Manufacturing::UnitCostLow,
    &Manufacturing::UnitCostHigh,
    &Manufacturing::Mass,
    &Manufacturing::Violations,
    &Manufacturing::TemperatureMargin,
    &Manufacturing::StressMargin,
];

fn choice<'a>(params: &'a Params, name: &str) -> Option<&'a str> {
    match params.get(name) {
        Some(ParamValue::Choice(c)) => Some(c),
        _ => None,
    }
}

/// (material, process) from `route` or separate `material`/`process` choices.
pub fn route(params: &Params) -> Option<(&str, &str)> {
    choice(params, "route")
        .and_then(|r| r.split_once("__"))
        .or_else(|| Some((choice(params, "material")?, choice(params, "process")?)))
}

pub fn geometry(metrics: &BTreeMap<String, f64>) -> Option<GeometrySummary> {
    let m = |name: &str| metrics.get(name).copied();
    Some(GeometrySummary {
        volume_m3: m("volume_m3")?,
        bbox_m: [
            m("xmax_m")? - m("xmin_m")?,
            m("ymax_m")? - m("ymin_m")?,
            m("zmax_m")? - m("zmin_m")?,
        ],
        min_wall_m: m("min_wall_m"),
        surface_area_m2: m("surface_area_m2"),
    })
}

fn quantity(metrics: &BTreeMap<String, f64>) -> Option<u32> {
    metrics
        .get("quantity")
        .filter(|q| q.fract() == 0. && (1. ..=1e6).contains(*q))
        .map(|q| *q as u32)
}

impl MetricProvider for Manufacturing {
    fn name(&self) -> &'static str {
        match self {
            Self::UnitCostLow => "unit_cost_low_usd",
            Self::UnitCostHigh => "unit_cost_high_usd",
            Self::Mass => "mass_kg",
            Self::Violations => "manufacturability_violations",
            Self::TemperatureMargin => "temperature_margin_k",
            Self::StressMargin => "stress_margin",
        }
    }
    fn unit(&self) -> &'static str {
        match self {
            Self::UnitCostLow | Self::UnitCostHigh => "USD",
            Self::Mass => "kg",
            Self::Violations | Self::StressMargin => "1",
            Self::TemperatureMargin => "K",
        }
    }
    fn evidence(&self) -> &'static str {
        "estimated"
    }
    fn evaluate(&self, params: &Params, metrics: &BTreeMap<String, f64>) -> Result<Option<f64>> {
        let Some((material, process)) = route(params) else {
            return Ok(None);
        };
        let library = Library::builtin()?;
        let get = |name: &str| metrics.get(name).copied();
        Ok(match self {
            Self::UnitCostLow | Self::UnitCostHigh => {
                let (Some(geometry), Some(quantity)) = (geometry(metrics), quantity(metrics))
                else {
                    return Ok(None);
                };
                let estimate = unit_cost(&library, &geometry, material, process, quantity)?;
                match self {
                    Self::UnitCostLow => estimate.low,
                    _ => estimate.high,
                }
            }
            // Conservative: the high end of the sourced density range.
            Self::Mass => match (geometry(metrics), &library.material(material)?.density) {
                (Some(geometry), PhysicalInput::Known { value, .. }) => {
                    Some(geometry.volume_m3 * value.si("density")?.1)
                }
                _ => None,
            },
            // Marginal findings (inside the sourced spread) count; any unknown
            // check makes the count unknown rather than zero.
            Self::Violations => {
                let Some(geometry) = geometry(metrics) else {
                    return Ok(None);
                };
                let found = manufacturability(&library, &geometry, material, process)?;
                (!found.iter().any(|v| v.severity == Severity::Unknown))
                    .then_some(found.len() as f64)
            }
            Self::TemperatureMargin => {
                let Some(max) = get("max_temperature_k") else {
                    return Ok(None);
                };
                let min = get("min_temperature_k");
                // The max-temperature margin does not depend on the minimum;
                // without one, only the max check is reported.
                let margins =
                    service_margins(&library, material, max, min.unwrap_or(max), None, 1.)?;
                let pick = |check| {
                    margins
                        .iter()
                        .find(|m| m.check == check)
                        .and_then(|m| m.margin)
                };
                match (pick(MarginCheck::MaxTemperature), min) {
                    (Some(high), None) => Some(high),
                    (high, Some(_)) => high
                        .zip(pick(MarginCheck::MinTemperature))
                        .map(|(h, l)| h.min(l)),
                    (None, None) => None,
                }
            }
            Self::StressMargin => {
                let (Some(stress), Some(factor)) =
                    (get("max_von_mises_pa"), get("stress_safety_factor"))
                else {
                    return Ok(None);
                };
                // Temperatures do not enter the stress margin; service_margins
                // only range-checks them, so a neutral pair is passed.
                service_margins(&library, material, 1., 1., Some(stress), factor)?
                    .into_iter()
                    .find(|m| m.check == MarginCheck::Stress)
                    .and_then(|m| m.margin)
            }
        })
    }
}
