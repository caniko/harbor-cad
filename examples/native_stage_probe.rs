//! Opt-in adapter probe using the production DRM binding and process containment.
//! Worker ownership/admission and multi-GPU exclusion remain separate gates.
use clap::Parser;
use harbor_cad::{
    contracts::Role,
    devices::{DrmSandbox, HipSandbox},
    lifecycle::NativeProcess,
    resources::{card_reservation_root, try_reserve_cards},
    storage::private_dir,
};
use std::{
    fs::{self, OpenOptions},
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Parser)]
struct Arguments {
    bwrap: PathBuf,
    executable: PathBuf,
    pci: String,
    work: PathBuf,
    plan: PathBuf,
    operation: String,
    #[arg(long)]
    uuid: Option<String>,
    #[arg(long, requires = "uuid")]
    inventory_only: bool,
    #[arg(long, requires = "inventory_only")]
    deny_render_node: bool,
}

fn packaged(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if path.components().any(|c| c == Component::ParentDir) {
        return Err("packaged path traversal".into());
    }
    let path = path.canonicalize()?;
    if !path.starts_with("/nix/store") || !path.is_file() {
        return Err("immutable packaged regular executable required".into());
    }
    Ok(path)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Arguments::parse();
    if !matches!(args.operation.as_str(), "render" | "video" | "openlb") {
        return Err("only render/video/openlb probes supported".into());
    }
    if args.inventory_only && args.operation != "openlb" {
        return Err("HIP inventory probe requires the OpenLB operation".into());
    }
    let bwrap = packaged(&args.bwrap)?;
    let executable = packaged(&args.executable)?;
    private_dir(&args.work)?;
    let work = args.work.canonicalize()?;
    let plan = args.plan.canonicalize()?;
    let root = card_reservation_root()?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let _reservation = loop {
        if let Some(held) = try_reserve_cards(&root, std::slice::from_ref(&args.pci))? {
            break held;
        }
        if Instant::now() >= deadline {
            return Err("selected physical card remained reserved".into());
        }
        thread::sleep(Duration::from_millis(100));
    };
    let mut command = Command::new(bwrap);
    command
        .env_clear()
        .args([
            "--unshare-all",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
            "--clearenv",
            "--ro-bind",
            "/nix/store",
            "/nix/store",
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
            "PATH",
            "/nonexistent",
            "--setenv",
            "OMP_NUM_THREADS",
            "1",
            "--setenv",
            "LANG",
            "C.UTF-8",
            "--bind",
        ])
        .arg(&work)
        .args(["/work", "--chdir", "/work"]);
    let (binding, sandbox_policy) = if args.operation == "openlb" {
        let binding = HipSandbox::resolve(
            &args.pci,
            args.uuid.as_deref().ok_or("exact HIP UUID required")?,
        )?;
        binding.apply(&mut command);
        (serde_json::to_value(binding)?, None)
    } else {
        let binding = DrmSandbox::resolve(&args.pci)?;
        let policy = if args.operation == "render" {
            binding.apply(Role::Render, "egl", &mut command)?
        } else {
            binding.apply(Role::Media, "vaapi", &mut command)?
        };
        (serde_json::to_value(binding)?, Some(policy))
    };
    if args.deny_render_node {
        command.args(["--tmpfs", "/dev/dri"]);
    }
    command
        .arg("--ro-bind")
        .arg(plan)
        .arg("/plan.json")
        .arg("--")
        .arg(executable);
    if args.inventory_only {
        command.arg("--gpu-inventory");
    } else {
        command.arg(&args.operation).arg("/plan.json");
    }
    let log = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(work.join(format!("{}-native.log", args.operation)))?;
    command
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    unsafe {
        command.pre_exec(|| {
            let limit = libc::rlimit {
                rlim_cur: 64 * 1024 * 1024,
                rlim_max: 64 * 1024 * 1024,
            };
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut process = NativeProcess::spawn(command)?;
    let status = process.wait(Duration::from_secs(160), || Ok(()))?;
    let membership = fs::read_to_string("/proc/self/cgroup")?;
    let group = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or("cgroup v2 resource evidence required")?;
    let counters = Path::new("/sys/fs/cgroup").join(group.trim_start_matches('/'));
    let peak_bytes: u64 = fs::read_to_string(counters.join("memory.peak"))?
        .trim()
        .parse()?;
    let cpu = fs::read_to_string(counters.join("cpu.stat"))?;
    let cpu_usage_usec: u64 = cpu
        .lines()
        .find_map(|line| line.strip_prefix("usage_usec "))
        .ok_or("cgroup CPU usage evidence required")?
        .parse()?;
    println!(
        "{}",
        serde_json::json!({
            "binding": binding, "sandbox_policy": sandbox_policy, "cgroup": group, "memory_peak_bytes": peak_bytes,
            "cpu_usage_usec": cpu_usage_usec, "scope": "entire owned probe service",
            "physical_card_reservation": "production shared anchor held",
        })
    );
    if !status.success() {
        return Err(format!("native {} probe failed: {status}", args.operation).into());
    }
    Ok(())
}
