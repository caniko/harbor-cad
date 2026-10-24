//! Design-loop metric providers backed by the W1 manufacturing screens.
//!
//! Inputs come only from the envelope: the material/process from the
//! categorical `route` (`MATERIAL__PROCESS` or `MATERIAL__PROCESS__FINISH`;
//! the finish only affects the thermal template) or `material` + `process`
//! parameters, and everything else from already-extracted metrics with these
//! conventional names (SI):
//!
//! - geometry: `volume_m3`, `xmin_m`, `xmax_m`, `ymin_m`, `ymax_m`, `zmin_m`,
//!   `zmax_m` (e.g. `cad-variant-recompute.json` `after.<region>.bounds_m`),
//!   optional `min_wall_m` and `surface_area_m2`;
//! - `quantity` (order size, an integer 1..=1e6);
//! - `max_temperature_k`, optional `min_temperature_k`;
//! - `max_von_mises_pa`, `stress_safety_factor`;
//! - `component_max_temperature_k`, `pcb_max_temperature_k` (component margin);
//! - power sweep (`max_supported_pcb_power_w`): `pcb_power_low_w`,
//!   `pcb_power_high_w`, `housing_margin_required_k` and, per solve, the
//!   board (`pcb_max_temperature_k`, `internal_air_temperature_k`) and housing
//!   (`max_temperature_k`) peaks: unprefixed at the low power, `hi_` at the
//!   high power and `check_` for the refine-tier verification solve.
//!
//! A missing input, missing sourced data or an incompatible route is unknown
//! (`None`), never a default.
use crate::{
    Result,
    design::{Level, MetricProvider, ParamValue, Params},
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
    &ComponentMargin,
    &PowerSweep,
];

pub const MAX_SUPPORTED_PCB_POWER: &str = "max_supported_pcb_power_w";
/// Refine-tier affinity tolerance: the verification solve at the supported
/// power must match the superposed prediction within this many kelvin.
pub const AFFINITY_TOLERANCE_K: f64 = 0.5;

/// Largest PCB power in `[p_lo, p_hi]` keeping every criterion at or below
/// its limit, from solves at the two powers. With radiation off and constant
/// films and solar the model is linear, so every nodal temperature is affine
/// in power; a peak (maximum over nodes) is convex, so the chord between the
/// two solves bounds it from above inside the interval (conservative).
/// `None` if a criterion already exceeds its limit at `p_lo` or does not
/// rise with power (affinity not established).
pub fn supported_power(powers: [f64; 2], lo: &[f64], hi: &[f64], limits: &[f64]) -> Option<f64> {
    let [p_lo, p_hi] = powers;
    if !(p_lo > 0. && p_hi > p_lo) || lo.len() != hi.len() || lo.len() != limits.len() {
        return None;
    }
    let mut best = p_hi;
    for ((lo, hi), limit) in lo.iter().zip(hi).zip(limits) {
        let slope = (hi - lo) / (p_hi - p_lo);
        if !(lo <= limit && slope > 0.) {
            return None;
        }
        best = best.min(p_lo + (limit - lo) / slope);
    }
    best.is_finite().then_some(best)
}

/// Superposed temperatures at `power` (the chord through the two solves).
pub fn superposed(powers: [f64; 2], lo: &[f64], hi: &[f64], power: f64) -> Vec<f64> {
    let t = (power - powers[0]) / (powers[1] - powers[0]);
    lo.iter()
        .zip(hi)
        .map(|(lo, hi)| lo + t * (hi - lo))
        .collect()
}

/// [PCB peak, internal air, housing peak] of one solve. Components sit
/// between board and air, so both must stay under the component limit; they
/// are separate criteria because each is (near-)affine in power while their
/// maximum switches branch and is not.
fn sweep_point(metrics: &BTreeMap<String, f64>, prefix: &str) -> Option<[f64; 3]> {
    let get = |name: &str| metrics.get(&format!("{prefix}{name}")).copied();
    Some([
        get("pcb_max_temperature_k")?,
        get("internal_air_temperature_k")?,
        get("max_temperature_k")?,
    ])
}

/// Maximum steady PCB dissipation the design supports at the hot point:
/// the component limit bounds the PCB peak and the internal air, and the route material's
/// conservative service maximum less `housing_margin_required_k` bounds the
/// housing (as `temperature_margin_k`). Clamped to the swept
/// `[pcb_power_low_w, pcb_power_high_w]`; unknown if even the low power
/// fails. Screen: the superposed estimate. Refine: additionally a third
/// solve at that power must match the prediction within
/// `AFFINITY_TOLERANCE_K`, else unknown.
pub struct PowerSweep;

impl MetricProvider for PowerSweep {
    fn name(&self) -> &'static str {
        MAX_SUPPORTED_PCB_POWER
    }
    fn unit(&self) -> &'static str {
        "W"
    }
    fn evidence(&self) -> &'static str {
        "estimated"
    }
    fn evaluate(&self, params: &Params, metrics: &BTreeMap<String, f64>) -> Result<Option<f64>> {
        self.evaluate_at(Level::Screen, params, metrics)
    }
    fn evaluate_at(
        &self,
        level: Level,
        params: &Params,
        metrics: &BTreeMap<String, f64>,
    ) -> Result<Option<f64>> {
        let get = |name: &str| metrics.get(name).copied();
        let (
            Some((material, _)),
            Some(p_lo),
            Some(p_hi),
            Some(component),
            Some(margin),
            Some(lo),
            Some(hi),
        ) = (
            route(params),
            get("pcb_power_low_w"),
            get("pcb_power_high_w"),
            get("component_max_temperature_k"),
            get("housing_margin_required_k"),
            sweep_point(metrics, ""),
            sweep_point(metrics, "hi_"),
        )
        else {
            return Ok(None);
        };
        let Some(service) =
            service_margins(&Library::builtin()?, material, lo[2], lo[2], None, 1.)?
                .into_iter()
                .find(|m| m.check == MarginCheck::MaxTemperature)
                .and_then(|m| m.capacity)
        else {
            return Ok(None);
        };
        let powers = [p_lo, p_hi];
        let limits = [component, component, service - margin];
        let Some(power) = supported_power(powers, &lo, &hi, &limits) else {
            return Ok(None);
        };
        if level == Level::Refine {
            let Some(check) = sweep_point(metrics, "check_") else {
                return Ok(None);
            };
            let expected = superposed(powers, &lo, &hi, power);
            if check
                .iter()
                .zip(&expected)
                .any(|(t, e)| (t - e).abs() > AFFINITY_TOLERANCE_K)
            {
                return Ok(None);
            }
        }
        Ok(Some(power))
    }
}

/// `component_max_temperature_k` (an operator input) minus the simulated
/// `pcb_max_temperature_k`; unknown unless both are known.
pub struct ComponentMargin;

impl MetricProvider for ComponentMargin {
    fn name(&self) -> &'static str {
        "component_temperature_margin_k"
    }
    fn unit(&self) -> &'static str {
        "K"
    }
    fn evidence(&self) -> &'static str {
        "estimated"
    }
    fn evaluate(&self, _: &Params, metrics: &BTreeMap<String, f64>) -> Result<Option<f64>> {
        let get = |name: &str| metrics.get(name).copied();
        // Components sit between the board solid and the internal air, so the
        // hotter of the two bounds them when the air temperature is extracted.
        let peak = get("pcb_max_temperature_k")
            .map(|pcb| get("internal_air_temperature_k").map_or(pcb, |air| pcb.max(air)));
        Ok(get("component_max_temperature_k")
            .zip(peak)
            .map(|(limit, peak)| limit - peak))
    }
}

fn choice<'a>(params: &'a Params, name: &str) -> Option<&'a str> {
    match params.get(name) {
        Some(ParamValue::Choice(c)) => Some(c),
        _ => None,
    }
}

/// `MATERIAL__PROCESS` or `MATERIAL__PROCESS__FINISH` as (material, process, finish).
pub fn split_route(route: &str) -> Option<(&str, &str, Option<&str>)> {
    let mut parts = route.split("__");
    let (material, process, finish) = (parts.next()?, parts.next()?, parts.next());
    (parts.next().is_none() && ![Some(material), Some(process), finish].contains(&Some("")))
        .then_some((material, process, finish))
}

/// (material, process) from `route` (a pair or a finished triple) or separate
/// `material`/`process` choices.
pub fn route(params: &Params) -> Option<(&str, &str)> {
    choice(params, "route")
        .and_then(split_route)
        .map(|(m, p, _)| (m, p))
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
