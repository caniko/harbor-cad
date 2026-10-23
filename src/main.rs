use clap::{Parser, Subcommand};
use harbor_cad::{Result, contracts::*, storage::Store, worker};
use std::{io::Write, path::PathBuf};

#[derive(Parser)]
#[command(
    version,
    about = "Local scientific CAD worker; qualification is independent of process success"
)]
struct Cli {
    #[arg(long, global = true, env = "HARBOR_CAD_SOCKET")]
    socket: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    Authority {
        #[command(subcommand)]
        command: Authority,
    },
    Doctor,
    Backend {
        #[command(subcommand)]
        command: Backend,
    },
    Schema {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Worker {
        #[arg(long)]
        state: PathBuf,
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        authority: Option<PathBuf>,
    },
    #[command(hide = true)]
    RunJob {
        #[arg(long)]
        state: PathBuf,
        #[arg(long)]
        profile: PathBuf,
        id: String,
    },
    Case {
        #[command(subcommand)]
        command: Case,
    },
    Cad {
        #[command(subcommand)]
        command: Cad,
    },
    Job {
        #[command(subcommand)]
        command: Job,
    },
    /// Bounded explicit collections over ordinary immutable worker jobs.
    Study {
        #[command(subcommand)]
        command: Study,
    },
    Results {
        #[command(subcommand)]
        command: Results,
    },
    /// Plan presentation from a registered source job; submit its approved plan with job submit.
    Render {
        request: PathBuf,
    },
    /// Plan hardware encoding from an immutable registered rendered-frame job.
    Video {
        request: PathBuf,
    },
    /// Plan HIP point-gradient filtering of registered retained fields.
    Filter {
        request: PathBuf,
    },
    /// Plan static synthetic FEM from an approved registered CAD solid.
    FemImported {
        request: PathBuf,
    },
    Artifact {
        #[command(subcommand)]
        command: Artifact,
    },
    Qualify {
        /// Inspect immutable historical evidence from a specific job.
        #[arg(long)]
        job: Option<String>,
    },
}
#[derive(Subcommand)]
enum Authority {
    Install { file: PathBuf },
}
#[derive(Subcommand)]
enum Backend {
    List,
    HipIdentity { pci: String },
}
#[derive(Subcommand)]
enum Study {
    /// Check complete per-case approvals and the aggregate output allowance.
    Prepare {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
    },
    /// Persist a collection intent and submit each approved case to the worker.
    Submit {
        file: PathBuf,
        #[arg(long)]
        idempotency_key: String,
    },
    Status {
        id: String,
    },
}

#[derive(Subcommand)]
enum Case {
    /// Prepare explicit pinned UV atmospheric angular observations, without executing.
    ValidateAtmosphericReference {
        file: PathBuf,
    },
    /// Plan an immutable native molecular UV solve with all original angular fields.
    PlanAtmosphericReference {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
    },
    /// Plan immutable native directional UV observations and prescribed exposure.
    PlanSpectralReference {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
    },
    /// Prepare a synthetic UV Lambertian reflection reference with independently bounded sensor model error.
    ValidateSpectralReflectionReference {
        file: PathBuf,
    },
    /// Prepare explicit angular UV source, optical weighting and prescribed exposure; no transport execution.
    ValidateSpectralReference {
        file: PathBuf,
    },
    /// Validate prescribed full-face dry snow insulation and quasi-steady applicability; no native execution.
    ValidateSnowReference {
        file: PathBuf,
    },
    /// Measure named planar aperture coverage by explicit prescribed snow prisms.
    ValidateSnowOpenings {
        file: PathBuf,
    },
    /// Plan native transient device conduction with an approval-bound prescribed snow resistance.
    PlanSnowReference {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
    },
    /// Validate SI conduction solidification inputs; no native execution or retained-water transfer.
    ValidateFreezingReference {
        file: PathBuf,
    },
    /// Plan immutable native CPU solidification, independent of retained-water transfer.
    PlanFreezingReference {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
    },
    /// Plan two native thermal histories, conservative projection and static contact.
    PlanThermalContact {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
    },
    /// Plan a native synthetic planar CPU contact/preload and thermal opening reference.
    PlanContactReference {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
    },
    /// Validate explicit SI planar contact/preload inputs and analytical states.
    ValidateContactReference {
        file: PathBuf,
    },
    /// Plan a prescribed synthetic transient CPU thermal history with independent output/solver schedules.
    PlanThermalReference {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
    },
    /// Plan a synthetic wall-centered planar wetting reference with explicit SI properties.
    PlanWettingReference {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
    },
    /// Plan a static synthetic Gmsh/CalculiX CPU reference with explicit SI inputs.
    PlanFemReference {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
    },
    Init,
    Validate {
        file: PathBuf,
    },
    /// Validate a conservative one-way map without running a coupled solver.
    ValidateTransfer {
        file: PathBuf,
    },
    /// Inspect cold-restart units, property ranges, histories and missing inputs.
    ValidateColdRestart {
        file: PathBuf,
    },
    Plan {
        file: PathBuf,
    },
    PlanOpenlbReference {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
    },
    PlanCadInspection {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
        #[arg(long, default_value = "67108864")]
        max_artifact_bytes: u64,
    },
    PlanB1 {
        file: PathBuf,
        #[arg(long)]
        devices: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
    },
}
#[derive(Subcommand)]
enum Job {
    /// Inspect submission-time package/source/policy checks without approval, reservation or execution.
    Preflight {
        plan: PathBuf,
    },
    Submit {
        plan: PathBuf,
        #[arg(long)]
        approve: String,
        #[arg(long)]
        idempotency_key: String,
    },
    Status {
        id: String,
    },
    Logs {
        id: String,
        #[arg(long, default_value = "0")]
        after: u64,
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    Cancel {
        id: String,
    },
}
#[derive(Subcommand)]
enum Cad {
    /// Verify original CAD triangles and explicit region optical properties without solving.
    PrepareSpectralScene { request: PathBuf },
    /// Independently approve direct-only original-CAD spectral power and dose.
    PlanSpectralTransport { request: PathBuf },
    /// Plan a controlled native box-copy edit from an authorized registered CAD job.
    Variant { request: PathBuf },
    /// Plan correspondence meshing from registered authorized BREP; requires the worker.
    Mesh { request: PathBuf },
    /// Prepare an approval-bound sandboxed inspection plan; submit with job submit.
    Inspect {
        file: PathBuf,
        #[arg(long, default_value = "research")]
        policy: String,
        #[arg(long, default_value = "67108864")]
        max_artifact_bytes: u64,
    },
    /// Read registered named-solid metadata from a succeeded bound CAD job.
    Regions { id: String },
    /// Export the complete checksummed CAD job, preserving the approved original.
    Export {
        #[arg(long)]
        state: PathBuf,
        id: String,
        destination: PathBuf,
    },
}
#[derive(Subcommand)]
enum Results {
    /// Read complete original-region facet power/dose at one explicitly retained seed.
    CadOptical {
        request: PathBuf,
    },
    Describe {
        id: String,
    },
    /// Sample up to 64 exact registered static native FEM entity locations.
    Sample {
        request: PathBuf,
    },
    /// Compare registered static samples on the exact same mesh and associations.
    Compare {
        request: PathBuf,
    },
    /// Sample registered thermal nodes at one exact approved retained time.
    SampleThermal {
        request: PathBuf,
    },
    /// Sample complete native freezing fields at an exact retained time and up to 64 (i,j) grid points.
    SampleFreezing {
        request: PathBuf,
    },
    /// Compare same-grid registered freezing samples; signed right minus left.
    CompareFreezing {
        request: PathBuf,
    },
    /// Compare registered thermal samples on the exact same mesh; right minus left.
    CompareThermal {
        request: PathBuf,
    },
    /// Screen one complete native thermal box surface with explicit air inputs.
    Moisture {
        request: PathBuf,
    },
    /// Conservatively project a complete verified native temperature field to an explicit uniform box.
    TransferTemperature {
        request: PathBuf,
    },
    /// Retain original native wetting phase/velocity through explicit conservative nodal extrusion.
    RetainWetting {
        request: PathBuf,
    },
    /// Prepare conserved original-phase cooling enthalpy from explicit thermal inputs without solving.
    PrepareRetainedCooling {
        request: PathBuf,
    },
    /// Plan independently approved native cooling from one exact registered stationary wetting state.
    PlanRetainedCooling {
        request: PathBuf,
    },
    /// Read original cooling subcontrols at an exact approved retained time and region.
    SampleRetainedCooling {
        request: PathBuf,
    },
    /// Prepare source-bound native direct/diffuse angular midpoint transfer, without executing transport.
    TransferAtmosphere {
        request: PathBuf,
    },
    /// Plan CPU spectral transport from succeeded registered atmospheric originals.
    PlanAtmosphericTransport {
        request: PathBuf,
    },
    /// Score measured test series (CSV under the worker input root) against a registered thermal history.
    Correlate {
        request: PathBuf,
    },
}
#[derive(Subcommand)]
enum Artifact {
    List {
        id: String,
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    Export {
        #[arg(long)]
        state: PathBuf,
        id: String,
        destination: PathBuf,
    },
}
fn read<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T> {
    serde_json::from_slice(&worker::read_bounded(path, MAX_MESSAGE)?).map_err(|error| {
        if error.is_data() {
            harbor_cad::contracts::invalid(format!("input schema: {error}"))
        } else {
            error.into()
        }
    })
}
fn print(value: &impl serde::Serialize) -> Result<()> {
    serde_json::to_writer_pretty(std::io::stdout(), value)?;
    std::io::stdout().write_all(b"\n")?;
    Ok(())
}
fn main() {
    if let Err(e) = run(Cli::parse()) {
        let _ = print(&serde_json::json!({"ok":false,"error":e.diagnostic()}));
        std::process::exit(1);
    }
}
fn run(cli: Cli) -> Result<()> {
    let op = match cli.command {
        Commands::Authority {
            command: Authority::Install { file },
        } => {
            let authority: harbor_cad::authority::HostAuthority = read(&file)?;
            let root = harbor_cad::admission::shared_root()?;
            harbor_cad::admission::Admission::install(&root, &authority)?;
            return print(
                &serde_json::json!({"authority_digest":digest(&authority)?,"admission_root":root}),
            );
        }
        Commands::Doctor => {
            return print(&worker::doctor()?);
        }
        Commands::Backend {
            command: Backend::List,
        } => {
            return print(&worker::backends());
        }
        Commands::Backend {
            command: Backend::HipIdentity { pci },
        } => {
            return print(&harbor_cad::devices::HipIdentity::resolve(&pci)?);
        }
        Commands::Qualify { job: Some(job_id) } => Operation::QualificationReport { job_id },
        Commands::Qualify { job: None } => {
            return print(
                &serde_json::json!({"qualified":false,"scope":"no specific immutable job/runtime selected","gates":{"A0":"partial","A1":"analytical-reference-only","B1":"unqualified","B2":"unqualified"},"doctor":worker::doctor()?}),
            );
        }
        Commands::Schema { output } => {
            let schema = serde_json::to_vec_pretty(&schemas())?;
            if let Some(path) = output {
                std::fs::write(path, schema)?;
            } else {
                print(&schemas())?;
            }
            return Ok(());
        }
        Commands::Worker {
            state,
            profile,
            authority,
        } => {
            let socket = cli.socket.unwrap_or_else(|| state.join("worker.sock"));
            return worker::serve_authorized(&state, &socket, &profile, authority.as_deref());
        }
        Commands::RunJob { state, profile, id } => {
            return worker::run_job(&state, &profile, &id);
        }
        Commands::Study { command } => match command {
            Study::Prepare { file, policy } => {
                return print(&harbor_cad::study::prepare(&read(&file)?, &policy)?);
            }
            Study::Submit {
                file,
                idempotency_key,
            } => Operation::SubmitStudy {
                request: Box::new(read(&file)?),
                idempotency_key,
            },
            Study::Status { id } => Operation::StudyStatus { study_id: id },
        },
        Commands::Case { command } => match command {
            Case::PlanThermalReference { file, policy } => {
                let plan = ExecutionPlan::thermal_reference(read(&file)?, policy)?;
                return print(&serde_json::json!({"approval_digest":plan.id()?,"plan":plan}));
            }
            Case::PlanFreezingReference { file, policy } => {
                let plan = ExecutionPlan::freezing_reference(read(&file)?, policy)?;
                return print(&serde_json::json!({"approval_digest":plan.id()?,"plan":plan}));
            }
            Case::PlanWettingReference { file, policy } => {
                let plan = ExecutionPlan::wetting_reference(read(&file)?, policy)?;
                return print(&serde_json::json!({"approval_digest":plan.id()?,"plan":plan}));
            }
            Case::PlanFemReference { file, policy } => {
                let plan = ExecutionPlan::fem_reference(read(&file)?, policy)?;
                return print(&serde_json::json!({"approval_digest":plan.id()?,"plan":plan}));
            }
            Case::Init => {
                return print(&CaseSpec::reference());
            }
            Case::Validate { file } => {
                let case: CaseSpec = read(&file)?;
                case.validate()?;
                return print(&serde_json::json!({"valid":true,"science_id":case.science_id()?}));
            }
            Case::ValidateTransfer { file } => {
                let transfer: harbor_cad::transfers::ConservativeTransfer = read(&file)?;
                transfer.validate()?;
                return print(
                    &serde_json::json!({"valid":true,"transfer_id":transfer.id()?,"source_elements":transfer.source.measures.len(),
                    "destination_elements":transfer.destination.measures.len(),"geometric_correspondence":"requires native mesh verification",
                    "physical_validation":"unqualified","executed":false}),
                );
            }
            Case::Plan { file } => {
                let plan = ExecutionPlan::reference(read(&file)?)?;
                return print(&serde_json::json!({"approval_digest":plan.id()?,"plan":plan}));
            }
            Case::ValidateColdRestart { file } => {
                let case: harbor_cad::recipes::ColdRestartSpec = read(&file)?;
                return print(&case.inspect()?);
            }
            Case::ValidateContactReference { file } => {
                let spec: harbor_cad::contact::ContactReferenceSpec = read(&file)?;
                spec.validate()?;
                return print(&serde_json::json!({"valid":true,"executed":false,
                    "science_id":digest(&spec)?,"preload":spec.reference(1)?,
                    "final":spec.reference(2)?,"physical_validation":"unqualified"}));
            }
            Case::ValidateFreezingReference { file } => {
                let spec: harbor_cad::freezing::FreezingReferenceSpec = read(&file)?;
                return print(&spec.inspect()?);
            }
            Case::ValidateSnowReference { file } => {
                let spec: harbor_cad::snow::SnowReferenceSpec = read(&file)?;
                return print(&spec.prepare()?);
            }
            Case::ValidateSnowOpenings { file } => {
                return print(&harbor_cad::snow_openings::prepare(&read(&file)?)?);
            }
            Case::ValidateSpectralReference { file } => {
                let spec: harbor_cad::radiation::SpectralReferenceSpec = read(&file)?;
                return print(&spec.prepare()?);
            }
            Case::PlanSpectralReference { file, policy } => {
                let spec: harbor_cad::radiation::SpectralReferenceSpec = read(&file)?;
                let plan = ExecutionPlan::spectral_reference(spec, policy)?;
                return print(&serde_json::json!({"approval_digest":plan.id()?,"plan":plan}));
            }
            Case::ValidateAtmosphericReference { file } => {
                let spec: harbor_cad::atmosphere::AtmosphericReferenceSpec = read(&file)?;
                return print(&spec.prepare()?);
            }
            Case::PlanAtmosphericReference { file, policy } => {
                let spec: harbor_cad::atmosphere::AtmosphericReferenceSpec = read(&file)?;
                let reference = spec.prepare()?;
                let plan = ExecutionPlan::atmospheric_reference(spec, policy)?;
                return print(
                    &serde_json::json!({"approval_digest":plan.id()?,"plan":plan,"atmospheric_reference":reference}),
                );
            }
            Case::ValidateSpectralReflectionReference { file } => {
                let spec: harbor_cad::radiation::SpectralReflectionSpec = read(&file)?;
                return print(&spec.prepare()?);
            }
            Case::PlanSnowReference { file, policy } => {
                let spec: harbor_cad::snow::SnowReferenceSpec = read(&file)?;
                return print(&spec.plan(policy)?);
            }
            Case::PlanContactReference { file, policy } => {
                let plan = ExecutionPlan::contact_reference(read(&file)?, policy)?;
                return print(&serde_json::json!({"approval_digest":plan.id()?,"plan":plan}));
            }
            Case::PlanThermalContact { file, policy } => {
                let plan = ExecutionPlan::thermal_contact(read(&file)?, policy)?;
                return print(&serde_json::json!({"approval_digest":plan.id()?,"plan":plan}));
            }
            Case::PlanOpenlbReference { file, policy } => {
                let plan = ExecutionPlan::openlb_reference(read(&file)?, policy)?;
                return print(&serde_json::json!({"approval_digest":plan.id()?,"plan":plan}));
            }
            Case::PlanCadInspection {
                file,
                policy,
                max_artifact_bytes,
            } => {
                let plan = ExecutionPlan::cad_inspection(read(&file)?, policy, max_artifact_bytes)?;
                return print(&serde_json::json!({"approval_digest":plan.id()?,"plan":plan}));
            }
            Case::PlanB1 {
                file,
                devices,
                policy,
            } => {
                let plan = ExecutionPlan::b1(read(&file)?, read(&devices)?, policy)?;
                return print(&serde_json::json!({"approval_digest":plan.id()?,"plan":plan}));
            }
        },
        Commands::FemImported { request } => Operation::PlanFemImported {
            request: Box::new(read(&request)?),
        },
        Commands::Cad { command } => match command {
            Cad::PrepareSpectralScene { request } => Operation::PrepareCadSpectralScene {
                request: Box::new(read(&request)?),
            },
            Cad::PlanSpectralTransport { request } => Operation::PlanCadSpectralTransport {
                request: Box::new(read(&request)?),
            },
            Cad::Mesh { request } => Operation::PlanCadMesh {
                request: Box::new(read(&request)?),
            },
            Cad::Variant { request } => Operation::PlanCadVariant {
                request: Box::new(read(&request)?),
            },
            Cad::Inspect {
                file,
                policy,
                max_artifact_bytes,
            } => {
                let plan = ExecutionPlan::cad_inspection(read(&file)?, policy, max_artifact_bytes)?;
                return print(&serde_json::json!({"approval_digest":plan.id()?,"plan":plan}));
            }
            Cad::Regions { id } => Operation::CadRegions { job_id: id },
            Cad::Export {
                state,
                id,
                destination,
            } => {
                let store = Store::open(&state)?;
                harbor_cad::cad::regions(&store, &id)?;
                let count = store.export(&id, &destination)?;
                return print(
                    &serde_json::json!({"exported":destination,"artifacts":count,"uploads":false}),
                );
            }
        },
        Commands::Job { command } => match command {
            Job::Preflight { plan } => Operation::ExecutionPreflight { plan: read(&plan)? },
            Job::Submit {
                plan,
                approve,
                idempotency_key,
            } => Operation::Submit {
                plan: read(&plan)?,
                approved_digest: approve,
                idempotency_key,
            },
            Job::Status { id } => Operation::Status { job_id: id },
            Job::Logs { id, after, limit } => Operation::Logs {
                job_id: id,
                after,
                limit,
            },
            Job::Cancel { id } => Operation::Cancel { job_id: id },
        },
        Commands::Results { command } => match command {
            Results::CadOptical { request } => Operation::CadOpticalResults {
                request: Box::new(read(&request)?),
            },
            Results::Describe { id } => Operation::Describe { job_id: id },
            Results::Sample { request } => Operation::ResultsSample {
                request: Box::new(read(&request)?),
            },
            Results::Compare { request } => Operation::ResultsCompare {
                request: Box::new(read(&request)?),
            },
            Results::SampleThermal { request } => Operation::ResultsSampleThermal {
                request: Box::new(read(&request)?),
            },
            Results::SampleFreezing { request } => Operation::ResultsSampleFreezing {
                request: Box::new(read(&request)?),
            },
            Results::CompareFreezing { request } => Operation::ResultsCompareFreezing {
                request: Box::new(read(&request)?),
            },
            Results::CompareThermal { request } => Operation::ResultsCompareThermal {
                request: Box::new(read(&request)?),
            },
            Results::Moisture { request } => Operation::ResultsMoisture {
                request: Box::new(read(&request)?),
            },
            Results::TransferTemperature { request } => Operation::ResultsTransferTemperature {
                request: Box::new(read(&request)?),
            },
            Results::RetainWetting { request } => Operation::ResultsRetainWetting {
                request: Box::new(read(&request)?),
            },
            Results::PrepareRetainedCooling { request } => {
                Operation::ResultsPrepareRetainedCooling {
                    request: Box::new(read(&request)?),
                }
            }
            Results::PlanRetainedCooling { request } => Operation::PlanRetainedCooling {
                request: Box::new(read(&request)?),
            },
            Results::SampleRetainedCooling { request } => Operation::ResultsSampleRetainedCooling {
                request: Box::new(read(&request)?),
            },
            Results::TransferAtmosphere { request } => Operation::ResultsTransferAtmosphere {
                request: Box::new(read(&request)?),
            },
            Results::PlanAtmosphericTransport { request } => Operation::PlanAtmosphericTransport {
                request: Box::new(read(&request)?),
            },
            Results::Correlate { request } => Operation::ResultsCorrelate {
                request: Box::new(read(&request)?),
            },
        },
        Commands::Render { request } => Operation::PlanPresentation {
            request: read(&request)?,
        },
        Commands::Video { request } => Operation::PlanVideo {
            request: read(&request)?,
        },
        Commands::Filter { request } => Operation::PlanFilter {
            request: Box::new(read(&request)?),
        },
        Commands::Artifact { command } => match command {
            Artifact::List { id, after, limit } => Operation::Artifacts {
                job_id: id,
                after,
                limit,
            },
            Artifact::Export {
                state,
                id,
                destination,
            } => {
                let store = Store::open(&state)?;
                let count = store.export(&id, &destination)?;
                return print(
                    &serde_json::json!({"exported":destination,"artifacts":count,"uploads":false}),
                );
            }
        },
    };
    let socket = cli
        .socket
        .ok_or_else(|| invalid("--socket or HARBOR_CAD_SOCKET required"))?;
    let response = worker::request(&socket, op)?;
    print(&response)?;
    if !response.ok {
        std::process::exit(1);
    }
    Ok(())
}
