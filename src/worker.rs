use crate::{
    Error, Result,
    admission::Admission,
    authority::{ExecutionAuthorization, HostAuthority},
    contracts::*,
    devices,
    execution::{ExecutionBinding, packaged_file},
    lifecycle::NativeProcess,
    resources, science,
    storage::*,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
        process::CommandExt,
    },
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize, Debug)]
pub struct Response {
    pub protocol_version: u32,
    pub request_id: String,
    pub ok: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
}
/// Validate the bounded envelope before the operation so schema rejections can
/// retain their request identity. Raw JSON preserves duplicate-field rejection.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IncomingEnvelope<'a> {
    protocol_version: u32,
    request_id: String,
    #[serde(borrow)]
    request: &'a serde_json::value::RawValue,
}
fn reply(request_id: String, result: Result<serde_json::Value>) -> Response {
    match result {
        Ok(data) => Response {
            protocol_version: 1,
            request_id,
            ok: true,
            data: Some(data),
            error: None,
        },
        Err(error) => Response {
            protocol_version: 1,
            request_id,
            ok: false,
            data: None,
            error: Some(serde_json::to_value(error.diagnostic()).unwrap_or_default()),
        },
    }
}
pub fn load_profile(path: &Path) -> Result<HostExecutionProfile> {
    let profile: HostExecutionProfile = serde_json::from_slice(&read_bounded(path, MAX_MESSAGE)?)?;
    if profile.schema_version != 1
        || profile.threads == 0
        || profile.threads > 128
        || profile.max_ram_bytes == 0
        || profile.max_ram_bytes > i64::MAX as u64
        || profile.max_disk_bytes == 0
        || profile.timeout_seconds == 0
        || profile.timeout_seconds > 86400
        || !["systemd", "foreground"].contains(&profile.service_mode.as_str())
        || !["ci", "prototype", "production", "research"].contains(&profile.policy.as_str())
    {
        return Err(invalid("host profile limits/version/policy"));
    }
    if profile.service_mode == "foreground" && profile.policy != "ci" {
        return Err(invalid("foreground is explicitly weaker CI-only execution"));
    }
    if !Path::new(&profile.allowed_input_root).is_absolute() {
        return Err(invalid("absolute input root required"));
    }
    Ok(profile)
}
pub fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    fs::File::open(path)?
        .take(maximum + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > maximum {
        return Err(invalid("file/request size limit"));
    }
    Ok(data)
}
fn systemctl(args: &[&str]) -> Result<std::process::Output> {
    Ok(
        Command::new(option_env!("HARBOR_CAD_SYSTEMCTL").unwrap_or("systemctl"))
            .arg("--user")
            .args(args)
            .stdin(Stdio::null())
            .output()?,
    )
}
fn unit_info(unit: &str) -> Result<BTreeMap<String, String>> {
    let output = systemctl(&[
        "show",
        unit,
        "--property=LoadState,ActiveState,InvocationID,ExecMainStatus,Result,MainPID,ControlGroup",
    ])?;
    if !output.status.success() {
        return Err(Error::Resource("systemd user manager unavailable".into()));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.into(), v.into())))
        .collect())
}
fn check_plan(plan: &ExecutionPlan, profile: &HostExecutionProfile) -> Result<()> {
    plan.validate()?;
    if plan.policy != profile.policy {
        return Err(invalid("plan policy differs from effective host profile"));
    }
    if plan.peak_ram() > profile.max_ram_bytes || plan.disk_reservation()? > profile.max_disk_bytes
    {
        return Err(Error::Resource(
            "approved plan exceeds host RAM/disk admission budget; scientific parameters preserved"
                .into(),
        ));
    }
    for stage in &plan.stages {
        if let Some(selection) = &stage.selection {
            // HIP is independently correlated through PCI/DRM/KFD; a Mesa selector is not CUDA identity.
            let inventory = devices::inventory()?;
            let resolved = devices::resolve(
                &inventory,
                selection.role,
                &selection.backend,
                Some(&selection.pci),
            )?;
            if selection.backend == "hip" && resolved.backend_uuid != selection.backend_uuid {
                return Err(invalid(
                    "approved HIP UUID differs from exact PCI/KFD identity",
                ));
            }
        }
        if !matches!(
            stage.operation,
            StageOperation::ChannelReference
                | StageOperation::Bundle
                | StageOperation::ThermalProjection
        ) {
            if profile.service_mode != "systemd" {
                return Err(Error::Unqualified(
                    "native stages require systemd cgroup ownership".into(),
                ));
            }
            let runtime = profile.native_runtime.as_ref().ok_or_else(|| {
                Error::Unqualified("isolated native runtime not configured".into())
            })?;
            let native = NativeRuntime::load(Path::new(runtime))?;
            let executable = native.executable_for(&stage.operation, plan)?;
            if matches!(
                stage.operation,
                StageOperation::CadInspect | StageOperation::CadFixture
            ) {
                crate::sandbox::importer_mounts(native.importer_closure()?, Path::new(executable))?;
            }
            if matches!(stage.operation, StageOperation::Openlb) {
                let backend = stage
                    .selection
                    .as_ref()
                    .map_or("cpu", |s| s.backend.as_str());
                if native.openlb_backend != backend {
                    return Err(Error::Unqualified(
                        "packaged OpenLB backend differs from approved stage; no fallback".into(),
                    ));
                }
            }
            if matches!(
                stage.operation,
                StageOperation::FemReference
                    | StageOperation::ThermalReference
                    | StageOperation::WettingReference
                    | StageOperation::FreezingReference
                    | StageOperation::SpectralReference
                    | StageOperation::AtmosphericReference
                    | StageOperation::AtmosphericTransport
                    | StageOperation::ContactReference
                    | StageOperation::CadMesh
                    | StageOperation::FemImported
            ) {
                crate::sandbox::importer_mounts(
                    native.solver_closure_for(&stage.operation, plan)?,
                    Path::new(executable),
                )?;
            }
        }
    }
    Ok(())
}
fn launch(
    store: &Store,
    capacity: &HostExecutionProfile,
    id: &str,
    admission: Option<&Admission>,
) -> Result<()> {
    let plan = store.plan(id)?;
    let profile = store.job_profile(id)?;
    check_plan(&plan, &profile)?;
    if let Some(authorization) = store.execution_authorization(id)? {
        authorization.verify(&plan, &profile, &store.execution_binding(id)?)?;
        let admission = admission
            .ok_or_else(|| invalid("authorized job requires its same-user admission policy"))?;
        if !admission.reserve(store, id)? {
            return Ok(());
        }
    } else if admission.is_some() {
        return Err(Error::Unqualified(
            "historical job has no authority binding; no automatic policy upgrade".into(),
        ));
    }
    let artifacts = safe_path(&store.root, "artifacts")?;
    let retained_disk = if artifacts.exists() {
        disk_bytes(&artifacts, false)?
    } else {
        0
    };
    if !store.try_start(id, capacity, retained_disk)? {
        return Ok(());
    }
    let binding = store.execution_binding(id)?;
    binding.verify(&plan, &profile)?;
    crate::retention::verify_ready(&store.root, id, &binding, profile.service_mode == "systemd")?;
    let exe = &binding.runner.path;
    let root = fs::canonicalize(&store.root)?;
    let profiles = safe_path(&root, "profiles")?;
    private_dir(&profiles)?;
    let name = format!("{id}.json");
    commit_artifact(
        &profiles,
        &name,
        &serde_json::to_vec(&profile)?,
        "json",
        "immutable effective job profile",
    )?;
    let profile_path = profiles.join(name);
    if profile.service_mode == "foreground" {
        let mut cmd = Command::new(exe);
        cmd.args(["run-job", "--state"])
            .arg(&root)
            .arg("--profile")
            .arg(&profile_path)
            .arg(id)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // No native grandchildren in CI foreground mode. Do not reconcile by saved PIDs.
        unsafe {
            cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = cmd.spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    } else {
        let job = store.job(id)?;
        let mut launcher =
            Command::new(option_env!("HARBOR_CAD_SYSTEMD_RUN").unwrap_or("systemd-run"));
        launcher
            .args(["--user", "--quiet", "--service-type=exec", "--unit"])
            .arg(&job.unit)
            .arg("--property=KillMode=control-group")
            .arg("--property=TimeoutStopSec=10s")
            .arg("--property=SendSIGKILL=yes")
            .arg("--property=Restart=no")
            .arg(format!("--property=MemoryMax={}", plan.peak_ram()))
            .arg("--property=MemorySwapMax=0")
            .arg("--property=TasksMax=128")
            .arg(format!(
                "--property=RuntimeMaxSec={}",
                profile.timeout_seconds
            ))
            .arg("--property=NoNewPrivileges=yes")
            .arg("--property=UMask=0077")
            .arg(format!("--property=CPUQuota={}00%", profile.threads));
        if std::env::var_os("HARBOR_CAD_IMPORT_PROBE_ROOT").is_some() {
            for name in [
                "HARBOR_CAD_IMPORT_PROBE_ROOT",
                "HARBOR_CAD_IMPORT_PROBE_PORT",
                "HARBOR_CAD_CREDENTIAL_SENTINEL",
            ] {
                if let Some(value) = std::env::var_os(name) {
                    let mut argument = std::ffi::OsString::from(format!("--setenv={name}="));
                    argument.push(value);
                    launcher.arg(argument);
                }
            }
        }
        let output = launcher
            .arg(exe)
            .arg("run-job")
            .arg("--state")
            .arg(root)
            .arg("--profile")
            .arg(profile_path)
            .arg(id)
            .stdin(Stdio::null())
            .output()?;
        if !output.status.success() {
            store.finish(id, 1, Some("systemd launch failed"))?;
            return Err(Error::Resource(
                String::from_utf8_lossy(&output.stderr).trim().into(),
            ));
        }
    }
    store.event(id, "launched", &profile.service_mode)?;
    Ok(())
}
fn reconcile(
    store: &Store,
    capacity: &HostExecutionProfile,
    startup: bool,
    admission: Option<&Admission>,
) -> Result<()> {
    if let Some(admission) = admission {
        admission.reconcile(|store, job| {
            if store.job_profile(&job.id)?.service_mode == "foreground" {
                return Ok(false);
            }
            owned_service_closed(store, job, &unit_info(&job.unit)?)
        })?;
    }
    for id in store.active()? {
        let job = store.job(&id)?;
        if job.state == "queued" {
            if let Err(e) = launch(store, capacity, &id, admission) {
                store.transition(&id, "queued", "failed", Some(&e.to_string()))?;
                store.transition(&id, "starting", "failed", Some(&e.to_string()))?;
                store.event(&id, "launch_rejected", &e.to_string())?;
            }
        } else if store.job_profile(&id)?.service_mode == "foreground" {
            if startup {
                store.transition(
                    &id,
                    &job.state,
                    "interrupted",
                    Some("CI foreground worker restart; no solver resume guarantee"),
                )?;
            }
        } else {
            let info = unit_info(&job.unit)?;
            let observed = info.get("InvocationID").map(String::as_str).unwrap_or("");
            if job
                .invocation_id
                .as_deref()
                .is_some_and(|saved| !observed.is_empty() && observed != saved)
            {
                store.transition(
                    &id,
                    &job.state,
                    "interrupted",
                    Some("unit invocation identity changed; refusing attachment/cancellation"),
                )?;
            } else if owned_service_closed(store, &job, &info)? {
                let code = info
                    .get("ExecMainStatus")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                let execution = if job.state == "cancelling" {
                    "cancelled"
                } else {
                    "failed"
                };
                let reason = closed_service_reason(
                    store,
                    &id,
                    execution,
                    "job ended without durable successful exit receipt",
                )?;
                store.finish(&id, if code == 0 { 1 } else { code }, Some(&reason))?;
            }
        }
    }
    store.cleanup_retention(|job| {
        if store.job_profile(&job.id)?.service_mode == "foreground" {
            return Ok(true);
        }
        owned_service_closed(store, job, &unit_info(&job.unit)?)
    })?;
    Ok(())
}
fn owned_service_closed(store: &Store, job: &Job, info: &BTreeMap<String, String>) -> Result<bool> {
    let missing = info.get("LoadState").map(String::as_str) == Some("not-found");
    if !missing
        && (!info
            .get("ActiveState")
            .is_some_and(|s| s == "inactive" || s == "failed")
            || job.invocation_id.as_ref().is_some_and(|id| {
                info.get("InvocationID")
                    .is_some_and(|observed| !observed.is_empty() && observed != id)
            }))
    {
        return Ok(false);
    }
    let mut groups: Vec<String> = info
        .get("ControlGroup")
        .filter(|g| !g.is_empty())
        .cloned()
        .into_iter()
        .collect();
    let owner = safe_path(
        &store.root,
        &format!("artifacts/{}/service-owner.json", job.id),
    )?;
    if owner.exists() {
        let owner = read_native_receipt(&owner)?;
        if owner["unit"] != job.unit
            || job
                .invocation_id
                .as_ref()
                .is_some_and(|id| owner["invocation"] != *id)
        {
            return Err(invalid("persisted service owner differs from job identity"));
        }
        groups.push(
            owner["control_group"]
                .as_str()
                .ok_or_else(|| invalid("recorded service cgroup missing"))?
                .into(),
        );
    }
    for group in groups {
        let relative = group
            .strip_prefix('/')
            .ok_or_else(|| invalid("absolute service cgroup required"))?;
        let path = safe_path(Path::new("/sys/fs/cgroup"), relative)?;
        match fs::read_to_string(path.join("cgroup.events")) {
            Ok(events) if events.lines().any(|l| l == "populated 0") => {}
            Ok(_) => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(true)
}
fn submission_binding(
    store: &Store,
    profile: &HostExecutionProfile,
    authority: Option<&HostAuthority>,
    plan: &ExecutionPlan,
) -> Result<ExecutionBinding> {
    if plan.source.is_some() && authority.is_none() {
        return Err(Error::Unqualified(
            "standalone presentation requires authoritative admission".into(),
        ));
    }
    if (plan.fem.is_some()
        || plan.retained_cooling.is_some()
        || plan.cad_transport.is_some()
        || plan.cad_variant.is_some()
        || plan.thermal.is_some()
        || plan.cad_source.is_some()
        || plan.wetting.is_some()
        || plan.freezing.is_some()
        || plan.spectral.is_some()
        || plan.atmosphere.is_some()
        || plan.atmospheric_transport.is_some()
        || plan.thermal_contact.is_some()
        || plan.contact.is_some())
        && authority.is_none()
    {
        return Err(Error::Unqualified(
            "CPU native recipe submission requires authoritative same-user admission".into(),
        ));
    }
    if authority.is_none()
        && plan.stages.iter().any(|stage| {
            stage
                .selection
                .as_ref()
                .is_some_and(|g| g.role == Role::Compute && g.backend == "hip")
        })
    {
        return Err(Error::Unqualified(
            "HIP submission requires authoritative same-user admission".into(),
        ));
    }
    check_plan(plan, profile)?;
    if let Some(spec) = &plan.retained_cooling {
        crate::cooling_execution::source(store, spec)?;
    }
    if let Some(spec) = &plan.cad_transport {
        crate::cad_transport::source(store, spec)?;
    }
    if let Some(spec) = &plan.atmospheric_transport {
        crate::atmospheric_transport::registered_source(store, spec)?;
    }
    if let Some(spec) = &plan.cad_variant {
        crate::cad_variant::source(store, spec)?;
    }
    if let Some(authority) = authority {
        authority.authorize(plan, profile)?;
    }
    let mut files = BTreeMap::new();
    if let Some(path) = &profile.native_runtime {
        let runtime = NativeRuntime::load(Path::new(path))?;
        files.insert("bwrap".into(), runtime.bwrap.clone());
        for stage in &plan.stages {
            if matches!(
                stage.operation,
                StageOperation::Bundle
                    | StageOperation::ChannelReference
                    | StageOperation::ThermalProjection
            ) {
                continue;
            }
            files.insert(
                serde_json::to_string(&stage.operation)?,
                runtime.executable_for(&stage.operation, plan)?.into(),
            );
            if matches!(
                stage.operation,
                StageOperation::CadInspect | StageOperation::CadFixture
            ) {
                files.insert(
                    "cad_closure".into(),
                    runtime
                        .importer_closure()?
                        .to_str()
                        .ok_or_else(|| invalid("importer closure path"))?
                        .into(),
                );
            }
            let closure_key = match stage.operation {
                StageOperation::ContactReference => "contact_closure",
                StageOperation::WettingReference => "wetting_closure",
                StageOperation::FreezingReference if plan.retained_cooling.is_some() => {
                    "retained_cooling_closure"
                }
                StageOperation::FreezingReference => "freezing_closure",
                StageOperation::SpectralReference => "spectral_closure",
                StageOperation::AtmosphericReference => "atmosphere_closure",
                StageOperation::AtmosphericTransport => "atmospheric_spectral_closure",
                StageOperation::FemImported => "fem_imported_closure",
                StageOperation::CadMesh => "cad_mesh_closure",
                StageOperation::ThermalReference => "thermal_closure",
                StageOperation::FemReference => "fem_closure",
                _ => continue,
            };
            files.insert(
                closure_key.into(),
                runtime
                    .solver_closure_for(&stage.operation, plan)?
                    .to_str()
                    .ok_or_else(|| invalid("FEM closure path"))?
                    .into(),
            );
        }
    }
    ExecutionBinding::capture(plan, profile, &std::env::current_exe()?, files)
}

fn dispatch(
    store: &Store,
    profile: &HostExecutionProfile,
    authority: Option<&HostAuthority>,
    op: Operation,
) -> Result<serde_json::Value> {
    match op {
        Operation::PrepareStudy { request } => Ok(serde_json::to_value(crate::study::prepare(
            &request,
            &profile.policy,
        )?)?),
        Operation::StudyStatus { study_id } => Ok(serde_json::to_value(crate::study::status(
            store, &study_id,
        )?)?),
        Operation::SubmitStudy {
            request,
            idempotency_key,
        } => {
            crate::study::prepare(&request, &profile.policy)?;
            let bindings = request
                .cases
                .iter()
                .map(|case| {
                    let binding = submission_binding(store, profile, authority, &case.plan)?;
                    let authorization = authority
                        .map(|authority| {
                            ExecutionAuthorization::capture(
                                &case.plan, profile, &binding, authority,
                            )
                        })
                        .transpose()?;
                    Ok((binding, authorization))
                })
                .collect::<Result<Vec<_>>>()?;
            let intent = crate::study::retain_intent(
                store,
                &request,
                &idempotency_key,
                profile,
                &digest(&bindings)?,
            )?;
            for ((case, key), (binding, authorization)) in
                request.cases.iter().zip(&intent.child_keys).zip(bindings)
            {
                if let Some(authorization) = authorization {
                    store.submit_authorized(&case.plan, key, profile, &binding, &authorization)?;
                } else {
                    store.submit_for_execution(&case.plan, key, profile, &binding)?;
                }
            }
            Ok(serde_json::to_value(crate::study::status(
                store, &intent.id,
            )?)?)
        }
        Operation::Doctor {} => doctor(),
        Operation::BackendList {} => Ok(backends()),
        Operation::ExecutionPreflight { plan } => {
            let execution_binding = submission_binding(store, profile, authority, &plan)?;
            Ok(serde_json::to_value(
                crate::execution::ExecutionPreflight {
                    schema_version: 1,
                    execution_binding,
                    peak_ram_bytes: plan.peak_ram(),
                    disk_reservation_bytes: plan.disk_reservation()?,
                    execution_approved: false,
                    resources_reserved: false,
                    executed: false,
                    qualification: "not_assessed".into(),
                },
            )?)
        }
        Operation::Validate { case } => {
            case.validate()?;
            Ok(
                serde_json::json!({"valid":true,"science_id":case.science_id()?,"physical_validation":"unqualified"}),
            )
        }
        Operation::Plan { case } => Ok(serde_json::to_value(ExecutionPlan::reference(*case)?)?),
        Operation::PlanFemReference { spec } => {
            let plan = ExecutionPlan::fem_reference(*spec, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::PlanThermalReference { spec } => {
            let plan = ExecutionPlan::thermal_reference(*spec, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::ValidateSnowReference { spec } => Ok(serde_json::to_value(spec.prepare()?)?),
        Operation::ValidateSnowOpenings { spec } => {
            Ok(serde_json::to_value(crate::snow_openings::prepare(&spec)?)?)
        }
        Operation::ValidateSpectralReference { spec } => Ok(serde_json::to_value(spec.prepare()?)?),
        Operation::PlanSpectralReference { spec } => {
            let plan = ExecutionPlan::spectral_reference(*spec, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::ValidateAtmosphericReference { spec } => {
            Ok(serde_json::to_value(spec.prepare()?)?)
        }
        Operation::PlanAtmosphericReference { spec } => {
            let reference = spec.prepare()?;
            let plan = ExecutionPlan::atmospheric_reference(*spec, profile.policy.clone())?;
            Ok(
                serde_json::json!({"approval_digest":plan.id()?,"plan":plan,"atmospheric_reference":reference}),
            )
        }
        Operation::ValidateSpectralReflectionReference { spec } => {
            Ok(serde_json::to_value(spec.prepare()?)?)
        }
        Operation::PlanAtmosphericTransport { request } => {
            request.validate()?;
            if authority.is_none() {
                return Err(Error::Unqualified(
                    "registered atmospheric transport requires authoritative admission".into(),
                ));
            }
            let plan = crate::atmospheric_transport::plan(store, *request, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::PlanSnowReference { spec } => spec.plan(profile.policy.clone()),
        Operation::PlanWettingReference { spec } => {
            let plan = ExecutionPlan::wetting_reference(*spec, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::PlanContactReference { spec } => {
            let plan = ExecutionPlan::contact_reference(*spec, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::PlanThermalContact { spec } => {
            let plan = ExecutionPlan::thermal_contact(*spec, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::ValidateFreezingReference { spec } => spec.inspect(),
        Operation::PlanFreezingReference { spec } => {
            let plan = ExecutionPlan::freezing_reference(*spec, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::PlanRetainedCooling { request } => {
            request.validate()?;
            if authority.is_none() {
                return Err(Error::Unqualified(
                    "registered native cooling planning requires authoritative same-user admission"
                        .into(),
                ));
            }
            let spec = crate::cooling_execution::resolve(store, *request)?;
            let plan = ExecutionPlan::retained_cooling(spec, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::ResultsSampleRetainedCooling { request } => {
            crate::cooling_results::sample(store, &request)
        }
        Operation::PlanOpenlbReference { case } => Ok(serde_json::to_value(
            ExecutionPlan::openlb_reference(*case, profile.policy.clone())?,
        )?),
        Operation::PlanCadInspection {
            case,
            max_artifact_bytes,
        } => Ok(serde_json::to_value(ExecutionPlan::cad_inspection(
            *case,
            profile.policy.clone(),
            max_artifact_bytes,
        )?)?),
        Operation::PlanB1 { case, selections } => Ok(serde_json::to_value(ExecutionPlan::b1(
            *case,
            selections,
            profile.policy.clone(),
        )?)?),
        Operation::PlanCadMesh { request } => {
            if authority.is_none() {
                return Err(Error::Unqualified(
                    "imported CAD mesh requires authoritative admission".into(),
                ));
            }
            let plan = crate::cad_source::plan(store, *request, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::PlanCadVariant { request } => {
            if authority.is_none() {
                return Err(Error::Unqualified(
                    "controlled CAD variants require authoritative admission".into(),
                ));
            }
            let plan = crate::cad_variant::plan(store, *request, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::PlanFemImported { request } => {
            if authority.is_none() {
                return Err(Error::Unqualified(
                    "imported FEM requires authoritative admission".into(),
                ));
            }
            let plan = crate::fem_imported::plan(store, *request, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::PlanCadSpectralTransport { request } => {
            if authority.is_none() {
                return Err(Error::Unqualified(
                    "CAD optical transport requires authoritative same-user admission".into(),
                ));
            }
            let spec = crate::cad_transport::resolve(store, *request)?;
            let plan = ExecutionPlan::cad_spectral_transport(spec, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::CadOpticalResults { request } => Ok(serde_json::to_value(
            crate::cad_transport_results::inspect(store, *request)?,
        )?),
        Operation::PlanPresentation { request } => {
            if authority.is_none() {
                return Err(Error::Unqualified(
                    "standalone presentation requires authoritative admission".into(),
                ));
            }
            let plan = crate::presentation::plan(store, *request, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::ValidateColdRestart { case } => case.inspect(),
        Operation::PlanVideo { request } => {
            if authority.is_none() {
                return Err(Error::Unqualified(
                    "independent video requires authoritative admission".into(),
                ));
            }
            let plan = crate::frames::plan(store, *request, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::PlanFilter { request } => {
            let plan = crate::filters::plan(store, *request, profile.policy.clone())?;
            Ok(serde_json::json!({"approval_digest":plan.id()?,"plan":plan}))
        }
        Operation::Submit {
            plan,
            approved_digest,
            idempotency_key,
        } => {
            if plan.id()? != approved_digest {
                return Err(invalid("immutable plan approval digest mismatch"));
            }
            if let Some(job) = store.existing_submission(&plan, &idempotency_key, profile)? {
                return Ok(serde_json::to_value(job)?);
            }
            let binding = submission_binding(store, profile, authority, &plan)?;
            let job = if let Some(authority) = authority {
                let authorization =
                    ExecutionAuthorization::capture(&plan, profile, &binding, authority)?;
                let submit = || {
                    store.submit_authorized(
                        &plan,
                        &idempotency_key,
                        profile,
                        &binding,
                        &authorization,
                    )
                };
                if let Some(source) = &plan.source {
                    let bytes = source
                        .bytes
                        .checked_add(plan.frames.as_ref().map_or(0, |f| f.bytes))
                        .ok_or_else(|| invalid("frame staging budget overflow"))?
                        .checked_add(16 * 1024 * 1024)
                        .ok_or_else(|| invalid("source staging budget overflow"))?;
                    if bytes
                        .checked_add(disk_bytes(&store.root, false)?)
                        .is_none_or(|n| n > profile.max_disk_bytes)
                    {
                        return Err(Error::Resource(
                            "insufficient state-root capacity for immutable source staging".into(),
                        ));
                    }
                    Admission::open(&crate::admission::shared_root()?, authority)?
                        .retain_inputs(store, bytes, submit)?
                } else if let Some(spec) = &plan.cad_variant {
                    let bytes = spec
                        .document
                        .bytes
                        .checked_add(spec.source.region_evidence.bytes)
                        .and_then(|b| b.checked_add(16 * 1024 * 1024))
                        .ok_or_else(|| invalid("CAD variant staging budget overflow"))?;
                    if bytes
                        .checked_add(disk_bytes(&store.root, false)?)
                        .is_none_or(|n| n > profile.max_disk_bytes)
                    {
                        return Err(Error::Resource(
                            "insufficient state-root capacity for immutable CAD variant staging"
                                .into(),
                        ));
                    }
                    Admission::open(&crate::admission::shared_root()?, authority)?
                        .retain_inputs(store, bytes, submit)?
                } else if let Some(source) = &plan.cad_source {
                    let bytes = source
                        .brep
                        .bytes
                        .checked_add(source.manifest.bytes)
                        .and_then(|b| b.checked_add(source.region_evidence.bytes))
                        .and_then(|b| b.checked_add(16 * 1024 * 1024))
                        .ok_or_else(|| invalid("CAD source staging budget overflow"))?;
                    if bytes
                        .checked_add(disk_bytes(&store.root, false)?)
                        .is_none_or(|n| n > profile.max_disk_bytes)
                    {
                        return Err(Error::Resource(
                            "insufficient state-root capacity for immutable CAD source staging"
                                .into(),
                        ));
                    }
                    Admission::open(&crate::admission::shared_root()?, authority)?
                        .retain_inputs(store, bytes, submit)?
                } else if let Some(spec) = &plan.cad_transport {
                    let bytes = crate::cad_transport::originals(spec)?.values().try_fold(
                        16 * 1024 * 1024u64,
                        |sum, (_, bytes, _)| {
                            sum.checked_add(*bytes)
                                .ok_or_else(|| invalid("CAD optical input budget overflow"))
                        },
                    )?;
                    if bytes
                        .checked_add(disk_bytes(&store.root, false)?)
                        .is_none_or(|n| n > profile.max_disk_bytes)
                    {
                        return Err(Error::Resource(
                            "insufficient state-root capacity for immutable CAD optical inputs"
                                .into(),
                        ));
                    }
                    Admission::open(&crate::admission::shared_root()?, authority)?
                        .retain_inputs(store, bytes, submit)?
                } else if let Some(spec) = &plan.retained_cooling {
                    let bytes = spec
                        .originals
                        .iter()
                        .try_fold(16 * 1024 * 1024u64, |sum, r| {
                            sum.checked_add(r.bytes).ok_or_else(|| {
                                invalid("original cooling input staging budget overflow")
                            })
                        })?;
                    if bytes
                        .checked_add(disk_bytes(&store.root, false)?)
                        .is_none_or(|n| n > profile.max_disk_bytes)
                    {
                        return Err(Error::Resource("insufficient state-root capacity for complete immutable cooling history".into()));
                    }
                    Admission::open(&crate::admission::shared_root()?, authority)?
                        .retain_inputs(store, bytes, submit)?
                } else if let Some(spec) = &plan.atmospheric_transport {
                    let bytes = spec
                        .source
                        .original
                        .bytes
                        .checked_add(spec.source.receipt.bytes)
                        .and_then(|b| b.checked_add(16 * 1024 * 1024))
                        .ok_or_else(|| invalid("atmospheric staging budget overflow"))?;
                    if bytes
                        .checked_add(disk_bytes(&store.root, false)?)
                        .is_none_or(|n| n > profile.max_disk_bytes)
                    {
                        return Err(Error::Resource("insufficient state-root capacity for immutable atmospheric source staging".into()));
                    }
                    Admission::open(&crate::admission::shared_root()?, authority)?
                        .retain_inputs(store, bytes, submit)?
                } else {
                    submit()?
                }
            } else {
                store.submit_for_execution(&plan, &idempotency_key, profile, &binding)?
            };
            Ok(serde_json::to_value(job)?)
        }
        Operation::Status { job_id } => Ok(serde_json::to_value(store.job(&job_id)?)?),
        Operation::Logs {
            job_id,
            after,
            limit,
        } => store.logs(&job_id, after, limit),
        Operation::Cancel { job_id } => {
            let job = store.job(&job_id)?;
            if job.state == "queued" {
                store.transition(&job_id, "queued", "cancelled", None)?;
            } else if ["running", "starting", "cancelling"].contains(&job.state.as_str()) {
                if store.job_profile(&job_id)?.service_mode != "systemd" {
                    return Err(Error::Unqualified(
                        "complete-tree cancellation requires owned systemd service".into(),
                    ));
                }
                let info = unit_info(&job.unit)?;
                if job.invocation_id.is_none()
                    || info.get("InvocationID") != job.invocation_id.as_ref()
                {
                    return Err(invalid(
                        "cancellation requires tracked live invocation identity",
                    ));
                }
                store.transition(&job_id, &job.state, "cancelling", None)?;
                let result = systemctl(&["stop", &job.unit])?;
                if !result.status.success() {
                    return Err(Error::Resource("owned unit cancellation failed".into()));
                }
                if !owned_service_closed(store, &store.job(&job_id)?, &unit_info(&job.unit)?)? {
                    return Err(Error::Resource(
                        "owned service tree has not fully terminated; retention preserved".into(),
                    ));
                }
                let reason = closed_service_reason(
                    store,
                    &job_id,
                    "cancelled",
                    "owned complete service tree cancelled",
                )?;
                store.finish(&job_id, 143, Some(&reason))?;
            }
            Ok(serde_json::to_value(store.job(&job_id)?)?)
        }
        Operation::Artifacts {
            job_id,
            after,
            limit,
        } => Ok(serde_json::to_value(store.artifact_page(
            &job_id,
            after.as_deref(),
            limit,
        )?)?),
        Operation::QualificationReport { job_id } => Ok(serde_json::to_value(
            crate::qualification::inspect(store, &job_id)?,
        )?),
        Operation::CadRegions { job_id } => {
            Ok(serde_json::to_value(crate::cad::regions(store, &job_id)?)?)
        }
        Operation::PrepareCadSpectralScene { request } => Ok(serde_json::to_value(
            crate::cad_spectral::prepare(store, *request)?,
        )?),
        Operation::ResultsSample { request } => Ok(serde_json::to_value(crate::results::sample(
            store, &request,
        )?)?),
        Operation::ResultsCompare { request } => Ok(serde_json::to_value(
            crate::results::compare(store, &request)?,
        )?),
        Operation::ResultsSampleThermal { request } => Ok(serde_json::to_value(
            crate::thermal_results::sample(store, &request)?,
        )?),
        Operation::ResultsSampleFreezing { request } => Ok(serde_json::to_value(
            crate::freezing_results::sample(store, &request)?,
        )?),
        Operation::ResultsCompareFreezing { request } => Ok(serde_json::to_value(
            crate::freezing_results::compare(store, &request)?,
        )?),
        Operation::ResultsCompareThermal { request } => Ok(serde_json::to_value(
            crate::thermal_results::compare(store, &request)?,
        )?),
        Operation::ResultsMoisture { request } => Ok(serde_json::to_value(
            crate::moisture_results::assess(store, &request)?,
        )?),
        Operation::ResultsTransferTemperature { request } => Ok(serde_json::to_value(
            crate::thermal_transfer::project(store, &request)?,
        )?),
        Operation::ResultsRetainWetting { request } => Ok(serde_json::to_value(
            crate::wetting_retention::prepare(store, &request)?,
        )?),
        Operation::ResultsPrepareRetainedCooling { request } => Ok(serde_json::to_value(
            crate::retained_cooling::prepare(store, *request)?,
        )?),
        Operation::ResultsTransferAtmosphere { request } => Ok(serde_json::to_value(
            crate::atmosphere_transfer::prepare(store, &request)?,
        )?),
        Operation::Describe { job_id } => {
            let plan = store.plan(&job_id)?;
            let binding = match store.execution_binding(&job_id) {
                Ok(binding) => Some(binding),
                Err(Error::Unqualified(_)) => None,
                Err(error) => return Err(error),
            };
            Ok(
                serde_json::json!({"job":store.job(&job_id)?,"science_id":plan.science_id()?,
                "execution_id":plan.id()?,"presentation_id":plan.case.as_ref().map(|c| digest(&c.presentation)).transpose()?,
                "execution_binding":binding,
                "execution_authorization":store.execution_authorization(&job_id)?,
                "artifacts":store.artifact_page(&job_id, None, default_artifact_limit())?,"arrays":"retained in artifacts; not embedded in responses"}),
            )
        }
    }
}
pub fn serve(state: &Path, socket: &Path, profile_path: &Path) -> Result<()> {
    serve_authorized(state, socket, profile_path, None)
}
pub fn serve_authorized(
    state: &Path,
    socket: &Path,
    profile_path: &Path,
    authority_path: Option<&Path>,
) -> Result<()> {
    let store = Store::open(state)?;
    let profile = load_profile(profile_path)?;
    let authority: Option<HostAuthority> = authority_path
        .map(|path| -> Result<_> {
            let value: HostAuthority = serde_json::from_slice(&read_bounded(path, MAX_MESSAGE)?)?;
            value.validate()?;
            Ok(value)
        })
        .transpose()?;
    if authority.is_some() && profile.service_mode != "systemd" {
        return Err(invalid(
            "shared durable admission requires systemd service-tree ownership",
        ));
    }
    let admission = authority
        .as_ref()
        .map(|a| Admission::open(&crate::admission::shared_root()?, a))
        .transpose()?;
    if let Some(admission) = &admission {
        admission.register_state(&fs::canonicalize(state)?)?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(safe_path(state, "worker.lock")?)?;
    lock.try_lock_exclusive()
        .map_err(|_| Error::Resource("one worker per state root".into()))?;
    if socket.parent() != Some(state) {
        return Err(invalid("socket must be inside private state root"));
    }
    if socket.exists() {
        use std::os::unix::fs::FileTypeExt;
        if !fs::symlink_metadata(socket)?.file_type().is_socket() {
            return Err(invalid("refusing to replace non-socket"));
        }
        fs::remove_file(socket)?;
    }
    reconcile(&store, &profile, true, admission.as_ref())?;
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                let mut bytes = Vec::new();
                let result = BufReader::new(&stream)
                    .take(MAX_MESSAGE + 1)
                    .read_until(b'\n', &mut bytes);
                let response = match result {
                    Ok(_) if bytes.len() as u64 <= MAX_MESSAGE && bytes.last() == Some(&b'\n') => {
                        match serde_json::from_slice::<IncomingEnvelope<'_>>(&bytes) {
                            Ok(request) => {
                                if !token(&request.request_id) {
                                    reply("invalid".into(), Err(invalid("protocol request ID")))
                                } else {
                                    let result = if request.protocol_version != PROTOCOL_VERSION {
                                        Err(invalid("protocol version"))
                                    } else {
                                        serde_json::from_str::<Operation>(request.request.get())
                                            .map_err(|error| {
                                                invalid(format!("operation schema: {error}"))
                                            })
                                            .and_then(|operation| {
                                                dispatch(
                                                    &store,
                                                    &profile,
                                                    authority.as_ref(),
                                                    operation,
                                                )
                                            })
                                    };
                                    reply(request.request_id, result)
                                }
                            }
                            Err(e) => reply("invalid".into(), Err(e.into())),
                        }
                    }
                    _ => reply(
                        "invalid".into(),
                        Err(invalid("bounded newline-delimited request required")),
                    ),
                };
                let data = serde_json::to_vec(&response)?;
                let data = if data.len() as u64 >= MAX_MESSAGE {
                    serde_json::to_vec(&reply(
                        response.request_id,
                        Err(Error::Resource("response limit; use bounded pages".into())),
                    ))?
                } else {
                    data
                };
                let _ = stream.write_all(&data);
                let _ = stream.write_all(b"\n");
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(e) => return Err(e.into()),
        }
        reconcile(&store, &profile, false, admission.as_ref())?;
    }
}
pub fn request(socket: &Path, op: Operation) -> Result<Response> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    let request = WorkerRequest {
        protocol_version: 1,
        request_id: uuid::Uuid::new_v4().to_string(),
        request: op,
    };
    let mut bytes = serde_json::to_vec(&request)?;
    if bytes.len() as u64 > MAX_MESSAGE {
        return Err(invalid("request limit"));
    }
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    let mut response = Vec::new();
    BufReader::new(stream)
        .take(MAX_MESSAGE + 1)
        .read_until(b'\n', &mut response)?;
    if response.len() as u64 > MAX_MESSAGE {
        return Err(invalid("response limit"));
    }
    let response: Response = serde_json::from_slice(&response)?;
    if response.request_id != request.request_id || response.protocol_version != 1 {
        return Err(invalid("response identity/version mismatch"));
    }
    Ok(response)
}
pub fn backends() -> serde_json::Value {
    serde_json::json!([
        {"adapter":"channel_reference","backend":"cpu","runtime":"implemented","numerical_verification":"analytical_residual","physical_validation":"unqualified","synthetic":true},
        {"adapter":"freecad","backend":"cpu","runtime":"unqualified","minimum_security_version":"1.1.4"},
        {"adapter":"openlb","backend":"cpu","runtime":"unqualified","precision":"float64","formulation":"periodic_forced_channel","reference_evidence":"docs/qualification.md"},
        {"adapter":"openlb","backend":"hip","runtime":"unqualified","priority":"primary","precision":"float64","formulation":"periodic_forced_channel","reason":"compiler/model/hardware and KFD sandbox qualification required","decision_evidence":"docs/gpu-backends.md"},
        {"adapter":"openlb","backend":"cuda","runtime":"unqualified","priority":"best_effort","precision":"float64","formulation":"periodic_forced_channel"},
        {"adapter":"openlb","backend":"vulkan","runtime":"unsupported","reason":"no Vulkan backend in pinned OpenLB; Float64 and workload performance require separate evidence","decision_evidence":"docs/gpu-backends.md"},
        {"adapter":"viskores","backend":"hip","runtime":"unqualified","precision":"float64","operation":"image_data_point_gradient","scope":"physVelocity/physPressure; one retained time/shard","reference_evidence":"docs/numerical-filters.md"},
        {"adapter":"calculix","backend":"cpu","runtime":"unqualified","precision":"float64","formulations":["thermal_boundary","free_expansion"],"factorization":"SPOOLES","scope":"synthetic static C3D8 Gmsh box; imported CAD/contact/transients separate","reference_evidence":"docs/fem-references.md"},
        {"adapter":"pastix4calculix","backend":"cuda","priority":"best_effort","availability":"inspection_only","candidate_revision":"d6b493bc431ecf860b3578a2ee6a25f392eb44be","candidate_version":"6.0.1","integer_abi":"ilp64_required_not_integrated","build":"not_packaged","runtime":"unqualified","execution":"unsupported","resource_estimate":null,"numerical_verification":"unqualified","physical_validation":"unqualified","executed":false,"reason":"source candidate only: hardcoded -march=native/CUDA paths need scoped reproducibility patches; separate ILP64 BLAS/ordering/CalculiX closure and symbolic fill-in/workspace evidence required; no SPOOLES or AMD fallback","reference_evidence":"docs/fem-factorization-readiness.md"},
        {"adapter":"parsec","backend":"cuda","priority":"best_effort","availability":"inspection_only","candidate_revision":"b580d208094e6cb4106e41d5eeec84f304a090b8","candidate_version":"2.0.0","integer_abi":"ilp64_required_not_integrated","build":"not_packaged","runtime":"unqualified","execution":"unsupported","resource_estimate":null,"numerical_verification":"unqualified","physical_validation":"unqualified","executed":false,"reason":"patched pastix-6.0.1 source candidate only; development headers, explicit MPI/thread/CUDA/home-config policy and exact modified-PaStiX compatibility unqualified; no approved worker execution","reference_evidence":"docs/fem-factorization-readiness.md"},
        {"adapter":"calculix_thermal","backend":"cpu","runtime":"unqualified","precision":"float64","formulations":["plane_wall_robin"],"factorization":"SPOOLES","plan_schema_version":6,"scope":"synthetic prescribed physical histories; constant properties; independent temperature/energy gates; exact job qualification required","reference_evidence":"docs/thermal-history.md"},
        {"adapter":"gmsh_cad_mesh","backend":"cpu","runtime":"unqualified","precision":"float64","formulations":["imported_axis_aligned_box"],"plan_schema_version":7,"scope":"named BREP/source-unit/world-placement correspondence; approved retained-source worker; no solver or contact conclusion; exact job qualification required","reference_evidence":"docs/cad-mesh.md"},
        {"adapter":"calculix_imported","backend":"cpu","runtime":"unqualified","precision":"float64","formulations":["thermal_boundary","free_expansion"],"factorization":"SPOOLES","plan_schema_version":8,"scope":"controlled synthetic imported box with explicit world-origin analytical reference; approved retained-source worker; exact job qualification required","reference_evidence":"docs/fem-imported.md"},
        {"adapter":"openlb_wetting","backend":"cpu","runtime":"unqualified","precision":"float64","formulation":"well_balanced_contact_angle_2d","plan_schema_version":9,"scope":"synthetic equal-property wall-centered initial half-circle; retained original phase/velocity fields; separate mass, angle, settling and refinement gates","reference_evidence":"docs/wetting-reference.md"},
        {"adapter":"openlb_freezing","backend":"cpu","runtime":"unqualified","precision":"float64","formulation":"conduction_stefan_solidification_2d","plan_schema_version":12,"scope":"synthetic equal-phase fixed-volume conduction; complete original enthalpy/temperature/phase fields and boundary-energy ledger; no expansion or retained-water transfer","reference_evidence":"docs/freezing-reference.md"},
        {"adapter":"openlb_retained_cooling","backend":"cpu","runtime":"unqualified","precision":"float64","formulation":"stationary_equal_property_retained_phase_conduction","plan_schema_version":18,"scope":"complete bounded stationary original wetting parent controls; independent explicit thermal inputs and approval; native TRT, half-link cold/insulated walls, periodic x; conservation, analytic/refinement and physical evidence separate","reference_evidence":"docs/retained-cooling.md"},
        {"adapter":"mitsuba_spectral","backend":"cpu","runtime":"unqualified","precision":"original_float32_compensated_float64_reduction","plan_schema_version":13,"scope":"scalar_spectral directional planar irradiance, prescribed dose and bounded opaque Lambertian reflection; exact immutable job qualification required","reference_evidence":"docs/spectral-reference.md"},
        {"adapter":"libradtran_atmosphere","backend":"cpu","runtime":"unqualified","plan_schema_version":14,"scope":"content-pinned molecular no-scattering DISORT with exact black boundary; retain original edir and uu angular midpoints; exact job qualification required","reference_evidence":"docs/atmosphere-reference.md"},
        {"adapter":"atmospheric_spectral_transport","backend":"cpu","runtime":"unqualified","plan_schema_version":15,"scope":"registered original anisotropic direct/diffuse packets with independently approved receiver and prescribed dose; source retention and native execution separate","reference_evidence":"docs/atmosphere-reference.md"},
        {"adapter":"cad_direct_spectral","backend":"cpu","runtime":"unqualified","plan_schema_version":17,"scope":"original planar STL facets, opaque whole-region Lambertian optics, explicit native Float32 geometry screens and seeded power/dose; no interreflection or lifetime inference","reference_evidence":"docs/cad-spectral-scenes.md"},
        {"adapter":"calculix_contact","backend":"cpu","runtime":"unqualified","precision":"float64","formulation":"planar_linear_penalty_contact","plan_schema_version":10,"scope":"synthetic zero-Poisson two-block preload and uniform thermal expansion/opening; independent original DAT force/stress/displacement and gap checks","reference_evidence":"docs/contact-reference.md"},
        {"adapter":"thermal_contact","backend":"cpu","runtime":"unqualified","precision":"float64","formulation":"one_way_native_capacitance_projection_to_planar_contact","plan_schema_version":11,"scope":"independent native thermal histories; complete C3D8 capacitance projection with explicit pointwise loss; derived uniform contact temperatures and original six-surface moisture; exact job qualification required","reference_evidence":"docs/contact-reference.md"},
        {"adapter":"paraview","backend":"egl","runtime":"unqualified"},
        {"adapter":"ffmpeg","backend":"vaapi","runtime":"unqualified"}
    ])
}
pub fn doctor() -> Result<serde_json::Value> {
    Ok(
        serde_json::json!({"schema_version":1,"devices":devices::inventory()?,"fleetix_revision":FLEETIX_REV,
        "fleetix_contract_digest":fleetix_digest(),"backends":backends(),
        "build_provenance":crate::provenance::inspect()?,
        "hardware_qualification":"unqualified: inventory is not execution evidence",
        "historical_evidence_query":"qualify --job JOB_ID through the owning worker; source/runtime/device scope remains immutable",
        "driver_boundary":"read-only; no driver installation or host activation",
        "service_guarantees":{"systemd":"tracked cgroup and invocation; no automatic logout/boot resume", "foreground":"CI-only; worker restart interrupts jobs"}}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeRuntime {
    bwrap: String,
    cad: Option<String>,
    #[serde(default)]
    cad_closure: Option<String>,
    openlb: Option<String>,
    openlb_backend: String,
    render: Option<String>,
    video: Option<String>,
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    fem: Option<String>,
    #[serde(default)]
    fem_closure: Option<String>,
    #[serde(default)]
    thermal: Option<String>,
    #[serde(default)]
    thermal_closure: Option<String>,
    #[serde(default)]
    wetting: Option<String>,
    #[serde(default)]
    wetting_closure: Option<String>,
    #[serde(default)]
    freezing: Option<String>,
    #[serde(default)]
    freezing_closure: Option<String>,
    #[serde(default)]
    retained_cooling: Option<String>,
    #[serde(default)]
    retained_cooling_closure: Option<String>,
    #[serde(default)]
    spectral: Option<String>,
    #[serde(default)]
    spectral_closure: Option<String>,
    #[serde(default)]
    atmosphere: Option<String>,
    #[serde(default)]
    atmosphere_closure: Option<String>,
    #[serde(default)]
    atmospheric_spectral: Option<String>,
    #[serde(default)]
    atmospheric_spectral_closure: Option<String>,
    #[serde(default)]
    contact: Option<String>,
    #[serde(default)]
    contact_closure: Option<String>,
    #[serde(default)]
    cad_mesh: Option<String>,
    #[serde(default)]
    cad_mesh_closure: Option<String>,
    #[serde(default)]
    fem_imported: Option<String>,
    #[serde(default)]
    fem_imported_closure: Option<String>,
}
impl NativeRuntime {
    fn executable_for(&self, operation: &StageOperation, plan: &ExecutionPlan) -> Result<&str> {
        if *operation == StageOperation::FreezingReference && plan.retained_cooling.is_some() {
            let path = self.retained_cooling.as_deref().ok_or_else(|| {
                Error::Unqualified("source-bound retained cooling CPU adapter absent".into())
            })?;
            packaged_file(Path::new(path))?;
            Ok(path)
        } else {
            self.executable(operation)
        }
    }
    fn solver_closure_for(
        &self,
        operation: &StageOperation,
        plan: &ExecutionPlan,
    ) -> Result<&Path> {
        if *operation == StageOperation::FreezingReference && plan.retained_cooling.is_some() {
            self.retained_cooling_closure
                .as_deref()
                .map(Path::new)
                .ok_or_else(|| {
                    Error::Unqualified("source-bound cooling operation-only closure absent".into())
                })
        } else {
            self.solver_closure(operation)
        }
    }
    fn solver_closure(&self, operation: &StageOperation) -> Result<&Path> {
        let selected = match operation {
            StageOperation::FemReference => &self.fem_closure,
            StageOperation::ThermalReference => &self.thermal_closure,
            StageOperation::WettingReference => &self.wetting_closure,
            StageOperation::FreezingReference => &self.freezing_closure,
            StageOperation::SpectralReference => &self.spectral_closure,
            StageOperation::AtmosphericReference => &self.atmosphere_closure,
            StageOperation::AtmosphericTransport => &self.atmospheric_spectral_closure,
            StageOperation::ContactReference => &self.contact_closure,
            StageOperation::CadMesh => &self.cad_mesh_closure,
            StageOperation::FemImported => &self.fem_imported_closure,
            _ => return Err(invalid("fixed CPU solver operation required for closure")),
        };
        selected
            .as_deref()
            .map(Path::new)
            .ok_or_else(|| Error::Unqualified("FEM operation closure absent".into()))
    }
    fn importer_closure(&self) -> Result<&Path> {
        self.cad_closure.as_deref().map(Path::new).ok_or_else(|| {
            Error::Unqualified("runtime lacks operation-specific importer closure policy".into())
        })
    }
    fn load(path: &Path) -> Result<Self> {
        let path = packaged_file(path)?;
        let runtime: Self = serde_json::from_slice(&read_bounded(&path, MAX_MESSAGE)?)?;
        packaged_file(Path::new(&runtime.bwrap))?;
        Ok(runtime)
    }
    fn executable(&self, operation: &StageOperation) -> Result<&str> {
        let path = match operation {
            StageOperation::CadFixture | StageOperation::CadInspect => {
                self.cad.as_deref().ok_or_else(|| {
                    Error::Unqualified("CAD importer absent from selected runtime".into())
                })?
            }
            StageOperation::Openlb => self
                .openlb
                .as_deref()
                .ok_or_else(|| Error::Unqualified("OpenLB package absent".into()))?,
            StageOperation::Render => self.render.as_deref().ok_or_else(|| {
                Error::Unqualified("EGL renderer absent from selected runtime".into())
            })?,
            StageOperation::Video => self.video.as_deref().ok_or_else(|| {
                Error::Unqualified("hardware media adapter absent from selected runtime".into())
            })?,
            StageOperation::NumericalFilter => self.filter.as_deref().ok_or_else(|| {
                Error::Unqualified(
                    "HIP numerical-filter adapter absent from selected runtime".into(),
                )
            })?,
            StageOperation::FemReference => self.fem.as_deref().ok_or_else(|| {
                Error::Unqualified("CPU FEM adapter absent from selected runtime".into())
            })?,
            StageOperation::ThermalReference => self.thermal.as_deref().ok_or_else(|| {
                Error::Unqualified("CPU thermal adapter absent from selected runtime".into())
            })?,
            StageOperation::WettingReference => self.wetting.as_deref().ok_or_else(|| {
                Error::Unqualified("CPU wetting adapter absent from selected runtime".into())
            })?,
            StageOperation::FreezingReference => self.freezing.as_deref().ok_or_else(|| {
                Error::Unqualified("CPU solidification adapter absent from selected runtime".into())
            })?,
            StageOperation::SpectralReference => self.spectral.as_deref().ok_or_else(|| {
                Error::Unqualified("isolated native spectral CPU adapter unavailable".into())
            })?,
            StageOperation::AtmosphericReference => {
                self.atmosphere.as_deref().ok_or_else(|| {
                    Error::Unqualified(
                        "isolated native molecular atmospheric CPU adapter unavailable".into(),
                    )
                })?
            }
            StageOperation::ContactReference => self.contact.as_deref().ok_or_else(|| {
                Error::Unqualified("CPU contact adapter absent from selected runtime".into())
            })?,
            StageOperation::AtmosphericTransport => {
                self.atmospheric_spectral.as_deref().ok_or_else(|| {
                    Error::Unqualified(
                        "isolated atmospheric spectral transport adapter unavailable".into(),
                    )
                })?
            }
            StageOperation::CadMesh => self.cad_mesh.as_deref().ok_or_else(|| {
                Error::Unqualified("CPU CAD mesh adapter absent from selected runtime".into())
            })?,
            StageOperation::FemImported => self.fem_imported.as_deref().ok_or_else(|| {
                Error::Unqualified("CPU imported FEM adapter absent from selected runtime".into())
            })?,
            _ => return Err(invalid("not a native adapter operation")),
        };
        packaged_file(Path::new(path))?;
        Ok(path)
    }
}
fn disk_bytes(root: &Path, reject_unsafe: bool) -> Result<u64> {
    fn visit(root: &Path, reject_unsafe: bool, depth: u32, entries: &mut u64) -> Result<u64> {
        if depth > 64 {
            return Err(Error::Resource(
                "bounded disk inventory depth exhausted".into(),
            ));
        }
        let mut total = 0u64;
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            *entries += 1;
            if *entries > 131072 {
                return Err(Error::Resource(
                    "bounded disk inventory entries exhausted".into(),
                ));
            }
            let m = fs::symlink_metadata(entry.path())?;
            if reject_unsafe && !m.is_dir() && !m.is_file() {
                return Err(invalid("native output symlink/special entry rejected"));
            }
            total = total
                .checked_add(if m.is_dir() {
                    visit(&entry.path(), reject_unsafe, depth + 1, entries)?
                } else {
                    m.len()
                })
                .ok_or_else(|| invalid("disk accounting overflow"))?;
        }
        Ok(total)
    }
    // Quarantined failures count against admission, but never follow their
    // symlinks or let an unsafe raw entry poison all later jobs in the root.
    visit(root, reject_unsafe, 0, &mut 0)
}
fn native_stage(
    store: &Store,
    profile: &HostExecutionProfile,
    plan: &ExecutionPlan,
    stage: &Stage,
    dir: &Path,
    id: &str,
) -> Result<()> {
    let runtime = NativeRuntime::load(Path::new(
        profile
            .native_runtime
            .as_deref()
            .ok_or_else(|| invalid("native runtime"))?,
    ))?;
    let exe = runtime.executable_for(&stage.operation, plan)?;
    let mut command = Command::new(&runtime.bwrap);
    command
        .env_clear()
        .args([
            "--unshare-all",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/home",
            "--dir",
            "/home/worker",
            "--setenv",
            "HOME",
            "/home/worker",
            "--setenv",
            "LANG",
            "C.UTF-8",
            "--setenv",
            "PATH",
            "/nonexistent",
            "--setenv",
            "OMP_NUM_THREADS",
        ])
        .arg(profile.threads.to_string())
        .args(["--bind"])
        .arg(dir)
        .arg("/work")
        .args(["--chdir", "/work"]);
    if matches!(
        stage.operation,
        StageOperation::CadInspect | StageOperation::CadFixture
    ) {
        crate::sandbox::mount_importer(&mut command, runtime.importer_closure()?, Path::new(exe))?;
    } else if matches!(
        stage.operation,
        StageOperation::FemReference
            | StageOperation::ThermalReference
            | StageOperation::WettingReference
            | StageOperation::FreezingReference
            | StageOperation::SpectralReference
            | StageOperation::AtmosphericReference
            | StageOperation::AtmosphericTransport
            | StageOperation::ContactReference
            | StageOperation::CadMesh
            | StageOperation::FemImported
    ) {
        let (closure_path, policy_variable, policy) = match stage.operation {
            StageOperation::FemReference => (
                "/fem-runtime-closure.txt",
                "HARBOR_CAD_FEM_POLICY",
                crate::execution::FEM_SANDBOX_POLICY,
            ),
            StageOperation::ThermalReference => (
                "/thermal-runtime-closure.txt",
                "HARBOR_CAD_THERMAL_POLICY",
                crate::execution::THERMAL_SANDBOX_POLICY,
            ),
            StageOperation::WettingReference => (
                "/wetting-runtime-closure.txt",
                "HARBOR_CAD_WETTING_POLICY",
                crate::execution::WETTING_SANDBOX_POLICY,
            ),
            StageOperation::FreezingReference if plan.retained_cooling.is_some() => (
                "/retained-cooling-runtime-closure.txt",
                "HARBOR_CAD_RETAINED_COOLING_POLICY",
                crate::cooling_execution::SANDBOX_POLICY,
            ),
            StageOperation::FreezingReference => (
                "/freezing-runtime-closure.txt",
                "HARBOR_CAD_FREEZING_POLICY",
                crate::freezing::SANDBOX_POLICY,
            ),
            StageOperation::SpectralReference if plan.cad_transport.is_some() => (
                "/spectral-runtime-closure.txt",
                "HARBOR_CAD_CAD_SPECTRAL_POLICY",
                crate::cad_transport::SANDBOX_POLICY,
            ),
            StageOperation::SpectralReference => (
                "/spectral-runtime-closure.txt",
                "HARBOR_CAD_SPECTRAL_POLICY",
                crate::radiation::SANDBOX_POLICY,
            ),
            StageOperation::AtmosphericReference => (
                "/atmosphere-runtime-closure.txt",
                "HARBOR_CAD_ATMOSPHERE_POLICY",
                crate::atmosphere::SANDBOX_POLICY,
            ),
            StageOperation::ContactReference => (
                "/contact-runtime-closure.txt",
                "HARBOR_CAD_CONTACT_POLICY",
                crate::execution::CONTACT_SANDBOX_POLICY,
            ),
            StageOperation::AtmosphericTransport => (
                "/spectral-runtime-closure.txt",
                "HARBOR_CAD_ATMOSPHERIC_SPECTRAL_POLICY",
                crate::atmospheric_transport::SANDBOX_POLICY,
            ),
            StageOperation::CadMesh => (
                "/cad-mesh-runtime-closure.txt",
                "HARBOR_CAD_CAD_MESH_POLICY",
                crate::execution::CAD_MESH_SANDBOX_POLICY,
            ),
            StageOperation::FemImported => (
                "/fem-imported-runtime-closure.txt",
                "HARBOR_CAD_FEM_IMPORTED_POLICY",
                crate::execution::FEM_IMPORTED_SANDBOX_POLICY,
            ),
            _ => return Err(invalid("fixed CPU descriptor operation required")),
        };
        crate::sandbox::mount_closure(
            &mut command,
            runtime.solver_closure_for(&stage.operation, plan)?,
            Path::new(exe),
        )?;
        command
            .args(["--ro-bind"])
            .arg(runtime.solver_closure_for(&stage.operation, plan)?)
            .arg(closure_path);
        command.args(["--setenv", policy_variable, policy]);
        command
            .args(["--setenv", "HARBOR_CAD_HOST_NETNS"])
            .arg(fs::read_link("/proc/self/ns/net")?);
    } else {
        command.args(["--ro-bind", "/nix/store", "/nix/store"]);
    }
    if stage.operation == StageOperation::SpectralReference && plan.cad_transport.is_some() {
        command
            .args(["--ro-bind"])
            .arg(crate::cad_transport::registered(store, id, plan)?)
            .arg("/source");
    }
    if matches!(stage.operation, StageOperation::NumericalFilter) {
        let (root, _, _) = crate::fields::registered(store, id)?;
        command.args(["--ro-bind"]).arg(root).arg("/inputs/fields");
    }
    if stage.operation == StageOperation::AtmosphericTransport {
        let spec = plan
            .atmospheric_transport
            .as_ref()
            .ok_or_else(|| invalid("approved original-source atmospheric transport required"))?;
        crate::atmospheric_transport::registered_source(store, spec)?;
        command
            .args(["--dir", "/inputs", "--ro-bind"])
            .arg(safe_path(
                &store.job_dir(id)?,
                "source-atmosphere-original.txt",
            )?)
            .arg("/inputs/atmosphere-original.txt");
    }
    if let Some(spec) = &plan.retained_cooling {
        crate::cooling_execution::source(store, spec)?;
        let original = crate::cooling_execution::registered(store, id, plan)?;
        command
            .args(["--dir", "/inputs", "--ro-bind"])
            .arg(original)
            .arg("/inputs/wetting-original.csv");
    }
    if matches!(
        stage.operation,
        StageOperation::CadMesh | StageOperation::FemImported
    ) {
        let root = crate::cad_source::registered(store, id, plan)?;
        command
            .args(["--dir", "/inputs", "--ro-bind"])
            .arg(safe_path(&root, "solid.brep")?)
            .arg("/inputs/solid.brep");
    }
    let retained_fields = if matches!(
        stage.operation,
        StageOperation::Render | StageOperation::Video
    ) {
        let (root, snapshot, manifest_digest) = crate::fields::registered(store, id)?;
        if plan.source.is_some() {
            crate::presentation::verify(plan, &snapshot, &manifest_digest)?;
            command.args([
                "--setenv",
                "HARBOR_CAD_PRESENTATION_EXECUTION_ID",
                &plan.id()?,
            ]);
        } else if snapshot.science_id != plan.science_id()?
            || snapshot.execution_id != plan.id()?
            || snapshot.execution_binding_digest != digest(&store.execution_binding(id)?)?
        {
            return Err(invalid(
                "presentation differs from immutable field science/execution identity",
            ));
        }
        command
            .args(["--ro-bind"])
            .arg(safe_path(&root, "tmp/vtkData")?)
            .arg("/work/tmp/vtkData")
            .args(["--ro-bind"])
            .arg(safe_path(&root, "openlb-receipt.json")?)
            .arg("/work/openlb-receipt.json")
            .args(["--ro-bind"])
            .arg(safe_path(&root, "snapshot.json")?)
            .arg("/field-snapshot.json")
            .args([
                "--setenv",
                "HARBOR_CAD_FIELD_SNAPSHOT",
                "/field-snapshot.json",
            ]);
        Some((snapshot, manifest_digest))
    } else {
        None
    };
    if let Some(frames) = &plan.frames {
        if !matches!(stage.operation, StageOperation::Video) {
            return Err(invalid("retained frames mount only in video stage"));
        }
        let (root, _) = crate::frames::registered(store, id, plan)?;
        command.args(["--ro-bind"]).arg(root).arg("/inputs/frames");
        command.args(["--setenv", "HARBOR_CAD_FRAMES_DIRECTORY", "/inputs/frames"]);
        command.args([
            "--setenv",
            "HARBOR_CAD_FRAME_SOURCE_EXECUTION_ID",
            &frames.plan_digest,
        ]);
    }
    let mut hip_identity = None;
    if let Some(selection) = &stage.selection {
        if selection.role != Role::Compute {
            let binding = devices::DrmSandbox::resolve(&selection.pci)?;
            binding.apply(&mut command);
            store.event(
                id,
                "dri_binding_verified",
                &serde_json::to_string(&binding)?,
            )?;
        } else if selection.backend == "hip" {
            if store.execution_authorization(id)?.is_none() {
                return Err(Error::Unqualified(
                    "HIP worker execution requires authoritative shared admission".into(),
                ));
            }
            let binding = devices::HipSandbox::resolve(
                &selection.pci,
                selection
                    .backend_uuid
                    .as_deref()
                    .ok_or_else(|| invalid("exact HIP UUID required"))?,
            )?;
            binding.apply(&mut command);
            store.event(
                id,
                "hip_binding_verified",
                &serde_json::to_string(&binding)?,
            )?;
            hip_identity = Some(binding.identity);
        } else {
            // NVIDIA nodes require UUID→minor correlation and a tested per-device mount policy.
            return Err(Error::Unqualified(
                "CUDA per-device sandbox mount policy not hardware-qualified".into(),
            ));
        }
    }
    if matches!(stage.operation, StageOperation::CadInspect) {
        if let Some(spec) = &plan.cad_variant {
            spec.validate()?;
            command
                .args(["--ro-bind"])
                .arg(crate::cad_variant::registered(store, id, plan)?)
                .arg("/input.FCStd");
        } else {
            let source = safe_path(
                Path::new(&profile.allowed_input_root),
                &plan.channel_case()?.geometry.source,
            )?;
            let job_dir = store.job_dir(id)?;
            let artifact = snapshot_cad_input(
                &source,
                &job_dir,
                plan.channel_case()?
                    .geometry
                    .sha256
                    .as_deref()
                    .ok_or_else(|| invalid("source CAD digest required"))?,
                plan.observation.max_artifact_bytes,
            )?;
            store.add_artifact(id, &artifact)?;
            command
                .args(["--ro-bind"])
                .arg(safe_path(&job_dir, &artifact.path)?)
                .arg("/input.FCStd");
        }
    }
    let op = serde_json::to_value(&stage.operation)?
        .as_str()
        .ok_or_else(|| invalid("operation"))?
        .to_owned();
    let receipt = safe_path(
        dir,
        &if matches!(stage.operation, StageOperation::FemReference) {
            "fem-reference-receipt.json".into()
        } else if matches!(stage.operation, StageOperation::ThermalReference) {
            "thermal-receipt.json".into()
        } else if matches!(stage.operation, StageOperation::WettingReference) {
            "verified-wetting-receipt.json".into()
        } else if stage.operation == StageOperation::FreezingReference
            && plan.retained_cooling.is_some()
        {
            "retained-cooling-receipt.json".into()
        } else if matches!(stage.operation, StageOperation::FreezingReference) {
            "freezing-receipt.json".into()
        } else if stage.operation == StageOperation::SpectralReference
            && plan.cad_transport.is_some()
        {
            "cad-spectral-receipt.json".into()
        } else if matches!(stage.operation, StageOperation::SpectralReference) {
            "spectral-receipt.json".into()
        } else if stage.operation == StageOperation::AtmosphericReference {
            "atmosphere-receipt.json".into()
        } else if stage.operation == StageOperation::AtmosphericTransport {
            "atmospheric-spectral-receipt.json".into()
        } else if matches!(stage.operation, StageOperation::ContactReference) {
            "contact-receipt.json".into()
        } else if matches!(stage.operation, StageOperation::CadMesh) {
            "cad-mesh-receipt.json".into()
        } else if matches!(stage.operation, StageOperation::FemImported) {
            "fem-imported-receipt.json".into()
        } else {
            format!("{op}-receipt.json")
        },
    )?;
    if fs::symlink_metadata(&receipt).is_ok() {
        return Err(invalid(
            "native receipt already exists before its stage; stale or forged output rejected",
        ));
    }
    command
        .args(["--ro-bind"])
        .arg(safe_path(
            &store.job_dir(id)?,
            &if stage.operation == StageOperation::ThermalReference
                && plan.thermal_contact.is_some()
            {
                format!("native-{}-request.json", stage.id)
            } else if matches!(stage.operation, StageOperation::NumericalFilter) {
                "native-filter-request.json".into()
            } else if matches!(stage.operation, StageOperation::FemReference) {
                "native-fem-request.json".into()
            } else if matches!(stage.operation, StageOperation::ThermalReference) {
                "native-thermal-request.json".into()
            } else if matches!(stage.operation, StageOperation::WettingReference) {
                "native-wetting-request.json".into()
            } else if stage.operation == StageOperation::FreezingReference
                && plan.retained_cooling.is_some()
            {
                "native-retained-cooling-request.json".into()
            } else if matches!(stage.operation, StageOperation::FreezingReference) {
                "native-freezing-request.json".into()
            } else if stage.operation == StageOperation::SpectralReference
                && plan.cad_transport.is_some()
            {
                "native-cad-spectral-request.json".into()
            } else if matches!(stage.operation, StageOperation::SpectralReference) {
                "native-spectral-request.json".into()
            } else if stage.operation == StageOperation::AtmosphericReference {
                "native-atmosphere-request.json".into()
            } else if stage.operation == StageOperation::AtmosphericTransport {
                "native-atmospheric-spectral-request.json".into()
            } else if matches!(stage.operation, StageOperation::ContactReference) {
                "native-contact-request.json".into()
            } else if matches!(stage.operation, StageOperation::CadMesh) {
                "native-cad-mesh-request.json".into()
            } else if matches!(stage.operation, StageOperation::FemImported) {
                "native-fem-imported-request.json".into()
            } else {
                "native-plan.json".into()
            },
        )?)
        .arg(if plan.retained_cooling.is_some() {
            "/inputs/request.json"
        } else {
            "/plan.json"
        });
    command
        .arg("--")
        .arg(exe)
        .arg(
            if matches!(
                stage.operation,
                StageOperation::FemReference
                    | StageOperation::FemImported
                    | StageOperation::WettingReference
                    | StageOperation::FreezingReference
                    | StageOperation::SpectralReference
                    | StageOperation::AtmosphericReference
                    | StageOperation::AtmosphericTransport
                    | StageOperation::ContactReference
            ) {
                "reference"
            } else if matches!(stage.operation, StageOperation::ThermalReference) {
                "run"
            } else if matches!(stage.operation, StageOperation::CadMesh) {
                "mesh"
            } else {
                &op
            },
        )
        .arg(if plan.retained_cooling.is_some() {
            "/inputs/request.json"
        } else {
            "/plan.json"
        });
    let log = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(dir.join(format!("{}.log", stage.id)))?;
    command
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    let maximum = profile.max_disk_bytes;
    unsafe {
        command.pre_exec(move || {
            let limit = libc::rlimit {
                rlim_cur: maximum,
                rlim_max: maximum,
            };
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = NativeProcess::spawn(command)?;
    let status = child.wait(
        Duration::from_secs(u64::from(profile.timeout_seconds)),
        || {
            if let Some(identity) = &hip_identity
                && devices::HipIdentity::resolve(&identity.pci)? != *identity
            {
                return Err(Error::Unqualified(
                    "HIP topology/device identity changed during execution".into(),
                ));
            }
            if disk_bytes(dir, true)? > plan.observation.max_artifact_bytes {
                return Err(Error::Resource("scientific output budget exhausted".into()));
            }
            Ok(())
        },
    )?;
    if !status.success() {
        return Err(Error::Unqualified(format!("native {op} failed: {status}")));
    }
    let evidence =
        if stage.operation == StageOperation::SpectralReference && plan.cad_transport.is_some() {
            crate::cad_transport_fields::read_receipt(&receipt)?
        } else if plan.retained_cooling.is_some() {
            serde_json::from_slice(&read_bounded(&receipt, 256 * 1024)?)?
        } else {
            read_native_receipt(&receipt)?
        };
    validate_native_receipt(stage, &evidence)?;
    if stage.operation == StageOperation::SpectralReference && plan.cad_transport.is_some() {
        let source = crate::cad_transport::registered(store, id, plan)?;
        crate::cad_transport_fields::verify_receipt(
            plan.cad_transport
                .as_ref()
                .ok_or_else(|| invalid("approved CAD optical recipe required"))?,
            &source,
            dir,
            &evidence,
        )?;
    } else if stage.operation == StageOperation::SpectralReference {
        crate::spectral_fields::verify(
            plan.spectral
                .as_ref()
                .ok_or_else(|| invalid("approved spectral recipe required"))?,
            dir,
            &evidence,
        )?;
    }
    if stage.operation == StageOperation::AtmosphericReference {
        crate::atmosphere::verify(
            plan.atmosphere
                .as_ref()
                .ok_or_else(|| invalid("approved atmospheric recipe required"))?,
            dir,
            &evidence,
        )?;
        let native = crate::execution::FileIdentity::capture(
            Path::new(
                evidence["native_executable"]
                    .as_str()
                    .ok_or_else(|| invalid("native atmosphere executable required"))?,
            ),
            true,
        )?;
        if evidence["native_executable_sha256"] != native.sha256 {
            return Err(invalid(
                "observed native atmosphere executable identity changed",
            ));
        }
    }
    if matches!(stage.operation, StageOperation::NumericalFilter) {
        crate::filters::verify_receipt(store, id, plan, dir, &evidence)?;
    }
    if stage.operation == StageOperation::AtmosphericTransport {
        let spec = plan
            .atmospheric_transport
            .as_ref()
            .ok_or_else(|| invalid("source-bound atmospheric transport recipe required"))?;
        crate::atmospheric_transport::registered_source(store, spec)?;
        let original = read_bounded(
            &safe_path(&store.job_dir(id)?, "source-atmosphere-original.txt")?,
            32 * 1024 * 1024,
        )?;
        crate::atmospheric_transport::verify_receipt(
            spec,
            std::str::from_utf8(&original)
                .map_err(|_| invalid("original atmosphere UTF-8 required"))?,
            dir,
            &evidence,
        )?;
    }
    if matches!(stage.operation, StageOperation::FemReference) {
        crate::fem::verify_receipt(plan, &evidence)?;
    }
    if matches!(stage.operation, StageOperation::ThermalReference) {
        if let Some(spec) = &plan.thermal_contact {
            crate::thermal::verify_spec(spec.thermal_stage(&stage.id)?, &evidence)?;
        } else {
            crate::thermal::verify_receipt(plan, &evidence)?;
        }
    }
    if matches!(stage.operation, StageOperation::WettingReference) {
        crate::wetting::verify_receipt(plan, dir, &evidence)?;
    }
    if stage.operation == StageOperation::FreezingReference && plan.retained_cooling.is_some() {
        let spec = plan
            .retained_cooling
            .as_ref()
            .ok_or_else(|| invalid("source-bound cooling recipe required"))?;
        crate::cooling_execution::source(store, spec)?;
        let original = crate::cooling_execution::registered(store, id, plan)?;
        let bytes = read_bounded(&original, 16 * 1024 * 1024)?;
        crate::cooling_fields::verify(spec, &bytes, dir, &evidence)?;
        crate::cooling_vtk::publish(store, id, plan, dir)?;
    } else if stage.operation == StageOperation::FreezingReference {
        crate::freezing_fields::verify(
            plan.freezing
                .as_ref()
                .ok_or_else(|| invalid("approved solidification recipe required"))?,
            dir,
            &evidence,
        )?;
    }
    if matches!(stage.operation, StageOperation::ContactReference) {
        let coupled = if let Some(spec) = &plan.thermal_contact {
            Some(
                crate::thermal_contact::derive(
                    spec,
                    id,
                    dir.parent()
                        .and_then(Path::parent)
                        .ok_or_else(|| invalid("coupling native root"))?,
                )?
                .0,
            )
        } else {
            None
        };
        crate::contact::verify_native_outputs(
            coupled
                .as_ref()
                .or(plan.contact.as_ref())
                .ok_or_else(|| invalid("native contact recipe required"))?,
            dir,
            &evidence,
        )?;
    }
    if matches!(stage.operation, StageOperation::CadMesh) {
        crate::cad_source::registered(store, id, plan)?;
        crate::cad_mesh::verify_receipt(plan, dir, &evidence)?;
    }
    if plan.cad_variant.is_some() && stage.operation == StageOperation::CadInspect {
        crate::cad_variant::registered(store, id, plan)?;
        crate::cad_variant::verify_outputs(plan, dir, &evidence)?;
    }
    if matches!(stage.operation, StageOperation::FemImported) {
        crate::cad_source::registered(store, id, plan)?;
        crate::fem_imported::verify_receipt(plan, dir, &evidence)?;
    }
    if let Some(frames) = &plan.frames {
        crate::frames::registered(store, id, plan)?;
        if evidence["source_render_execution_id"] != frames.plan_digest
            || evidence["frame_sequence_sha256"] != frames.sequence_sha256
        {
            return Err(invalid(
                "independent video receipt differs from immutable source frames",
            ));
        }
    }
    if let Some((snapshot, manifest_digest)) = retained_fields {
        let (_, _, observed_digest) = crate::fields::registered(store, id)?;
        if observed_digest != manifest_digest
            || evidence["field_snapshot_sha256"] != manifest_digest
            || evidence["field_artifact_id"] != snapshot.artifact_id
            || evidence["science_id"] != snapshot.science_id
            || evidence["execution_id"] != snapshot.execution_id
            || (plan.source.is_some() && evidence["presentation_execution_id"] != plan.id()?)
        {
            return Err(invalid(
                "presentation receipt lacks exact retained-field identity",
            ));
        }
    }
    if let Some(identity) = hip_identity {
        if matches!(stage.operation, StageOperation::NumericalFilter) {
            identity.verify_filter_receipt(&evidence)?;
        } else {
            identity.verify_receipt(&evidence)?;
        }
    }
    store.event(
        id,
        "native_receipt",
        &serde_json::json!({
            "stage":stage.id, "operation":op, "receipt_sha256":digest(&evidence)?,
            "backend":evidence["backend"], "executed":evidence["executed"],
            "physical_validation":"unqualified"
        })
        .to_string(),
    )?;
    Ok(())
}

fn read_native_receipt(path: &Path) -> Result<serde_json::Value> {
    let input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_MESSAGE {
        return Err(invalid("native receipt must be a bounded regular file"));
    }
    let mut bytes = Vec::new();
    input.take(MAX_MESSAGE + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MESSAGE {
        return Err(invalid("native receipt grew beyond its response budget"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn validate_native_receipt(stage: &Stage, evidence: &serde_json::Value) -> Result<()> {
    let adapter = match stage.operation {
        StageOperation::CadFixture | StageOperation::CadInspect => "FreeCAD",
        StageOperation::Openlb => "OpenLB",
        StageOperation::Render => "ParaView",
        StageOperation::Video => "FFmpeg",
        StageOperation::NumericalFilter => "Viskores",
        StageOperation::FemReference => "CalculiX",
        StageOperation::ThermalReference => "CalculiX",
        StageOperation::WettingReference => "OpenLB",
        StageOperation::FreezingReference => "OpenLB",
        StageOperation::SpectralReference => "Mitsuba",
        StageOperation::AtmosphericTransport => "Mitsuba",
        StageOperation::AtmosphericReference => "libRadtran",
        StageOperation::ContactReference => "CalculiX",
        StageOperation::CadMesh => "Gmsh",
        StageOperation::FemImported => "CalculiX",
        _ => return Err(invalid("not a native adapter operation")),
    };
    let backend = stage
        .selection
        .as_ref()
        .map_or("cpu", |s| s.backend.as_str());
    if evidence["adapter"] != adapter
        || evidence["backend"] != backend
        || evidence["executed"] != true
        || evidence["software_fallback"] != false
        || (matches!(
            stage.operation,
            StageOperation::CadFixture | StageOperation::CadInspect
        ) && evidence["import_policy"] != crate::sandbox::IMPORT_POLICY)
        || (stage.gpu == GpuRequirement::Required
            && evidence["pci"].as_str() != stage.selection.as_ref().map(|s| s.pci.as_str()))
    {
        return Err(Error::Unqualified(
            "adapter/backend/execution receipt mismatch; fallback rejected".into(),
        ));
    }
    Ok(())
}

pub fn run_job(state: &Path, profile_path: &Path, id: &str) -> Result<()> {
    let store = Store::open(state)?;
    let profile = load_profile(profile_path)?;
    if digest(&profile)? != digest(&store.job_profile(id)?)? {
        return Err(invalid(
            "effective profile differs from immutable job binding",
        ));
    }
    let job = store.job(id)?;
    let plan = store.plan(id)?;
    let mut service_group = None;
    let invocation = if profile.service_mode == "systemd" {
        let invocation = std::env::var("INVOCATION_ID")
            .map_err(|_| invalid("missing systemd invocation identity"))?;
        if invocation.len() != 32 || !invocation.bytes().all(|v| v.is_ascii_hexdigit()) {
            return Err(invalid("invalid systemd invocation identity"));
        }
        let info = unit_info(&job.unit)?;
        let group = info
            .get("ControlGroup")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| invalid("missing owned service cgroup"))?;
        let membership = fs::read_to_string("/proc/self/cgroup")?;
        if info.get("InvocationID") != Some(&invocation)
            || info.get("MainPID").and_then(|v| v.parse::<u32>().ok()) != Some(std::process::id())
            || !membership
                .lines()
                .any(|line| line.strip_prefix("0::") == Some(group.as_str()))
            || info.get("LoadState").map(String::as_str) != Some("loaded")
            || !info
                .get("ActiveState")
                .is_some_and(|v| v == "active" || v == "activating")
        {
            return Err(invalid(
                "job execution requires its live systemd invocation, main PID and exact cgroup",
            ));
        }
        let group_root = safe_path(
            Path::new("/sys/fs/cgroup"),
            group
                .strip_prefix('/')
                .ok_or_else(|| invalid("absolute owned service group required"))?,
        )?;
        let kernel_resources = crate::measurements::capture(
            &group_root,
            plan.peak_ram(),
            profile.threads,
            "before_native_launch",
        )?;
        service_group = Some((group.clone(), group_root));
        let owner = serde_json::json!({"unit":job.unit,"invocation":invocation,"main_pid":std::process::id(),"control_group":group,"kernel_resources":kernel_resources});
        store.add_artifact(
            id,
            &commit_artifact(
                &store.job_dir(id)?,
                "service-owner.json",
                &serde_json::to_vec_pretty(&owner)?,
                "json",
                "live systemd unit, main PID and exact cgroup verified before job execution",
            )?,
        )?;
        store.event(id, "service_owner_verified", &owner.to_string())?;
        Some(invocation)
    } else {
        None
    };
    let binding = store.execution_binding(id)?;
    binding.verify(&store.plan(id)?, &profile)?;
    binding.verify_running_runner(&profile)?;
    crate::retention::verify_ready(&store.root, id, &binding, profile.service_mode == "systemd")?;
    if let Some(authorization) = store.execution_authorization(id)? {
        authorization.verify(&store.plan(id)?, &profile, &binding)?;
        Admission::open(&crate::admission::shared_root()?, &authorization.authority)?
            .verify(&store, id)?;
    }
    if !store.transition(id, "starting", "running", None)? {
        return Err(invalid("job must be starting exactly once"));
    }
    store.connection.execute(
        "UPDATE jobs SET invocation=?1 WHERE id=?2",
        rusqlite::params![invocation, id],
    )?;
    let execution_result = execute_job(&store, &profile, id);
    let measurement_result = service_group.map_or(Ok(()), |(control_group, group)| -> Result<()> {
        let metrics = crate::measurements::capture(&group, plan.peak_ram(), profile.threads, "after_native_execution")?;
        let report = serde_json::json!({"schema_version":1,"job_id":id,"unit":job.unit,"invocation":invocation,
            "execution_id":plan.id()?,"control_group":control_group,"kernel_resources":metrics});
        store.add_artifact(id, &commit_artifact(&store.job_dir(id)?, "service-resources.json",
            &serde_json::to_vec_pretty(&report)?, "json", "verified owned-service kernel controls and aggregate process-tree peaks")?)
    });
    if let Err(error) = &measurement_result {
        store.event(id, "service_measurement_failed", &error.to_string())?;
    }
    let result = execution_result.and(measurement_result);
    if let Err(error) = &result {
        let saved = retain_native_failure(
            &store,
            id,
            "failed",
            &serde_json::to_value(error.diagnostic())?,
        );
        if let Err(retention) = saved {
            store.event(id, "native_failure_retention_error", &retention.to_string())?;
        }
    }
    store.finish(
        id,
        if result.is_ok() { 0 } else { 1 },
        result.as_ref().err().map(ToString::to_string).as_deref(),
    )?;
    let _ = job;
    result
}

/// Called after adapter cleanup or confirmed owned-service termination. A
/// killed job cannot run its own cleanup; the persistent worker recovers the
/// bounded closed records before publishing its terminal state.
fn retain_native_failure(
    store: &Store,
    id: &str,
    execution: &str,
    cause: &serde_json::Value,
) -> Result<()> {
    let dir = store.job_dir(id)?;
    let raw = safe_path(&dir, ".native-incomplete")?;
    if !raw.exists() {
        return Ok(());
    }
    let registered: bool = store.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM artifacts WHERE job=?1 AND path='native-failure.json')",
        [id],
        |row| row.get(0),
    )?;
    if registered {
        return Ok(());
    }
    let snapshot = retain_failed_native_tree(
        &raw,
        &dir,
        store.recorded_plan(id)?.observation.max_artifact_bytes,
    )?;
    for artifact in &snapshot.artifacts {
        store.add_artifact(id, artifact)?;
    }
    let report = serde_json::json!({"schema_version":1,"execution":execution,"physical_validation":"unqualified",
        "cause":cause,"snapshot":snapshot,"raw_tree_retained":".native-incomplete",
        "partial_files":"opaque failed-attempt bytes; not qualified scientific outputs"});
    let manifest = commit_artifact(
        &dir,
        "native-failure.json",
        &serde_json::to_vec_pretty(&report)?,
        "json",
        "closed failed/cancelled native output inventory; omissions are explicit",
    )?;
    store.add_artifact(id, &manifest)?;
    Ok(())
}

fn closed_service_reason(store: &Store, id: &str, execution: &str, reason: &str) -> Result<String> {
    if let Err(error) =
        retain_native_failure(store, id, execution, &serde_json::json!({"reason":reason}))
    {
        store.event(id, "native_failure_retention_error", &error.to_string())?;
        return Ok(format!(
            "{reason}; raw output quarantined; recovery error: {error}"
        ));
    }
    Ok(reason.into())
}
fn execute_job(store: &Store, profile: &HostExecutionProfile, id: &str) -> Result<()> {
    let plan = store.plan(id)?;
    check_plan(&plan, profile)?;
    let dir = store.job_dir(id)?;
    store.add_artifact(
        id,
        &commit_artifact(
            &dir,
            "execution-binding.json",
            &serde_json::to_vec_pretty(&store.execution_binding(id)?)?,
            "json",
            "exact runner, runtime, host profile and sandbox policy binding",
        )?,
    )?;
    // Hold one physical-card reservation across roles and stages, independent of worker lifetime.
    let keys: Vec<_> = plan
        .stages
        .iter()
        .filter_map(|s| s.selection.as_ref().map(|g| g.pci.clone()))
        .collect();
    let authorized = store.execution_authorization(id)?.is_some();
    let _locks = if keys.is_empty() {
        Vec::new()
    } else if authorized {
        // Durable admission already owns these cards. Never spend the service
        // deadline waiting; legacy probe locks were checked before launch.
        resources::try_lock_cards(&resources::card_reservation_root()?, &keys)?.ok_or_else(
            || {
                Error::Resource(
                    "selected card became busy before launch; no runtime wait or fallback".into(),
                )
            },
        )?
    } else {
        let reservations = resources::card_reservation_root()?;
        let start = Instant::now();
        loop {
            if let Some(locks) = resources::try_reserve_cards(&reservations, &keys)? {
                break locks;
            }
            if start.elapsed().as_secs() > u64::from(profile.timeout_seconds) {
                return Err(Error::Resource(
                    "physical card reservation wait expired".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let manifest = commit_artifact(
        &dir,
        "plan.json",
        &serde_json::to_vec_pretty(&plan)?,
        "json",
        "immutable approved execution plan",
    )?;
    store.add_artifact(id, &manifest)?;
    if let Some(authorization) = store.execution_authorization(id)? {
        store.add_artifact(
            id,
            &commit_artifact(
                &dir,
                "execution-authorization.json",
                &serde_json::to_vec_pretty(&authorization)?,
                "json",
                "immutable host/device authority bound to exact plan/profile/execution",
            )?,
        )?;
    }
    store.add_artifact(
        id,
        &commit_artifact(
            &dir,
            "host-profile.json",
            &serde_json::to_vec_pretty(profile)?,
            "json",
            "immutable effective host policy and resource limits",
        )?,
    )?;
    let native_work = dir.join(".native-incomplete");
    let mut native_pending = false;
    for stage in &plan.stages {
        store.event(id, "stage_started", &stage.id)?;
        match stage.operation {
            StageOperation::ChannelReference => {
                let c = plan.channel_case()?;
                let result = science::channel_reference(
                    c.acceleration.si("acceleration")?,
                    c.channel_height.si("length")?,
                    c.kinematic_viscosity.si("kinematic_viscosity")?,
                    c.resolution as usize,
                )?;
                if result.numerical_error > c.applicability.numerical_tolerance {
                    return Err(invalid(
                        "analytical reference residual outside declared tolerance",
                    ));
                }
                let mut csv = "y_m,velocity_m_s\n".to_string();
                for (y, u) in result.y_m.iter().zip(&result.velocity_m_s) {
                    csv.push_str(&format!("{y:.17e},{u:.17e}\n"));
                }
                let mut artifact = commit_artifact(
                    &dir,
                    "channel.csv",
                    csv.as_bytes(),
                    "csv",
                    "synthetic analytical reference; not OpenLB",
                )?;
                artifact.units = Some("m,m/s".into());
                artifact.time_s = Some(0.);
                artifact.association = Some("point".into());
                store.add_artifact(id, &artifact)?;
                let report = serde_json::json!({"process":"succeeded","convergence":"analytical","numerical_error":result.numerical_error,
                    "mean_velocity_m_s":result.mean_velocity_m_s,"physical_validation":"unqualified",
                    "moisture_risk":"missing humidity, surface temperature and operating history; screening only",
                    "limitations":["synthetic fixture","not a numerical solver run","no cooling, ingress, damage or lifetime claim"],
                    "science_id":c.science_id()?,"execution_id":plan.id()?,"presentation_id":digest(&c.presentation)?});
                store.add_artifact(
                    id,
                    &commit_artifact(
                        &dir,
                        "validation.json",
                        &serde_json::to_vec_pretty(&report)?,
                        "json",
                        "independent analytical reference",
                    )?,
                )?;
            }
            StageOperation::ThermalProjection => {
                let spec = plan
                    .thermal_contact
                    .as_ref()
                    .ok_or_else(|| invalid("approved native coupling required"))?;
                let (_, report) = crate::thermal_contact::derive(spec, id, &native_work)?;
                let working = native_work.join("stages/projection");
                private_dir(&working)?;
                commit_artifact(
                    &working,
                    "projection-receipt.json",
                    &serde_json::to_vec_pretty(&report)?,
                    "json",
                    "source-bound complete native capacitance projection and original six-surface moisture assessments",
                )?;
            }
            StageOperation::Bundle => {
                if native_pending {
                    let mut artifacts = ingest_native_tree(
                        &native_work,
                        &dir,
                        plan.observation.max_artifact_bytes,
                    )?;
                    annotate_native_artifacts(&plan, &mut artifacts)?;
                    for artifact in artifacts {
                        store.add_artifact(id, &artifact)?;
                    }
                    fs::remove_dir_all(&native_work)?;
                    native_pending = false;
                }
                let bundle = serde_json::json!({"schema_version":1,"relative_paths":true,"plan":plan,
                    "artifacts":store.artifacts(id)?,"fleetix_revision":FLEETIX_REV,"fleetix_contract_digest":fleetix_digest(),
                    "physical_validation":"unqualified","private_geometry_omitted":false});
                store.add_artifact(
                    id,
                    &commit_artifact(
                        &dir,
                        "bundle.json",
                        &serde_json::to_vec_pretty(&bundle)?,
                        "json",
                        "offline bundle index; copy entire job directory",
                    )?,
                )?;
            }
            _ => {
                if !native_pending {
                    private_dir(&native_work)?;
                    let runtime = packaged_file(Path::new(
                        profile
                            .native_runtime
                            .as_deref()
                            .ok_or_else(|| invalid("native runtime"))?,
                    ))?;
                    store.add_artifact(id, &commit_artifact(&dir, "native-runtime.json", &read_bounded(&runtime, MAX_MESSAGE)?, "json",
                        &format!("exact immutable runtime manifest from {}; store paths bind packaged executable closures", runtime.display()))?)?;
                    let mut normalized = plan.clone();
                    if let Some(c) = &mut normalized.case {
                        for (quantity, dimension, unit) in [
                            (&mut c.length, "length", "m"),
                            (&mut c.channel_height, "length", "m"),
                            (&mut c.geometry_tolerance, "length", "m"),
                            (&mut c.kinematic_viscosity, "kinematic_viscosity", "m2/s"),
                            (&mut c.acceleration, "acceleration", "m/s2"),
                            (&mut c.material.density, "density", "kg/m3"),
                        ] {
                            quantity.value = quantity.si(dimension)?;
                            quantity.unit = unit.into();
                        }
                    }
                    store.add_artifact(id, &commit_artifact(
                        &dir,
                        "native-plan.json",
                        &serde_json::to_vec_pretty(&normalized)?,
                        "json",
                        "trusted SI adapter view; exposed only through read-only sandbox mount; original quantities in plan.json",
                    )?)?;
                    native_pending = true;
                }
                if matches!(stage.operation, StageOperation::NumericalFilter) {
                    let working = native_work.join("stages/filter");
                    private_dir(&working)?;
                    store.add_artifact(
                        id,
                        &commit_artifact(
                            &dir,
                            "native-filter-request.json",
                            &serde_json::to_vec_pretty(&crate::filters::descriptor(
                                store, id, &plan,
                            )?)?,
                            "json",
                            "trusted source-bound filter descriptor; mounted read-only",
                        )?,
                    )?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else if matches!(stage.operation, StageOperation::FemReference) {
                    let working = native_work.join("stages/fem");
                    private_dir(&working)?;
                    let spec = plan
                        .fem
                        .as_ref()
                        .ok_or_else(|| invalid("FEM recipe required"))?;
                    store.add_artifact(id,&commit_artifact(&dir,"native-fem-request.json",&serde_json::to_vec(spec)?,"json","exact SI native FEM descriptor; static solver parameter is not physical time")?)?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else if matches!(stage.operation, StageOperation::ThermalReference) {
                    let name = if plan.thermal_contact.is_some() {
                        stage.id.as_str()
                    } else {
                        "thermal"
                    };
                    let working = native_work.join(format!("stages/{name}"));
                    private_dir(&working)?;
                    let spec = if let Some(coupling) = &plan.thermal_contact {
                        coupling.thermal_stage(&stage.id)?
                    } else {
                        plan.thermal
                            .as_ref()
                            .ok_or_else(|| invalid("thermal recipe required"))?
                    };
                    store.add_artifact(id,&commit_artifact(&dir,&format!("native-{name}-request.json"),&serde_json::to_vec(spec)?,"json","exact SI prescribed thermal history; explicit native substeps and independent energy/retained output schedules")?)?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else if matches!(stage.operation, StageOperation::WettingReference) {
                    let working = native_work.join("stages/wetting");
                    private_dir(&working)?;
                    let spec = plan
                        .wetting
                        .as_ref()
                        .ok_or_else(|| invalid("wetting recipe required"))?;
                    store.add_artifact(id,&commit_artifact(&dir,"native-wetting-request.json",&serde_json::to_vec(spec)?,"json","exact SI synthetic planar wetting descriptor; fixed physical initial half-circle and interface width; native retained steps")?)?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else if matches!(stage.operation, StageOperation::ContactReference) {
                    let working = native_work.join("stages/contact");
                    private_dir(&working)?;
                    let coupled = if let Some(spec) = &plan.thermal_contact {
                        let (contact, report) =
                            crate::thermal_contact::derive(spec, id, &native_work)?;
                        let recorded: serde_json::Value = serde_json::from_slice(&read_bounded(
                            &safe_path(&native_work, "stages/projection/projection-receipt.json")?,
                            MAX_MESSAGE,
                        )?)?;
                        if recorded != report {
                            return Err(invalid(
                                "native sources or approved coupling projection changed before contact",
                            ));
                        }
                        Some(contact)
                    } else {
                        None
                    };
                    let spec = coupled
                        .as_ref()
                        .or(plan.contact.as_ref())
                        .ok_or_else(|| invalid("contact recipe required"))?;
                    store.add_artifact(id, &commit_artifact(&dir,"native-contact-request.json",&serde_json::to_vec(spec)?,"json","exact approved synthetic SI planar contact/preload/temperature inputs; two static parameters without inferred physical time")?)?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else if stage.operation == StageOperation::FreezingReference
                    && plan.retained_cooling.is_some()
                {
                    let working = native_work.join("stages/retained-cooling");
                    private_dir(&working)?;
                    let spec = plan
                        .retained_cooling
                        .as_ref()
                        .ok_or_else(|| invalid("source-bound cooling recipe required"))?;
                    store.add_artifact(id, &commit_artifact(&dir, "native-retained-cooling-request.json", &serde_json::to_vec(&spec.native_request()?)?, "json", "exact independently approved source-bound retained-water cooling native SI envelope")?)?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else if stage.operation == StageOperation::FreezingReference {
                    let working = native_work.join("stages/freezing");
                    private_dir(&working)?;
                    let spec = plan
                        .freezing
                        .as_ref()
                        .ok_or_else(|| invalid("approved solidification recipe required"))?;
                    store.add_artifact(id, &commit_artifact(&dir, "native-freezing-request.json",
                        &serde_json::to_vec(spec)?, "json", "exact synthetic SI total-enthalpy solidification inputs, nodal control volumes and retained physical times")?)?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else if stage.operation == StageOperation::SpectralReference
                    && plan.cad_transport.is_some()
                {
                    let working =
                        native_work.join(format!("stages/{}", crate::cad_transport::STAGE));
                    private_dir(&working)?;
                    crate::cad_transport::registered(store, id, &plan)?;
                    let spec = plan
                        .cad_transport
                        .as_ref()
                        .ok_or_else(|| invalid("approved original-CAD optical recipe required"))?;
                    store.add_artifact(id,&commit_artifact(&dir,"native-cad-spectral-request.json",&serde_json::to_vec(&spec.native_request()?)?,"json","exact independently approved original CAD materials, collimated spectral source, geometry budget and prescribed history")?)?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else if stage.operation == StageOperation::SpectralReference {
                    let working = native_work.join("stages/spectral");
                    private_dir(&working)?;
                    let spec = plan
                        .spectral
                        .as_ref()
                        .ok_or_else(|| invalid("approved directional spectral recipe required"))?;
                    store.add_artifact(id,&commit_artifact(&dir,"native-spectral-request.json",&serde_json::to_vec(spec)?,"json","exact approved angular UV source, SI sensor, spectral optical weights, retained seeds and complete prescribed dose history")?)?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else if stage.operation == StageOperation::AtmosphericReference {
                    let working = native_work.join("stages/atmosphere");
                    private_dir(&working)?;
                    let spec = plan
                        .atmosphere
                        .as_ref()
                        .ok_or_else(|| invalid("approved atmospheric recipe required"))?;
                    store.add_artifact(id,&commit_artifact(&dir,"native-atmosphere-request.json",&serde_json::to_vec(spec)?,"json","exact original UV source/profile, solar direction and full anisotropic angular grid; no inferred physical time")?)?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else if matches!(stage.operation, StageOperation::FemImported) {
                    let working = native_work.join("stages/fem-imported");
                    private_dir(&working)?;
                    store.add_artifact(id,&commit_artifact(&dir,"native-fem-imported-request.json",&serde_json::to_vec(&crate::fem_imported::descriptor(&plan)?)?,"json","exact original CAD geometry, explicit reference/material/boundary provenance; no invented physical time")?)?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else if stage.operation == StageOperation::AtmosphericTransport {
                    let working = native_work.join("stages/atmospheric-transport");
                    private_dir(&working)?;
                    let spec = plan.atmospheric_transport.as_ref().ok_or_else(|| {
                        invalid("approved registered atmospheric transport recipe required")
                    })?;
                    crate::atmospheric_transport::registered_source(store, spec)?;
                    store.add_artifact(id,&commit_artifact(&dir,"native-atmospheric-spectral-request.json",&serde_json::to_vec(&spec.native_request()?)?,"json","exact original atmospheric angular fields, registered-source checksum, SI receiver and prescribed optical history")?)?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else if matches!(stage.operation, StageOperation::CadMesh) {
                    let working = native_work.join("stages/mesh");
                    private_dir(&working)?;
                    let source = plan
                        .cad_source
                        .as_ref()
                        .ok_or_else(|| invalid("approved imported CAD source required"))?;
                    store.add_artifact(id,&commit_artifact(&dir,"native-cad-mesh-request.json",&serde_json::to_vec(&source.geometry)?,"json","exact approved SI imported CAD mesh descriptor; original source retained separately")?)?;
                    native_stage(store, profile, &plan, stage, &working, id)?;
                } else {
                    native_stage(store, profile, &plan, stage, &native_work, id)?;
                }
                if matches!(stage.operation, StageOperation::Openlb)
                    && plan
                        .stages
                        .iter()
                        .any(|s| matches!(s.operation, StageOperation::Render))
                {
                    let root = safe_path(&dir, "retained-fields")?;
                    let snapshot = crate::fields::capture(
                        &native_work,
                        &root,
                        &plan,
                        &digest(&store.execution_binding(id)?)?,
                    )?;
                    store.add_artifacts(id, &crate::fields::manifests(&root, &snapshot)?)?;
                    store.event(id, "fields_retained", &serde_json::json!({"science_id":snapshot.science_id,"artifact_id":snapshot.artifact_id,"times":snapshot.times}).to_string())?;
                    // Release only the copied authoritative graph. Other native
                    // collections (including material-only output) remain for export.
                    for record in &snapshot.files {
                        if record.path.starts_with("tmp/vtkData/") {
                            fs::remove_file(safe_path(&native_work, &record.path)?)?;
                        }
                    }
                }
            }
        }
        if disk_bytes(&dir, true)? > plan.observation.max_artifact_bytes {
            return Err(Error::Resource(
                "artifact budget exhausted; scientific data retained, job failed".into(),
            ));
        }
        store.event(id, "stage_finished", &stage.id)?;
    }
    if native_pending {
        let mut artifacts =
            ingest_native_tree(&native_work, &dir, plan.observation.max_artifact_bytes)?;
        annotate_native_artifacts(&plan, &mut artifacts)?;
        for artifact in artifacts {
            store.add_artifact(id, &artifact)?;
        }
        fs::remove_dir_all(&native_work)?;
    }
    Ok(())
}

fn annotate_native_artifacts(
    plan: &ExecutionPlan,
    artifacts: &mut [crate::contracts::ArtifactManifest],
) -> Result<()> {
    crate::wetting::annotate_fields(plan, artifacts)?;
    crate::freezing::annotate_fields(plan, artifacts)?;
    crate::cooling_fields::annotate(plan, artifacts)?;
    crate::cooling_vtk::annotate(plan, artifacts)?;
    crate::radiation::annotate_fields(plan, artifacts)?;
    crate::atmosphere::annotate_fields(plan, artifacts)?;
    crate::cad_transport_fields::annotate(plan, artifacts)?;
    crate::atmospheric_transport::annotate_fields(plan, artifacts)?;
    crate::contact::annotate_fields(plan, artifacts);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{disk_bytes, packaged_file, read_native_receipt, validate_native_receipt};
    use crate::contracts::{CaseSpec, ExecutionPlan};
    use std::{fs, path::Path};

    #[test]
    fn bundled_native_atmospheric_originals_retain_registered_units_and_association() {
        let (original, spec, _) = crate::atmosphere_fields::tests::fixture();
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        let stage = work.join("stages/atmosphere");
        fs::create_dir_all(&stage).unwrap();
        fs::copy(
            original.path().join("uvspec-original.txt"),
            stage.join("uvspec-original.txt"),
        )
        .unwrap();
        let committed = root.path().join("artifacts");
        fs::create_dir(&committed).unwrap();
        let plan = ExecutionPlan::atmospheric_reference(spec, "research".into()).unwrap();
        let mut artifacts = crate::storage::ingest_native_tree(
            &work,
            &committed,
            plan.observation.max_artifact_bytes,
        )
        .unwrap();
        super::annotate_native_artifacts(&plan, &mut artifacts).unwrap();
        assert_eq!(artifacts.len(), 1);
        let original = &artifacts[0];
        assert_eq!(original.path, crate::atmosphere::ORIGINAL_PATH);
        assert_eq!(
            original.association.as_deref(),
            Some(crate::atmosphere::FIELD_ASSOCIATION)
        );
        assert_eq!(
            original.units.as_deref(),
            Some(crate::atmosphere::FIELD_UNITS)
        );
        assert!(original.time_s.is_none());
        assert!(
            original
                .provenance
                .contains(crate::atmosphere::SOURCE_SHA256)
        );
    }

    #[test]
    fn lexical_store_prefix_cannot_authorize_mutable_native_code() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("manifest.json");
        fs::write(&outside, b"{}").unwrap();
        let escape = Path::new("/nix/store/../..").join(outside.strip_prefix("/").unwrap());
        assert!(matches!(
            packaged_file(&escape),
            Err(crate::Error::Invalid(_))
        ));
        assert!(matches!(
            packaged_file(&outside),
            Err(crate::Error::Invalid(_))
        ));
        assert!(packaged_file(Path::new("/nix/store")).is_err());
    }

    #[test]
    fn quarantined_unsafe_entries_are_counted_without_following_or_poisoning_admission() {
        let temp = tempfile::tempdir().unwrap();
        let raw = temp.path().join("quarantined");
        fs::create_dir(&raw).unwrap();
        let outside = temp.path().join("outside");
        fs::write(&outside, vec![0u8; 1048576]).unwrap();
        fs::write(raw.join("log"), b"fail").unwrap();
        std::os::unix::fs::symlink(&outside, raw.join("escape")).unwrap();
        assert!(disk_bytes(&raw, true).is_err());
        assert_eq!(
            disk_bytes(&raw, false).unwrap(),
            4 + fs::symlink_metadata(raw.join("escape")).unwrap().len()
        );
    }

    #[test]
    fn cpu_native_success_requires_matching_executed_adapter_receipts() {
        let mut case = CaseSpec::reference();
        case.applicability.formulation = "periodic_forced_channel".into();
        case.acceleration.value = 0.001;
        let plan = ExecutionPlan::openlb_reference(case, "research".into()).unwrap();
        let flow = &plan.stages[1];
        let valid = serde_json::json!({"adapter":"OpenLB","backend":"cpu","executed":true,"software_fallback":false});
        validate_native_receipt(flow, &valid).unwrap();
        assert!(validate_native_receipt(flow, &serde_json::json!({})).is_err());
        for (field, value) in [
            ("adapter", serde_json::json!("FreeCAD")),
            ("backend", serde_json::json!("cuda")),
            ("executed", serde_json::json!(false)),
            ("software_fallback", serde_json::json!(true)),
        ] {
            let mut corrupted = valid.clone();
            corrupted[field] = value;
            assert!(validate_native_receipt(flow, &corrupted).is_err());
        }
    }

    #[test]
    fn wetting_freezing_and_coupling_require_shared_authority_before_runtime_resolution() {
        let spec=serde_json::from_value(serde_json::json!({"schema_version":1,"synthetic":true,"backend":"cpu","formulation":"well_balanced_contact_angle_2d","diameter_m":48e-6,"initial_center_above_wall_m":0.,"resolution":24,"interface_width_m":6e-6,"density_liquid_kg_m3":1000.,"density_vapor_kg_m3":1000.,"viscosity_liquid_m2_s":1e-6,"viscosity_vapor_m2_s":1e-6,"surface_tension_n_m":1e-4,"contact_angle_deg":90.,"phase_relaxation_time":1.,"steps":100,"observation_steps":[0,100],"mass_tolerance":1e-3,"angle_tolerance_deg":5.,"material_provenance":"synthetic","boundary_provenance":"planar"})).unwrap();
        let plan = ExecutionPlan::wetting_reference(spec, "research".into()).unwrap();
        let root = tempfile::tempdir().unwrap();
        let store = crate::storage::Store::open(&root.path().join("state")).unwrap();
        let profile = crate::contracts::HostExecutionProfile {
            schema_version: 1,
            policy: "research".into(),
            allowed_input_root: root.path().to_str().unwrap().into(),
            max_ram_bytes: 2 * 1024 * 1024 * 1024,
            max_disk_bytes: 2 * 1024 * 1024 * 1024,
            threads: 1,
            timeout_seconds: 30,
            native_runtime: None,
            service_mode: "systemd".into(),
        };
        let coupling = ExecutionPlan::thermal_contact(
            serde_json::from_str(include_str!("../examples/thermal-contact.json")).unwrap(),
            "research".into(),
        )
        .unwrap();
        let freezing = ExecutionPlan::freezing_reference(
            serde_json::from_str(include_str!("../examples/freezing-reference.json")).unwrap(),
            "research".into(),
        )
        .unwrap();
        let atmosphere = ExecutionPlan::atmospheric_reference(
            serde_json::from_str(include_str!("../examples/atmosphere-reference.json")).unwrap(),
            "research".into(),
        )
        .unwrap();
        for plan in [plan, coupling, freezing, atmosphere] {
            let request = crate::contracts::Operation::Submit {
                approved_digest: plan.id().unwrap(),
                plan: Box::new(plan),
                idempotency_key: "no-authority".into(),
            };
            let error = super::dispatch(&store, &profile, None, request).unwrap_err();
            assert!(
                matches!(error, crate::Error::Unqualified(_))
                    && error.to_string().contains("authoritative"),
                "{error}"
            );
            assert!(store.active().unwrap().is_empty());
        }
    }

    #[test]
    fn native_receipt_rejects_special_symlink_and_oversized_records() {
        use std::{
            ffi::CString,
            os::unix::{ffi::OsStrExt, fs::symlink},
        };
        let temp = tempfile::tempdir().unwrap();
        let regular = temp.path().join("receipt.json");
        fs::write(&regular, br#"{"executed":true}"#).unwrap();
        assert_eq!(read_native_receipt(&regular).unwrap()["executed"], true);
        let link = temp.path().join("alias.json");
        symlink(&regular, &link).unwrap();
        assert!(read_native_receipt(&link).is_err());
        let fifo = temp.path().join("pipe.json");
        let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is NUL-terminated and lives for the complete call.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(read_native_receipt(&fifo).is_err());
        fs::write(&regular, vec![b' '; super::MAX_MESSAGE as usize + 1]).unwrap();
        assert!(read_native_receipt(&regular).is_err());
    }
}
