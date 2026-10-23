//! Explicit one-way transient-temperature to static planar-contact recipe.
use crate::contracts::*;
use crate::{
    Result, contact::ContactReferenceSpec, contracts::invalid,
    moisture_results::NativeMoistureAssessment, thermal::ThermalReferenceSpec,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Mechanical inputs only: final temperatures are derived from native fields.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContactMechanics {
    pub size_m: [f64; 3],
    pub resolution: u32,
    pub geometry_tolerance_m: f64,
    pub initial_gap_m: f64,
    pub preload_compression_m: f64,
    pub final_compression_m: f64,
    pub young_modulus_pa: [f64; 2],
    pub expansion_per_k: [f64; 2],
    pub reference_temperature_k: f64,
    pub contact_stiffness_pa_m: f64,
    pub numerical_tolerance: f64,
    pub material_provenance: String,
    pub contact_provenance: String,
    pub boundary_provenance: String,
}
impl ContactMechanics {
    pub fn reference(&self, temperatures_k: [f64; 2]) -> Result<ContactReferenceSpec> {
        let spec = ContactReferenceSpec {
            schema_version: 1,
            synthetic: true,
            backend: "cpu".into(),
            formulation: "planar_linear_penalty_contact".into(),
            size_m: self.size_m,
            resolution: self.resolution,
            geometry_tolerance_m: self.geometry_tolerance_m,
            initial_gap_m: self.initial_gap_m,
            preload_compression_m: self.preload_compression_m,
            final_compression_m: self.final_compression_m,
            young_modulus_pa: self.young_modulus_pa,
            expansion_per_k: self.expansion_per_k,
            reference_temperature_k: self.reference_temperature_k,
            final_temperatures_k: temperatures_k,
            contact_stiffness_pa_m: self.contact_stiffness_pa_m,
            numerical_tolerance: self.numerical_tolerance,
            material_provenance: self.material_provenance.clone(),
            contact_provenance: self.contact_provenance.clone(),
            boundary_provenance: self.boundary_provenance.clone(),
        };
        spec.validate()?;
        Ok(spec)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThermalContactSpec {
    pub schema_version: u32,
    pub synthetic: bool,
    /// Independent lower/upper block histories. There is no feedback of contact
    /// pressure/gap into thermal interface conductance in this one-way model.
    pub thermal: [ThermalReferenceSpec; 2],
    pub mechanical: ContactMechanics,
    pub coupling_time_s: f64,
    pub maximum_projection_error_k: [f64; 2],
    pub maximum_relative_conservation_error: f64,
    pub moisture_risk: NativeMoistureAssessment,
    pub coupling_provenance: String,
}
impl ThermalContactSpec {
    fn times(&self) -> Vec<f64> {
        let mut times: Vec<_> = self
            .thermal
            .iter()
            .flat_map(|s| s.observation_times_s.iter().copied())
            .collect();
        times.sort_by(f64::total_cmp);
        times.dedup();
        times
    }

    fn transfers(&self) -> Vec<TransferSpec> {
        ["lower", "upper"]
            .into_iter()
            .map(|block| TransferSpec {
                source_region: format!("thermal-{block}-entire-box"),
                destination_region: block.into(),
                source_quantity: "temperature".into(),
                destination_quantity: "temperature".into(),
                unit: "K".into(),
                orientation: [0., 0., 1.],
                interpolation: "complete_c3d8_lumped_capacitance_to_congruent_uniform_box".into(),
                maximum_relative_conservation_error: self.maximum_relative_conservation_error,
            })
            .collect()
    }

    pub(crate) fn validate_plan(&self, plan: &ExecutionPlan) -> Result<()> {
        self.validate()?;
        let expected = coupling_stages();
        if plan.policy == "ci"
            || plan.stages.len() != expected.len()
            || plan.stages.iter().zip(expected).any(|(actual, expected)| {
                actual.id != expected.id
                    || actual.operation != expected.operation
                    || actual.dependencies != expected.dependencies
            })
            || digest(&plan.transfers)? != digest(&self.transfers())?
            || plan.observation.retained_times_s != self.times()
            || !plan.observation.metrics.is_empty()
            || !plan.observation.probes.is_empty()
            || !plan.observation.checkpoint_times_s.is_empty()
            || !plan.observation.preview_times_s.is_empty()
            || plan.observation.preview_may_drop
        {
            return Err(invalid(
                "exact serial native lower/upper thermal, conservative projection, contact and bundle DAG with independent retained histories required",
            ));
        }
        Ok(())
    }

    pub(crate) fn thermal_stage(&self, id: &str) -> Result<&ThermalReferenceSpec> {
        match id {
            "thermal-lower" => Ok(&self.thermal[0]),
            "thermal-upper" => Ok(&self.thermal[1]),
            _ => Err(invalid(
                "named lower or upper native thermal source stage required",
            )),
        }
    }

    pub fn validate(&self) -> Result<()> {
        let m = &self.mechanical;
        m.reference([m.reference_temperature_k; 2])?;
        if self.schema_version != 1
            || !self.synthetic
            || !self.coupling_time_s.is_finite()
            || self.coupling_time_s <= 0.
            || self
                .maximum_projection_error_k
                .iter()
                .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
            || !self.maximum_relative_conservation_error.is_finite()
            || self.maximum_relative_conservation_error <= 0.
            || self.maximum_relative_conservation_error > 1e-10
            || self.coupling_provenance.trim().is_empty()
            || self.coupling_provenance.len() > 4096
        {
            return Err(invalid(
                "explicit synthetic one-way coupling, exact retained time, bounded Kelvin projection loss and conservative gate required",
            ));
        }
        for (block, thermal) in self.thermal.iter().enumerate() {
            thermal.validate()?;
            if thermal.size_m != m.size_m
                || thermal.geometry_tolerance_m != m.geometry_tolerance_m
                || thermal.initial_temperature_k != m.reference_temperature_k
                || !thermal.observation_times_s.contains(&self.coupling_time_s)
                || thermal.material_temperature_domain_k.iter().any(|t| {
                    !(100. ..=1000.).contains(t)
                        || (m.expansion_per_k[block] * (t - m.reference_temperature_k)).abs()
                            > 0.001
                })
            {
                return Err(invalid(
                    "congruent native thermal/contact boxes, same geometry tolerance and stress-free initial reference, exact retained coupling time and complete small-strain material range required",
                ));
            }
        }
        // Validates the declared air/missing/inapplicable branch, never a
        // caller-provided surface temperature. Actual screening is post-solve.
        self.moisture_risk.inspect(m.reference_temperature_k)?;
        Ok(())
    }
}

fn coupling_stages() -> Vec<Stage> {
    [
        (
            "thermal-lower",
            vec![],
            StageOperation::ThermalReference,
            2048,
        ),
        (
            "thermal-upper",
            vec!["thermal-lower"],
            StageOperation::ThermalReference,
            2048,
        ),
        (
            "projection",
            vec!["thermal-lower", "thermal-upper"],
            StageOperation::ThermalProjection,
            512,
        ),
        (
            "contact",
            vec!["projection"],
            StageOperation::ContactReference,
            2048,
        ),
        ("bundle", vec!["contact"], StageOperation::Bundle, 16),
    ]
    .into_iter()
    .map(|(id, deps, operation, mib)| Stage {
        id: id.into(),
        dependencies: deps.into_iter().map(String::from).collect(),
        operation,
        gpu: GpuRequirement::CpuOnly,
        selection: None,
        ram_bytes: mib * 1024 * 1024,
        vram_bytes: 0,
    })
    .collect()
}

impl ExecutionPlan {
    pub fn thermal_contact(spec: ThermalContactSpec, policy: String) -> Result<Self> {
        spec.validate()?;
        let mut plan = Self {
            schema_version: 11,
            case: None,
            fem: None,
            thermal: None,
            cad_source: None,
            imported_fem: None,
            wetting: None,
            contact: None,
            source: None,
            frames: None,
            filter: None,
            stages: coupling_stages(),
            transfers: spec.transfers(),
            observation: ObservationPlan {
                metrics: vec![],
                probes: vec![],
                retained_times_s: spec.times(),
                checkpoint_times_s: vec![],
                preview_times_s: vec![],
                max_artifact_bytes: 0,
                scientific_congestion: "fail".into(),
                preview_may_drop: false,
            },
            thermal_contact: Some(spec),
            freezing: None,
            spectral: None,
            atmosphere: None,
            atmospheric_transport: None,
            cad_variant: None,
            cad_sheet_variant: None,
            cad_transport: None,
            retained_cooling: None,
            fleetix_revision: FLEETIX_REV.into(),
            fleetix_contract_digest: fleetix_digest(),
            policy,
        };
        crate::estimates::minimum(&plan)?.apply(&mut plan);
        plan.validate()?;
        Ok(plan)
    }
}

/// Reconstruct the mechanical descriptor from complete original native fields.
/// The same deterministic operation runs before contact launch and during
/// historical qualification; no caller-supplied final temperature is accepted.
pub(crate) fn derive(
    spec: &ThermalContactSpec,
    job_id: &str,
    root: &std::path::Path,
) -> Result<(ContactReferenceSpec, serde_json::Value)> {
    use crate::transfers::*;
    use sha2::{Digest, Sha256};
    spec.validate()?;
    let mut temperatures = [0.; 2];
    let mut projections = Vec::new();
    for (i, block) in ["lower", "upper"].into_iter().enumerate() {
        let thermal = &spec.thermal[i];
        let prefix = format!("stages/thermal-{block}");
        let mut bytes = std::collections::BTreeMap::new();
        let mut hashes = std::collections::BTreeMap::new();
        for name in [
            "mesh.json",
            "thermal-fields.json",
            "reference.dat",
            "thermal-receipt.json",
        ] {
            let path = format!("{prefix}/{name}");
            let data = crate::worker::read_bounded(
                &crate::storage::safe_path(root, &path)?,
                if name == "thermal-receipt.json" {
                    256 * 1024
                } else {
                    32 * 1024 * 1024
                },
            )?;
            hashes.insert(path, format!("{:x}", Sha256::digest(&data)));
            bytes.insert(name, data);
        }
        let receipt: serde_json::Value = serde_json::from_slice(&bytes["thermal-receipt.json"])?;
        crate::thermal::verify_spec(thermal, &receipt)?;
        if receipt["mesh_sha256"] != hashes[&format!("{prefix}/mesh.json")]
            || receipt["native_field_sha256"] != hashes[&format!("{prefix}/reference.dat")]
        {
            return Err(invalid(
                "complete native coupling source bytes differ from verified receipt",
            ));
        }
        let mesh: serde_json::Value = serde_json::from_slice(&bytes["mesh.json"])?;
        let fields: serde_json::Value = serde_json::from_slice(&bytes["thermal-fields.json"])?;
        let native_time_s = crate::thermal_results::verify_native_state(
            thermal,
            job_id,
            spec.coupling_time_s,
            &mesh,
            &fields,
            std::str::from_utf8(&bytes["reference.dat"])
                .map_err(|_| invalid("native thermal DAT text required"))?,
        )?;
        let snapshot = fields["times"]
            .as_array()
            .and_then(|times| {
                times
                    .iter()
                    .find(|t| t["requested_s"].as_f64() == Some(spec.coupling_time_s))
            })
            .ok_or_else(|| invalid("exact retained coupling source snapshot required"))?;
        let projection = crate::thermal_transfer::project_box(
            &mesh,
            &snapshot["temperature_k"],
            thermal.size_m,
            thermal.resolution,
            thermal.geometry_tolerance_m,
            [thermal.density_kg_m3, thermal.specific_heat_j_kg_k],
            spec.maximum_projection_error_k[i],
        )?;
        let origin = if i == 0 {
            [0.; 3]
        } else {
            [
                0.,
                0.,
                spec.mechanical.size_m[2] + spec.mechanical.initial_gap_m,
            ]
        };
        let destination = serde_json::json!({"region":block,"size_m":spec.mechanical.size_m,"origin_m":origin,
            "coordinate_unit":"m","association":"cell","cells":1,"formulation":"congruent_constant_capacitance_box_projection"});
        let measure = |value| crate::science::Quantity {
            value,
            unit: "J/K".into(),
        };
        let map = ConservativeTransfer {
            schema_version: 1,
            source_artifact_sha256: hashes[&format!("{prefix}/thermal-fields.json")].clone(),
            quantity: TransferQuantity::Temperature,
            source: TransferEndpoint {
                mesh_sha256: hashes[&format!("{prefix}/mesh.json")].clone(),
                region: format!("thermal-{block}-entire-box"),
                association: Association::Point,
                orientation: [0., 0., 1.],
                measures: projection
                    .capacitances
                    .iter()
                    .copied()
                    .map(measure)
                    .collect(),
            },
            destination: TransferEndpoint {
                mesh_sha256: digest(&destination)?,
                region: block.into(),
                association: Association::Cell,
                orientation: [0., 0., 1.],
                measures: vec![measure(projection.capacitance_j_k)],
            },
            normal_mapping: NormalMapping::SameDirection,
            interpolation: "piecewise_constant_overlap".into(),
            overlaps: projection
                .capacitances
                .iter()
                .enumerate()
                .map(|(source, &weight)| Overlap {
                    source,
                    destination: 0,
                    measure: measure(weight),
                })
                .collect(),
            maximum_relative_conservation_error: spec.maximum_relative_conservation_error,
        };
        let transfer = map.apply(&projection.temperatures, "K")?;
        if transfer.values_si.len() != 1
            || (transfer.values_si[0] - projection.mean_k).abs() > 1e-10
            || (transfer.receipt.source_integral - projection.source_integral_j).abs()
                / projection.source_integral_j
                > 1e-12
        {
            return Err(invalid(
                "independent native mean and conservative coupling projection disagree",
            ));
        }
        temperatures[i] = transfer.values_si[0];
        let mut surfaces = Vec::new();
        for region in ["xmin", "xmax", "ymin", "ymax", "zmin", "zmax"] {
            let (node, temperature, count) = crate::moisture_results::surface_minimum(
                &mesh,
                &snapshot["temperature_k"],
                region,
                thermal.size_m,
                thermal.resolution,
                thermal.geometry_tolerance_m,
            )?;
            surfaces.push(
                serde_json::json!({"region":region,"minimum_node_id":node,"surface_nodes":count,"minimum_surface_temperature_k":temperature,
                "moisture_risk":spec.moisture_risk.inspect(temperature)?}),
            );
        }
        projections.push(serde_json::json!({"source_stage":format!("thermal-{block}"),"source_files_sha256":hashes,
            "source_spec_sha256":digest(thermal)?,"physical_time_s":spec.coupling_time_s,"native_time_s":native_time_s,
            "source_nodes":projection.temperatures.len(),"material_provenance":thermal.material_provenance,
            "source_density_kg_m3":thermal.density_kg_m3,"source_specific_heat_j_kg_k":thermal.specific_heat_j_kg_k,
            "destination":destination,"destination_temperature_k":temperatures[i],"capacitance_j_k":projection.capacitance_j_k,
            "maximum_abs_projection_error_k":projection.maximum_error_k,"approved_maximum_projection_error_k":spec.maximum_projection_error_k[i],
            "transfer":transfer.receipt,"surfaces":surfaces}));
    }
    let contact = spec.mechanical.reference(temperatures)?;
    let mut report = serde_json::json!({"schema_version":1,"process":"succeeded","backend":"cpu","precision":"float64",
        "physical_validation":"unqualified","approved_coupling_sha256":digest(spec)?,"job_id":job_id,
        "derived_contact":contact,"projections":projections,
        "model":"complete native C3D8 lumped-capacitance projection to explicitly translated congruent uniform contact blocks; no contact-to-thermal feedback",
        "coupling_provenance":spec.coupling_provenance});
    report["projection_id"] = serde_json::json!(digest(&report)?);
    Ok((contact, report))
}

pub(crate) fn verify_registered(
    store: &crate::storage::Store,
    job_id: &str,
    plan: &ExecutionPlan,
) -> Result<(
    ContactReferenceSpec,
    crate::qualification::NumericalEvidence,
)> {
    let spec = plan
        .thermal_contact
        .as_ref()
        .ok_or_else(|| invalid("registered approved coupling required"))?;
    let (contact, expected) = derive(spec, job_id, &store.job_dir(job_id)?)?;
    let (_, actual) =
        crate::results::registered(store, job_id, "stages/projection/projection-receipt.json")?;
    if actual != expected {
        return Err(invalid(
            "registered coupling projection differs from complete original native sources",
        ));
    }
    let mut maximum: f64 = 0.;
    for block in expected["projections"]
        .as_array()
        .ok_or_else(|| invalid("two native projections required"))?
    {
        for (path, hash) in block["source_files_sha256"]
            .as_object()
            .ok_or_else(|| invalid("native source hashes required"))?
        {
            let record = store
                .artifact_record(job_id, path)?
                .ok_or_else(|| invalid("registered native coupling source absent"))?;
            if record.sha256 != *hash {
                return Err(invalid(
                    "coupling source hash differs from registered original bytes",
                ));
            }
        }
        maximum = maximum.max(
            block["transfer"]["relative_conservation_error"]
                .as_f64()
                .ok_or_else(|| invalid("conservation receipt required"))?,
        );
    }
    Ok((contact,crate::qualification::NumericalEvidence {
        reference:"complete original C3D8 DAT/JSON fields, affine positive volumes and rho*cp*V/8 capacitance conservation; approved pointwise projection loss checked independently".into(),
        scope:"two explicitly translated congruent uniform contact blocks; native-source surface moisture; no feedback or physical-validation claim".into(),
        error_kind:"maximum_relative_conservation_error".into(),error:maximum,tolerance:spec.maximum_relative_conservation_error,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, fs, path::Path};

    // Synthetic parser/transfer data, not an executed thermal solve. Preserve
    // different block fields and all native schedules to test the real boundary.
    fn source_fixture(root: &Path, spec: &ThermalReferenceSpec, block: &str, base: f64) {
        let id = |x: u32, y: u32, z: u32| z * 9 + y * 3 + x + 1;
        let mut nodes = BTreeMap::new();
        let mut temperatures = BTreeMap::new();
        let mut sets: BTreeMap<&str, Vec<u32>> = ["xmin", "xmax", "ymin", "ymax", "zmin", "zmax"]
            .into_iter()
            .map(|s| (s, vec![]))
            .collect();
        for z in 0..=2 {
            for y in 0..=2 {
                for x in 0..=2 {
                    let tag = id(x, y, z);
                    nodes.insert(
                        tag,
                        [x as f64 * 0.0005, y as f64 * 0.0005, z as f64 * 0.0005],
                    );
                    temperatures.insert(
                        tag,
                        format!("{:.15e}", base + 0.4 * (x as f64 / 2.).powi(2))
                            .parse::<f64>()
                            .unwrap(),
                    );
                    for (axis, v) in [x, y, z].into_iter().enumerate() {
                        if v == 0 {
                            sets.get_mut(["xmin", "ymin", "zmin"][axis])
                                .unwrap()
                                .push(tag);
                        }
                        if v == 2 {
                            sets.get_mut(["xmax", "ymax", "zmax"][axis])
                                .unwrap()
                                .push(tag);
                        }
                    }
                }
            }
        }
        let mut cells = BTreeMap::new();
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    cells.insert(
                        z * 4 + y * 2 + x + 1,
                        [
                            id(x, y, z),
                            id(x + 1, y, z),
                            id(x + 1, y + 1, z),
                            id(x, y + 1, z),
                            id(x, y, z + 1),
                            id(x + 1, y, z + 1),
                            id(x + 1, y + 1, z + 1),
                            id(x, y + 1, z + 1),
                        ],
                    );
                }
            }
        }
        let mesh = json!({"schema_version":1,"coordinate_unit":"m","element_type":"C3D8","nodes":nodes,"elements":cells,"boundary_node_sets":sets});
        let fields = json!({"schema_version":1,"field":"temperature","unit":"K","association":"point","coordinate_unit":"m",
            "initial_condition":{"time_s":0.,"temperature_k":spec.initial_temperature_k},
            "times":spec.observation_times_s.iter().map(|&t|json!({"requested_s":t,"observed_s":t,"temperature_k":temperatures})).collect::<Vec<_>>()});
        let mut dat = String::new();
        for t in spec.output_times() {
            dat.push_str(&format!(" temperatures for set NALL and time {t:.15e}\n"));
            for (id, t) in &temperatures {
                dat.push_str(&format!("{id} {t:.15e}\n"));
            }
        }
        let checks: BTreeMap<_, _> = [
            "operation_closure_only",
            "no_gpu_nodes",
            "no_sysfs",
            "no_host_home",
            "no_session_bus",
            "no_worker_socket",
            "network_namespace_isolated",
            "descriptor_readonly",
        ]
        .into_iter()
        .map(|s| (s, true))
        .collect();
        let path = root.join(format!("stages/thermal-{block}"));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("mesh.json"), serde_json::to_vec(&mesh).unwrap()).unwrap();
        fs::write(
            path.join("thermal-fields.json"),
            serde_json::to_vec(&fields).unwrap(),
        )
        .unwrap();
        fs::write(path.join("reference.dat"), dat).unwrap();
        let receipt = json!({"schema_version":1,"adapter":"CalculiX","backend":"cpu","factorization":"SPOOLES","executed":true,"software_fallback":false,"synthetic":true,"precision":"float64",
            "request_sha256":digest(spec).unwrap(),"formulation":spec.formulation,"calculix_version":"2.23","gmsh_version":"4.15.2",
            "calculix_source_sha256":"9c88385c10fb04f5dc6c4e98027a51bebdd8aee3920e05190d6c1dd08357d6e7","gmsh_source_sha256":"be3f66f225d27ba9fa014f07e83169285da8a051b0e8ab7103d88066b39bdd3e",
            "temperature_serialization_patch_sha256":crate::thermal::temperature_patch_sha256(),"temperature_serialization":"E23.15; 16 significant decimal digits from native real*8",
            "mesh_sha256":format!("{:x}",Sha256::digest(fs::read(path.join("mesh.json")).unwrap())),"native_field_sha256":format!("{:x}",Sha256::digest(fs::read(path.join("reference.dat")).unwrap())),
            "nodes":27,"elements":8,"physical_validation":"unqualified","moisture_risk":spec.moisture_risk,"physical_times_s":spec.observation_times_s,
            "energy_output_times_s":spec.output_times(),"maximum_native_step_s":spec.integration_step(),"integration_substeps":spec.integration_substeps,
            "sandbox":{"policy":crate::execution::THERMAL_SANDBOX_POLICY,"checks":checks},
            "numerical_verification":{"temperature":{"passed":true,"normalized_max_abs_error":0.,"tolerance":0.02,"reference":"synthetic parser fixture only","unit":"K","samples":81},
            "energy":{"passed":true,"maximum_relative_balance_error":0.,"tolerance":0.02,"reference":"synthetic parser fixture only","unit":"J","samples":spec.output_times().len()}}});
        fs::write(
            path.join("thermal-receipt.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn registered_thermal_originals_reject_changed_missing_and_receipt_substitution() {
        let coupling: ThermalContactSpec =
            serde_json::from_str(include_str!("../examples/thermal-contact.json")).unwrap();
        let spec = &coupling.thermal[0];
        let plan = ExecutionPlan::thermal_reference(spec.clone(), "research".into()).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let store = crate::storage::Store::open(&temp.path().join("state")).unwrap();
        let job = store
            .submit(&plan, "unexecuted-thermal-original-parser")
            .unwrap();
        let root = store.job_dir(&job.id).unwrap();
        source_fixture(&root, spec, "lower", 283.15);
        let prefix = "stages/thermal-lower";
        for name in [
            "mesh.json",
            "thermal-fields.json",
            "reference.dat",
            "thermal-receipt.json",
        ] {
            let path = format!("{prefix}/{name}");
            let record = crate::storage::native_manifest(
                &root,
                &path,
                32 * 1024 * 1024,
                "unexecuted synthetic parser fixture",
            )
            .unwrap();
            store.add_artifact(&job.id, &record).unwrap();
        }
        let receipt: Value = serde_json::from_slice(
            &fs::read(root.join(format!("{prefix}/thermal-receipt.json"))).unwrap(),
        )
        .unwrap();
        assert!(
            crate::thermal_results::verify_registered_originals(
                &store, &job.id, spec, prefix, &receipt
            )
            .is_ok()
        );
        for name in ["mesh.json", "thermal-fields.json", "reference.dat"] {
            let path = root.join(format!("{prefix}/{name}"));
            let original = fs::read(&path).unwrap();
            let mut changed = original.clone();
            changed.extend_from_slice(b"changed");
            fs::write(&path, &changed).unwrap();
            assert!(
                crate::thermal_results::verify_registered_originals(
                    &store, &job.id, spec, prefix, &receipt
                )
                .is_err(),
                "{name}"
            );
            fs::remove_file(&path).unwrap();
            assert!(
                crate::thermal_results::verify_registered_originals(
                    &store, &job.id, spec, prefix, &receipt
                )
                .is_err(),
                "{name}"
            );
            fs::write(&path, original).unwrap();
        }
        let mut changed = receipt;
        changed["native_field_sha256"] = json!("a".repeat(64));
        assert!(
            crate::thermal_results::verify_registered_originals(
                &store, &job.id, spec, prefix, &changed
            )
            .is_err()
        );
        // These manufactured records never establish an executed solver.
        assert_eq!(store.job(&job.id).unwrap().state, "queued");
    }

    #[test]
    fn derived_contact_uses_native_capacitance_weighting_and_original_surface_moisture() {
        let mut spec: ThermalContactSpec =
            serde_json::from_str(include_str!("../examples/thermal-contact.json")).unwrap();
        spec.moisture_risk = NativeMoistureAssessment::DewPointScreening {
            air_temperature: crate::science::Quantity {
                value: 293.15,
                unit: "K".into(),
            },
            relative_humidity: 0.8,
            provenance: "synthetic test air".into(),
        };
        let root = tempfile::tempdir().unwrap();
        source_fixture(root.path(), &spec.thermal[0], "lower", 283.15);
        source_fixture(root.path(), &spec.thermal[1], "upper", 288.15);
        let job = uuid::Uuid::new_v4().to_string();
        let (contact, report) = derive(&spec, &job, root.path()).unwrap();
        for (actual, expected) in contact
            .final_temperatures_k
            .into_iter()
            .zip([283.30, 288.30])
        {
            assert!((actual - expected).abs() < 1e-10);
        }
        for (i, base) in [283.15, 288.15].into_iter().enumerate() {
            let p = &report["projections"][i];
            assert!((p["capacitance_j_k"].as_f64().unwrap() - 0.001).abs() < 1e-14);
            assert!((p["maximum_abs_projection_error_k"].as_f64().unwrap() - 0.25).abs() < 1e-10);
            assert!(
                p["transfer"]["relative_conservation_error"]
                    .as_f64()
                    .unwrap()
                    < 1e-12
            );
            assert_eq!(p["surfaces"].as_array().unwrap().len(), 6);
            assert_eq!(
                p["surfaces"][0]["moisture_risk"]["minimum_surface_temperature_k"],
                base
            );
        }
        assert_eq!(
            report["projections"][1]["destination"]["origin_m"],
            json!([0., 0., 0.00100025])
        );
        let field = root.path().join("stages/thermal-upper/thermal-fields.json");
        let original = fs::read(&field).unwrap();
        let mut changed: Value = serde_json::from_slice(&original).unwrap();
        changed["times"][0]["temperature_k"]["1"] = json!(288.);
        fs::write(&field, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(derive(&spec, &job, root.path()).is_err());
        fs::write(&field, original).unwrap();
        spec.maximum_projection_error_k[1] = 0.1;
        assert!(derive(&spec, &job, root.path()).is_err());
    }

    #[test]
    fn registered_coupling_rejects_self_consistent_replacement_without_changing_sources_or_jobs() {
        let spec: ThermalContactSpec =
            serde_json::from_str(include_str!("../examples/thermal-contact.json")).unwrap();
        let plan = ExecutionPlan::thermal_contact(spec.clone(), "research".into()).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let store = crate::storage::Store::open(&state).unwrap();
        let job = store
            .submit(&plan, "synthetic-unexecuted-coupling-parser")
            .unwrap();
        let root = store.job_dir(&job.id).unwrap();
        source_fixture(&root, &spec.thermal[0], "lower", 283.15);
        source_fixture(&root, &spec.thermal[1], "upper", 288.15);
        for block in ["lower", "upper"] {
            for name in [
                "mesh.json",
                "thermal-fields.json",
                "reference.dat",
                "thermal-receipt.json",
            ] {
                let path = format!("stages/thermal-{block}/{name}");
                store
                    .add_artifact(
                        &job.id,
                        &crate::storage::native_manifest(
                            &root,
                            &path,
                            32 * 1024 * 1024,
                            "synthetic unexecuted parser fixture",
                        )
                        .unwrap(),
                    )
                    .unwrap();
            }
        }
        let (_, report) = derive(&spec, &job.id, &root).unwrap();
        let record = crate::storage::commit_artifact(
            &root,
            "stages/projection/projection-receipt.json",
            &serde_json::to_vec(&report).unwrap(),
            "json",
            "synthetic unexecuted transfer parser fixture",
        )
        .unwrap();
        store.add_artifact(&job.id, &record).unwrap();
        let database = fs::read(state.join("jobs.sqlite3")).unwrap();
        verify_registered(&store, &job.id, &plan).unwrap();
        assert_eq!(fs::read(state.join("jobs.sqlite3")).unwrap(), database);
        let historical = crate::qualification::inspect(&store, &job.id).unwrap();
        assert!(historical.capabilities.iter().all(|c| matches!(
            c.runtime_execution,
            crate::qualification::EvidenceState::NotObserved
        )));
        let field = root.join("stages/thermal-upper/thermal-fields.json");
        let original = fs::read(&field).unwrap();
        fs::write(&field, [original.clone(), b"\n".to_vec()].concat()).unwrap();
        // Same values, valid native DAT and a newly self-consistent receipt do
        // not replace the originally registered exact-source approval.
        let (_, changed) = derive(&spec, &job.id, &root).unwrap();
        fs::write(
            root.join(&record.path),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert!(verify_registered(&store, &job.id, &plan).is_err());
        fs::write(&field, original).unwrap();
        fs::write(
            root.join(&record.path),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();
        verify_registered(&store, &job.id, &plan).unwrap();
        assert_eq!(fs::read(state.join("jobs.sqlite3")).unwrap(), database);
    }
}
