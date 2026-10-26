//! Conservative minima for the currently supported fixed adapters, not measured peaks.
use crate::{Result, contracts::*};

const MIB: u64 = 1024 * 1024;

pub struct StageMinimum {
    pub ram_bytes: u64,
    pub vram_bytes: u64,
}
pub struct MinimumResources {
    pub stages: Vec<StageMinimum>,
    pub artifact_bytes: u64,
}

fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b)
        .ok_or_else(|| invalid("resource estimate overflow"))
}
fn multiply(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b)
        .ok_or_else(|| invalid("resource estimate overflow"))
}
fn lattice_cells(case: &CaseSpec) -> Result<u64> {
    let n = u64::from(case.resolution);
    let nx = (case.length.si("length")? / case.channel_height.si("length")?
        * f64::from(case.resolution))
    .ceil();
    if !nx.is_finite() || nx < 1. || nx >= u64::MAX as f64 {
        return Err(invalid("allocated periodic lattice extent overflow"));
    }
    // The driver allocates a padded cuboid, including non-fluid and halo cells.
    // These extents dominate its nx+2, ny+4, nz+2 VTI fixture.
    multiply(multiply(add(nx as u64, 4)?, add(n, 6)?)?, add(n, 4)?)
}

/// Output allowance every plan with a native stage starts from.
pub const NATIVE_OUTPUT_BYTES: u64 = 16 * MIB;

/// Variant output allowance beyond the copied original document and regions:
/// bounded new FCStd/BREP/STL outputs, plus 64 MiB per sheet region after the first.
pub fn variant_output_bytes(extra_regions: u64) -> Result<u64> {
    add(192 * MIB, multiply(extra_regions, 64 * MIB)?)
}

/// General-solid mesh (or enclosure thermal) output allowance beyond the staged
/// sources, from the requested element budget.
pub fn solid_output_bytes(max_elements: u64, thermal: bool) -> Result<u64> {
    if thermal {
        add(64 * MIB, multiply(max_elements, 1024)?)
    } else {
        add(16 * MIB, multiply(max_elements, 256)?)
    }
}

fn solid_elements(plan: &ExecutionPlan) -> Result<u64> {
    plan.cad_solids
        .as_ref()
        .map(|recipe| recipe.source.geometry.mesh.max_elements)
        .ok_or_else(|| invalid("general-solid resource recipe required"))
}

pub fn minimum(plan: &ExecutionPlan) -> Result<MinimumResources> {
    let pixels = plan.case.as_ref().map_or(Ok(0), |case| {
        multiply(
            u64::from(case.presentation.width),
            u64::from(case.presentation.height),
        )
    })?;
    let snapshots = plan.observation.retained_times_s.len() as u64;
    let native = plan.stages.iter().any(|s| {
        !matches!(
            s.operation,
            StageOperation::ChannelReference | StageOperation::Bundle
        )
    });
    let mut output = if native { NATIVE_OUTPUT_BYTES } else { MIB };
    if let Some(variant) = crate::cad_variant::original(plan)? {
        // Original document/region copies plus bounded closed new FCStd, BREP
        // and STL outputs; native document compression ratios are not assumed.
        let extra = plan
            .cad_sheet_variant
            .as_ref()
            .map_or(0, |s| s.request.regions.len() as u64 - 1);
        output = add(
            output,
            add(
                add(variant.document.bytes, variant.regions.bytes)?,
                variant_output_bytes(extra)?,
            )?,
        )?;
    }
    if let Some(recipe) = &plan.cad_solids {
        output = add(
            output,
            crate::enclosure_thermal::staged_bytes(&recipe.source)?,
        )?;
    }
    if let Some(source) = &plan.cad_source {
        output = add(
            output,
            add(
                source.brep.bytes,
                add(source.manifest.bytes, source.region_evidence.bytes)?,
            )?,
        )?;
    }
    if let Some(source) = &plan.source {
        output = add(output, source.bytes)?;
    }
    if let Some(frames) = &plan.frames {
        output = add(output, frames.bytes)?;
    }
    let mut stages = Vec::new();
    for stage in &plan.stages {
        let mut vram = 0;
        let ram = match stage.operation {
            StageOperation::ChannelReference => {
                output = output.max(multiply(u64::from(plan.channel_case()?.resolution), 128)?);
                add(
                    16 * MIB,
                    multiply(u64::from(plan.channel_case()?.resolution), 160)?,
                )?
            }
            StageOperation::CadFixture
            | StageOperation::CadInspect
            | StageOperation::EnclosureFixture => 1024 * MIB,
            StageOperation::Openlb => {
                let cells = lattice_cells(plan.channel_case()?)?;
                // Two D3Q19 Float64 distribution sets, fields, geometry, halo
                // storage and staging; use the established 2048-byte allowance.
                let ram = add(64 * MIB, multiply(cells, 2048)?)?;
                output = add(32 * MIB, multiply(multiply(cells, 96)?, snapshots)?)?;
                if stage.gpu != GpuRequirement::CpuOnly {
                    vram = ram;
                }
                ram
            }
            StageOperation::Render => {
                let cells = lattice_cells(plan.channel_case()?)?;
                vram = add(add(128 * MIB, multiply(pixels, 32)?)?, multiply(cells, 64)?)?;
                // Bound PNG worst-case staging and reader/display copies.
                output = add(
                    output,
                    multiply(add(multiply(pixels, 8)?, MIB)?, snapshots)?,
                )?;
                (1024 * MIB).max(add(multiply(cells, 256)?, multiply(pixels, 32)?)?)
            }
            StageOperation::Video => {
                vram = add(128 * MIB, multiply(pixels, 16)?)?;
                output = add(
                    output,
                    add(128 * MIB, multiply(multiply(pixels, 4)?, snapshots)?)?,
                )?;
                (512 * MIB).max(multiply(pixels, 16)?)
            }
            StageOperation::NumericalFilter => {
                // The first allowlist is bounded to one million image points,
                // nine Float64 gradient components and all original arrays.
                vram = 1024 * MIB;
                output = add(output, 256 * MIB)?;
                1024 * MIB
            }
            StageOperation::FemReference => {
                let recipe = plan
                    .fem
                    .as_ref()
                    .ok_or_else(|| invalid("FEM recipe required for resource estimate"))?;
                // Sparse fill-in depends on ordering/topology. The initial
                // allowlist caps n<=32 and reserves a full 2 GiB, no GPU claim.
                output = add(
                    output,
                    add(
                        64 * MIB,
                        multiply(u64::from(recipe.resolution).pow(3), 4096)?,
                    )?,
                )?;
                2048 * MIB
            }
            StageOperation::ContactReference => {
                let resolution = if let Some(spec) = &plan.thermal_contact {
                    spec.mechanical.resolution
                } else {
                    plan.contact
                        .as_ref()
                        .ok_or_else(|| invalid("contact resource recipe required"))?
                        .resolution
                };
                output = add(
                    output,
                    add(128 * MIB, multiply(u64::from(resolution).pow(3), 16384)?)?,
                )?;
                2048 * MIB
            }
            StageOperation::FemImported if plan.cad_solids.is_some() => {
                // Mesh, deck and nodal field bytes plus SPOOLES fill-in; per-element
                // allowances are conservative bounds, not measured peaks.
                let elements = solid_elements(plan)?;
                output = add(output, solid_output_bytes(elements, true)?)?;
                add(2048 * MIB, multiply(elements, 16 * 1024)?)?
            }
            StageOperation::CadMesh if plan.cad_solids.is_some() => {
                let elements = solid_elements(plan)?;
                output = add(output, solid_output_bytes(elements, false)?)?;
                add(1024 * MIB, multiply(elements, 8 * 1024)?)?
            }
            StageOperation::FemImported => {
                let spec = plan
                    .imported_fem
                    .as_ref()
                    .ok_or_else(|| invalid("imported FEM resource recipe required"))?;
                output = add(
                    output,
                    add(
                        64 * MIB,
                        multiply(u64::from(spec.reference.resolution).pow(3), 4096)?,
                    )?,
                )?;
                2048 * MIB
            }
            StageOperation::CadMesh => {
                let source = plan
                    .cad_source
                    .as_ref()
                    .ok_or_else(|| invalid("imported CAD mesh source required"))?;
                let n = u64::from(source.geometry.resolution);
                let nodes = multiply(multiply(add(n, 1)?, add(n, 1)?)?, add(n, 1)?)?;
                output = add(output, multiply(nodes, 4096)?)?;
                add(1024 * MIB, multiply(nodes, 65536)?)?
            }
            StageOperation::ThermalReference => {
                let spec = if let Some(coupling) = &plan.thermal_contact {
                    coupling.thermal_stage(&stage.id)?
                } else {
                    plan.thermal
                        .as_ref()
                        .ok_or_else(|| invalid("thermal recipe required for resource estimate"))?
                };
                spec.validate()?;
                output = add(output, 256 * MIB)?;
                2048 * MIB
            }
            StageOperation::ThermalProjection => {
                if plan.thermal_contact.is_none() {
                    return Err(invalid("native coupling projection recipe required"));
                }
                output = add(output, 16 * MIB)?;
                512 * MIB
            }
            StageOperation::WettingReference => {
                let spec = plan
                    .wetting
                    .as_ref()
                    .ok_or_else(|| invalid("wetting resource recipe required"))?;
                spec.validate()?;
                let n = u64::from(spec.resolution);
                let cells = multiply(add(5 * n / 2, 5)?, add(3 * n / 2, 5)?)?;
                // Two Float64 D2Q9 lattices, halos and phase/coupling fields;
                // Python contour verification retains its own closed CSV copy.
                output = add(output, multiply(multiply(cells, 256)?, snapshots)?)?;
                let ram = add(512 * MIB, multiply(cells, 4096)?)?;
                // HIP keeps both lattices, fields and halos device-resident with
                // host mirrors; reuse the host allowance, as for airflow OpenLB.
                if stage.gpu != GpuRequirement::CpuOnly {
                    vram = ram;
                }
                ram
            }
            StageOperation::Bundle => 16 * MIB,
            StageOperation::AtmosphericTransport => {
                let spec = plan.atmospheric_transport.as_ref().ok_or_else(|| {
                    invalid("source-bound atmospheric transport resource recipe required")
                })?;
                spec.validate()?;
                let knots = spec.request.receiver.wavelengths.len() as u64;
                let shard = multiply(
                    multiply(u64::from(spec.request.receiver.samples), knots.div_ceil(4))?,
                    1024,
                )?;
                let angular = multiply(
                    multiply(
                        2 * u64::from(spec.atmosphere.mu_bins),
                        u64::from(spec.atmosphere.phi_bins),
                    )?,
                    add(512, multiply(knots, 64)?)?,
                )?;
                output = add(
                    output,
                    add(
                        multiply(shard, 6)?,
                        add(angular, spec.source.original.bytes)?,
                    )?,
                )?;
                add(512 * MIB, add(multiply(shard, 2)?, multiply(angular, 8)?)?)?
            }
            StageOperation::AtmosphericReference => {
                let spec = plan
                    .atmosphere
                    .as_ref()
                    .ok_or_else(|| invalid("atmospheric resource recipe required"))?;
                spec.prepare()?;
                let columns = add(
                    4,
                    multiply(2 * u64::from(spec.mu_bins), u64::from(spec.phi_bins))?,
                )?;
                let original = multiply(multiply(spec.wavelengths.len() as u64, columns)?, 32)?;
                output = add(output, add(16 * MIB, original)?)?;
                // Native DISORT work/Fourier matrices and closed decimal output,
                // Python decoding and independent original reconstruction copies.
                add(512 * MIB, multiply(original, 8)?)?
            }
            StageOperation::SpectralReference => {
                if let Some(spec) = &plan.cad_transport {
                    spec.validate()?;
                    let facets = spec
                        .scene
                        .regions
                        .iter()
                        .map(|r| r.geometry.triangles as u64)
                        .sum::<u64>();
                    let packets = multiply(
                        multiply(facets, u64::from(spec.request.samples_per_triangle))?,
                        (spec.request.scene.wavelengths.len() as u64).div_ceil(4),
                    )?;
                    let shard = multiply(packets, 1024)?;
                    let source = crate::cad_transport::originals(spec)?
                        .values()
                        .try_fold(0u64, |total, (_, bytes, _)| add(total, *bytes))?;
                    output = add(output, add(multiply(shard, 3)?, add(source, 8 * MIB)?)?)?;
                    stages.push(StageMinimum {
                        ram_bytes: add(512 * MIB, multiply(shard, 2)?)?,
                        vram_bytes: 0,
                    });
                    continue;
                }
                let spec = plan
                    .spectral
                    .as_ref()
                    .ok_or_else(|| invalid("spectral resource recipe required"))?;
                spec.prepare()?;
                // Three seeds retain every native sample/position/cosine and
                // spectral knot. Reserve decimal expansion, independent parsing
                // and complete native/receipt copies.
                let shard = multiply(
                    u64::from(spec.samples),
                    add(256, multiply(spec.wavelengths.len() as u64, 32)?)?,
                )?;
                output = add(output, multiply(shard, 3)?)?;
                add(512 * MIB, multiply(shard, 2)?)?
            }
            StageOperation::FreezingReference if plan.retained_cooling.is_some() => {
                let spec = plan.retained_cooling.as_ref().ok_or_else(|| {
                    invalid("source-bound native cooling resource recipe required")
                })?;
                spec.validate()?;
                let [nx, ny] = spec.prepared.retained.extrusion.source_grid_shape;
                let q = u64::from(spec.request.spatial_refinement);
                let cells = multiply(
                    add(multiply(nx as u64, q)?, 4)?,
                    add(multiply((ny - 2) as u64, q)?, 6)?,
                )?;
                output = add(output, multiply(multiply(cells, 512)?, snapshots)?)?;
                output = add(output, multiply(spec.request.native_steps(), 128)?)?;
                for record in &spec.originals {
                    output = add(output, record.bytes)?;
                }
                // Both native lattices and halo/coupling storage, independent
                // complete-field CSV/XML copies and the per-step boundary ledger.
                add(
                    add(512 * MIB, multiply(cells, 4096)?)?,
                    multiply(spec.request.native_steps(), 256)?,
                )?
            }
            StageOperation::FreezingReference => {
                let spec = plan
                    .freezing
                    .as_ref()
                    .ok_or_else(|| invalid("freezing resource recipe required"))?;
                spec.scale()?;
                let n = u64::from(spec.resolution);
                let cells = multiply(add(n, 5)?, add(n / 8, 4)?)?;
                // D2Q5 enthalpy and frozen D2Q9 phase-coupling lattice, both
                // Float64, complete halos/fields and independent CSV/XML copies.
                output = add(
                    output,
                    add(
                        multiply(spec.steps, 128)?,
                        multiply(multiply(cells, 512)?, snapshots)?,
                    )?,
                )?;
                add(512 * MIB, multiply(cells, 4096)?)?
            }
        };
        stages.push(StageMinimum {
            ram_bytes: ram,
            vram_bytes: vram,
        });
    }
    if output > 1_000_000_000_000
        || stages
            .iter()
            .any(|s| s.ram_bytes > i64::MAX as u64 || s.vram_bytes > i64::MAX as u64)
    {
        return Err(invalid(
            "resource estimate outside bounded adapter capacity",
        ));
    }
    Ok(MinimumResources {
        stages,
        artifact_bytes: output,
    })
}

impl MinimumResources {
    pub fn validate(&self, plan: &ExecutionPlan) -> Result<()> {
        for (stage, required) in plan.stages.iter().zip(&self.stages) {
            if stage.ram_bytes < required.ram_bytes || stage.vram_bytes < required.vram_bytes {
                return Err(invalid(format!(
                    "stage {} underestimates operation RAM/VRAM: at least {}/{} bytes required; parameters preserved",
                    stage.id, required.ram_bytes, required.vram_bytes
                )));
            }
        }
        if plan.observation.max_artifact_bytes < self.artifact_bytes {
            return Err(invalid(format!(
                "scientific/frame output estimate requires at least {} bytes for every approved retained time; parameters preserved",
                self.artifact_bytes
            )));
        }
        Ok(())
    }
    pub fn apply(&self, plan: &mut ExecutionPlan) {
        for (stage, required) in plan.stages.iter_mut().zip(&self.stages) {
            stage.ram_bytes = stage.ram_bytes.max(required.ram_bytes);
            stage.vram_bytes = stage.vram_bytes.max(required.vram_bytes);
        }
        plan.observation.max_artifact_bytes =
            plan.observation.max_artifact_bytes.max(self.artifact_bytes);
    }
}
