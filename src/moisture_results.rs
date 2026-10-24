//! Source-bound planar-box surface dew-point screening from native thermal fields.
use crate::{
    Result,
    contracts::{digest, invalid},
    recipes::MoistureRisk,
    results::{SampleLocation, SampleValue},
    science::Quantity,
    storage::Store,
    thermal_results::{self, ThermalField, ThermalSampleReport, ThermalSampleRequest},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceRegion {
    Xmin,
    Xmax,
    Ymin,
    Ymax,
    Zmin,
    Zmax,
}
impl SurfaceRegion {
    fn name(&self) -> &'static str {
        match self {
            Self::Xmin => "xmin",
            Self::Xmax => "xmax",
            Self::Ymin => "ymin",
            Self::Ymax => "ymax",
            Self::Zmin => "zmin",
            Self::Zmax => "zmax",
        }
    }
}

crate::json::tagged_enum! {
"assessment";
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(tag = "assessment", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeMoistureAssessment {
    Missing {
        reason: String,
    },
    Inapplicable {
        justification: String,
    },
    DewPointScreening {
        air_temperature: Quantity,
        relative_humidity: f64,
        provenance: String,
    },
}
}
impl NativeMoistureAssessment {
    pub(crate) fn inspect(&self, surface_k: f64) -> Result<serde_json::Value> {
        let risk = match self {
            Self::Missing { reason } => MoistureRisk::Missing {
                reason: reason.clone(),
            },
            Self::Inapplicable { justification } => MoistureRisk::Inapplicable {
                justification: justification.clone(),
            },
            Self::DewPointScreening {
                air_temperature,
                relative_humidity,
                provenance,
            } => MoistureRisk::DewPointScreening {
                air_temperature: air_temperature.clone(),
                relative_humidity: *relative_humidity,
                minimum_surface_temperature: Quantity {
                    value: surface_k,
                    unit: "K".into(),
                },
                provenance: provenance.clone(),
            },
        };
        risk.inspect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeMoistureRequest {
    pub schema_version: u32,
    pub job_id: String,
    pub physical_time_s: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_stage: Option<u32>,
    pub surface_region: SurfaceRegion,
    pub moisture_risk: NativeMoistureAssessment,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeMoistureReport {
    pub schema_version: u32,
    pub assessment_id: String,
    pub request: NativeMoistureRequest,
    pub source: ThermalSampleReport,
    pub surface_nodes: usize,
    pub moisture_risk: serde_json::Value,
    pub physical_validation: String,
}

pub(crate) fn surface_minimum(
    mesh: &serde_json::Value,
    temperatures: &serde_json::Value,
    region: &str,
    size: [f64; 3],
    resolution: u32,
    tolerance: f64,
) -> Result<(u64, f64, usize)> {
    let (axis, upper) = match region {
        "xmin" => (0, false),
        "xmax" => (0, true),
        "ymin" => (1, false),
        "ymax" => (1, true),
        "zmin" => (2, false),
        "zmax" => (2, true),
        _ => {
            return Err(invalid(
                "geometrically identified planar box surface required",
            ));
        }
    };
    if resolution == 0
        || resolution > 16
        || size.iter().any(|v| !v.is_finite() || *v <= 0.)
        || !tolerance.is_finite()
        || tolerance <= 0.
        || tolerance >= size[axis] / f64::from(resolution) / 4.
    {
        return Err(invalid("bounded resolved planar surface geometry required"));
    }
    let face = if upper { size[axis] } else { 0. };
    let declared = mesh["boundary_node_sets"][region]
        .as_array()
        .ok_or_else(|| invalid("registered semantic surface node set required"))?;
    let ids = declared
        .iter()
        .map(|v| {
            v.as_u64()
                .filter(|id| *id > 0)
                .ok_or_else(|| invalid("positive native boundary IDs required"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let expected = (resolution as usize + 1).pow(2);
    if ids.len() != expected || ids.len() != declared.len() {
        return Err(invalid("complete unique native surface nodes required"));
    }
    let nodes = mesh["nodes"]
        .as_object()
        .ok_or_else(|| invalid("complete native geometry required"))?;
    let mut located = BTreeSet::new();
    for (id, coordinate) in nodes {
        let id = id
            .parse::<u64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or_else(|| invalid("native node identity required"))?;
        let coordinates = coordinate
            .as_array()
            .filter(|v| v.len() == 3)
            .ok_or_else(|| invalid("native SI 3D coordinates required"))?;
        let values = coordinates
            .iter()
            .enumerate()
            .map(|(a, v)| {
                v.as_f64()
                    .filter(|v| v.is_finite() && *v >= -tolerance && *v <= size[a] + tolerance)
                    .ok_or_else(|| invalid("coordinates outside approved source box"))
            })
            .collect::<Result<Vec<_>>>()?;
        if (values[axis] - face).abs() <= tolerance {
            located.insert(id);
        }
    }
    if located != ids {
        return Err(invalid(
            "semantic surface set differs from native geometric selection",
        ));
    }
    let mut minimum = None;
    for id in ids {
        let temperature = temperatures[id.to_string()]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.)
            .ok_or_else(|| invalid("complete positive native surface temperatures required"))?;
        if minimum.is_none_or(|(_, previous)| temperature < previous) {
            minimum = Some((id, temperature));
        }
    }
    let (id, temperature) = minimum.ok_or_else(|| invalid("native surface absent"))?;
    Ok((id, temperature, expected))
}

pub fn assess(store: &Store, request: &NativeMoistureRequest) -> Result<NativeMoistureReport> {
    if request.schema_version != 1 {
        return Err(invalid("versioned native moisture assessment required"));
    }
    let sample = ThermalSampleRequest {
        schema_version: 1,
        job_id: request.job_id.clone(),
        field: ThermalField::Temperature,
        physical_time_s: request.physical_time_s,
        thermal_stage: request.thermal_stage,
        locations: vec![SampleLocation::Node { node_id: 1 }],
    };
    let verified = thermal_results::verified_sample(store, &sample)?;
    let plan = store.plan(&request.job_id)?;
    let (spec, _) = thermal_results::source_stage(&plan, request.thermal_stage)?;
    let snapshot = verified.fields["times"]
        .as_array()
        .and_then(|v| {
            v.iter()
                .find(|v| v["requested_s"].as_f64() == Some(request.physical_time_s))
        })
        .ok_or_else(|| invalid("verified retained surface time required"))?;
    let (node, temperature, surface_nodes) = surface_minimum(
        &verified.mesh,
        &snapshot["temperature_k"],
        request.surface_region.name(),
        spec.size_m,
        spec.resolution,
        spec.geometry_tolerance_m,
    )?;
    let moisture_risk = request.moisture_risk.inspect(temperature)?;
    let mut source = verified.report;
    source.sample.samples = vec![SampleValue {
        location: SampleLocation::Node { node_id: node },
        value: vec![temperature],
    }];
    Ok(NativeMoistureReport {
        schema_version: 1,
        assessment_id: digest(&(
            request,
            &source.sample.field_artifact,
            &source.sample.mesh_artifact,
            &source.sample.native_artifact,
        ))?,
        request: request.clone(),
        source,
        surface_nodes,
        moisture_risk,
        physical_validation: "unqualified".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn complete_surface_minimum_uses_geometry_not_ordinal_face_or_caller_temperature() {
        let mesh = json!({"nodes":{"1":[0.,0.,0.],"2":[0.,1.,0.],"3":[0.,0.,1.],"4":[0.,1.,1.],"5":[1.,0.,0.],"6":[1.,1.,0.],"7":[1.,0.,1.],"8":[1.,1.,1.]},"boundary_node_sets":{"xmin":[1,2,3,4]}});
        let temperatures =
            json!({"1":290.,"2":285.,"3":285.,"4":295.,"5":200.,"6":300.,"7":300.,"8":300.});
        let (id, min, count) =
            surface_minimum(&mesh, &temperatures, "xmin", [1.; 3], 1, 1e-6).unwrap();
        assert_eq!((id, min, count), (2, 285., 4));
        let mut changed = mesh.clone();
        changed["boundary_node_sets"]["xmin"] = json!([1, 2, 3, 5]);
        assert!(surface_minimum(&changed, &temperatures, "xmin", [1.; 3], 1, 1e-6).is_err());
        let mut changed = temperatures.clone();
        changed.as_object_mut().unwrap().remove("3");
        assert!(surface_minimum(&mesh, &changed, "xmin", [1.; 3], 1, 1e-6).is_err());
        assert!(surface_minimum(&mesh, &temperatures, "face1", [1.; 3], 1, 1e-6).is_err());
    }

    #[test]
    fn source_surface_temperature_controls_screening_and_frost_remains_unsupported() {
        let assessment = NativeMoistureAssessment::DewPointScreening {
            air_temperature: Quantity {
                value: 20.,
                unit: "degC".into(),
            },
            relative_humidity: 0.5,
            provenance: "synthetic indoor air reference".into(),
        };
        let value = assessment.inspect(283.15).unwrap();
        assert_eq!(value["status"], "screening");
        assert_eq!(value["minimum_surface_temperature_k"], 283.15);
        assert_eq!(
            assessment.inspect(253.15).unwrap()["status"],
            "unsupported_screening"
        );
        assert_eq!(
            NativeMoistureAssessment::Missing {
                reason: "humidity unavailable".into()
            }
            .inspect(283.15)
            .unwrap()["status"],
            "missing_inputs"
        );
        assert_eq!(
            NativeMoistureAssessment::Inapplicable {
                justification: "explicit dry sealed reference; not an ingress assessment".into()
            }
            .inspect(283.15)
            .unwrap()["status"],
            "inapplicable"
        );
    }
}
