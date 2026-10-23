use crate::{
    Error, Result, authority::ExecutionAuthorization, contracts::*, execution::ExecutionBinding,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().min(i64::MAX as u64) as i64)
}
pub fn private_dir(root: &Path) -> Result<()> {
    if root.exists() {
        let m = fs::symlink_metadata(root)?;
        use std::os::unix::fs::MetadataExt;
        if !m.is_dir()
            || m.uid() != unsafe { libc::geteuid() }
            || m.permissions().mode() & 0o077 != 0
        {
            return Err(invalid(
                "state directory must be owned, nonsymlink and mode 0700",
            ));
        }
    } else {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)?;
    }
    Ok(())
}
pub fn safe_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let p = Path::new(relative);
    if relative.is_empty() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(invalid(
            "only relative non-traversing artifact paths allowed",
        ));
    }
    let mut out = root.to_path_buf();
    for component in p.components() {
        out.push(component);
        match fs::symlink_metadata(&out) {
            Ok(m) if m.file_type().is_symlink() => return Err(invalid("symlink in artifact path")),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}
pub fn commit_artifact(
    root: &Path,
    name: &str,
    data: &[u8],
    format: &str,
    provenance: &str,
) -> Result<ArtifactManifest> {
    let path = safe_path(root, name)?;
    let parent = path.parent().ok_or_else(|| invalid("artifact parent"))?;
    fs::create_dir_all(parent)?;
    let partial = parent.join(format!(".partial-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&partial)?;
        file.write_all(data)?;
        file.sync_all()?;
        // Same-filesystem, no-clobber commit. The temporary inode is already closed and immutable.
        fs::hard_link(&partial, &path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(ArtifactManifest {
            schema_version: 1,
            path: name.into(),
            sha256: format!("{:x}", Sha256::digest(data)),
            bytes: data.len() as u64,
            format: format.into(),
            provenance: provenance.into(),
            units: None,
            time_s: None,
            association: None,
        })
    })();
    let _ = fs::remove_file(&partial);
    result
}

/// Snapshot a scoped CAD source through one descriptor, with bounded streaming.
/// The importer receives only the private verified copy as a read-only mount.
pub fn snapshot_cad_input(
    source: &Path,
    root: &Path,
    expected_sha256: &str,
    maximum: u64,
) -> Result<ArtifactManifest> {
    let destination = safe_path(root, "input.FCStd")?;
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(source)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() {
        return Err(invalid("CAD source must be a regular file"));
    }
    if metadata.len() > maximum {
        return Err(Error::Resource(
            "CAD input exceeds approved scientific output allowance".into(),
        ));
    }
    let partial = root.join(format!(".partial-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&partial)?;
        let mut hash = Sha256::new();
        let mut count = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            count = count
                .checked_add(n as u64)
                .ok_or_else(|| invalid("CAD size overflow"))?;
            if count > maximum || count > metadata.len() {
                return Err(invalid("CAD source grew during snapshot"));
            }
            hash.update(&buffer[..n]);
            output.write_all(&buffer[..n])?;
        }
        let sha256 = format!("{:x}", hash.finalize());
        if count != metadata.len() || sha256 != expected_sha256 {
            return Err(invalid("source CAD checksum/size mismatch"));
        }
        output.sync_all()?;
        drop(output);
        fs::hard_link(&partial, &destination)?;
        fs::File::open(root)?.sync_all()?;
        Ok(ArtifactManifest {schema_version:1,path:"input.FCStd".into(),sha256,bytes:count,format:"FCStd".into(),
            provenance:"approved original CAD snapshot; distinct closed inode exposed read-only to the import sandbox".into(),
            units:None,time_s:None,association:None})
    })();
    let _ = fs::remove_file(partial);
    result
}
pub(crate) fn copy_verified(
    source: &Path,
    destination: &Path,
    artifact: &ArtifactManifest,
) -> Result<()> {
    let source = safe_path(source, &artifact.path)?;
    let destination = safe_path(destination, &artifact.path)?;
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(source)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() || metadata.len() != artifact.bytes {
        return Err(invalid("artifact type/size mismatch"));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| invalid("artifact parent"))?;
    fs::create_dir_all(parent)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&destination)?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| invalid("artifact size overflow"))?;
        if total > artifact.bytes {
            return Err(invalid("artifact grew during export"));
        }
        hash.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    if total != artifact.bytes || format!("{:x}", hash.finalize()) != artifact.sha256 {
        return Err(invalid("artifact checksum/size mismatch"));
    }
    output.sync_all()?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub(crate) fn publish_directory(partial: &Path, destination: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let partial =
        CString::new(partial.as_os_str().as_bytes()).map_err(|_| invalid("export path"))?;
    let destination =
        CString::new(destination.as_os_str().as_bytes()).map_err(|_| invalid("export path"))?;
    // Atomic whole-directory publication, with no replacement even if a competing
    // exporter creates the destination after the initial check.
    let status = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            partial.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if status != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
pub(crate) fn sync_directories(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_directories(&entry.path())?;
        }
    }
    fs::File::open(root)?.sync_all()?;
    Ok(())
}
pub fn ingest_native_tree(
    source: &Path,
    destination: &Path,
    maximum: u64,
) -> Result<Vec<ArtifactManifest>> {
    fn collect(
        root: &Path,
        prefix: &Path,
        paths: &mut Vec<String>,
        total: &mut u64,
        maximum: u64,
    ) -> Result<()> {
        if prefix.components().count() > 64 {
            return Err(Error::Resource(
                "bounded native output depth exhausted".into(),
            ));
        }
        for entry in fs::read_dir(root.join(prefix))? {
            let entry = entry?;
            let relative = prefix.join(entry.file_name());
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
                return Err(invalid("native output symlink/special file rejected"));
            }
            if metadata.is_dir() {
                collect(root, &relative, paths, total, maximum)?;
            } else {
                let name = relative
                    .to_str()
                    .ok_or_else(|| invalid("native output path encoding"))?;
                if name.ends_with(".partial")
                    || entry.file_name().to_string_lossy().starts_with('.')
                {
                    return Err(invalid("incomplete native output rejected"));
                }
                if name == "plan.json" {
                    continue;
                }
                *total = total
                    .checked_add(metadata.len())
                    .ok_or_else(|| invalid("native size overflow"))?;
                if *total > maximum || paths.len() >= 8192 {
                    return Err(Error::Resource(
                        "bounded native output ingestion budget exhausted".into(),
                    ));
                }
                paths.push(name.into());
            }
        }
        Ok(())
    }
    let mut paths = Vec::new();
    collect(source, Path::new(""), &mut paths, &mut 0, maximum)?;
    paths.sort();
    let mut artifacts = Vec::new();
    for path in paths {
        let manifest = native_manifest(
            source,
            &path,
            maximum,
            "closed isolated native output; model/device evidence in companion receipts",
        )?;
        copy_verified(source, destination, &manifest)?;
        artifacts.push(manifest);
    }
    sync_directories(destination)?;
    Ok(artifacts)
}
pub(crate) fn native_manifest(
    source: &Path,
    path: &str,
    maximum: u64,
    provenance: &str,
) -> Result<ArtifactManifest> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(safe_path(source, path)?)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(invalid("native output must be regular"));
    }
    let bytes = metadata.len();
    if bytes > maximum {
        return Err(Error::Resource("native snapshot budget exhausted".into()));
    }
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count = count
            .checked_add(n as u64)
            .ok_or_else(|| invalid("native size overflow"))?;
        if count > bytes || count > maximum {
            return Err(invalid("native output changed during ingestion"));
        }
        hash.update(&buffer[..n]);
    }
    if count != bytes {
        return Err(invalid("native output changed during ingestion"));
    }
    let format = Path::new(&path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("binary")
        .to_owned();
    Ok(ArtifactManifest {
        schema_version: 1,
        path: path.into(),
        sha256: format!("{:x}", hash.finalize()),
        bytes,
        format,
        provenance: provenance.into(),
        units: None,
        time_s: None,
        association: None,
    })
}

#[derive(Debug, Serialize)]
pub struct SkippedNativeFile {
    pub path: String,
    pub reason: String,
}
#[derive(Debug, Serialize)]
pub struct FailedNativeSnapshot {
    pub artifacts: Vec<ArtifactManifest>,
    pub skipped: Vec<SkippedNativeFile>,
}
/// Called only after adapter termination. Even .partial files are preserved as
/// opaque failed-attempt bytes, never promoted into successful scientific data.
/// Unsafe or over-budget entries stay quarantined in the raw tree and are
/// explicitly identified in the failure report rather than silently omitted.
pub fn retain_failed_native_tree(
    source: &Path,
    destination: &Path,
    maximum: u64,
) -> Result<FailedNativeSnapshot> {
    if !fs::symlink_metadata(source)?.is_dir() {
        return Err(invalid(
            "native failure root must be a nonsymlink directory",
        ));
    }
    fn collect(
        root: &Path,
        prefix: &Path,
        files: &mut Vec<String>,
        skipped: &mut Vec<SkippedNativeFile>,
        visited: &mut usize,
    ) -> Result<()> {
        if prefix.components().count() > 64 {
            return Err(invalid("native failure tree depth limit"));
        }
        for entry in fs::read_dir(root.join(prefix))? {
            let entry = entry?;
            *visited += 1;
            if *visited > 8192 {
                return Err(Error::Resource(
                    "failure inventory entry limit; raw tree preserved".into(),
                ));
            }
            let relative = prefix.join(entry.file_name());
            let path = relative
                .to_str()
                .ok_or_else(|| invalid("native failure path encoding"))?
                .to_owned();
            if path.len() > 4096 {
                return Err(invalid("native failure path length limit"));
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() {
                collect(root, &relative, files, skipped, visited)?;
            } else if metadata.is_file() {
                files.push(path);
            } else {
                skipped.push(SkippedNativeFile {
                    path,
                    reason: "unsafe symlink/special entry; raw tree quarantined".into(),
                });
            }
        }
        Ok(())
    }
    let target = safe_path(destination, "failed-native")?;
    private_dir(&target)?;
    let mut files = Vec::new();
    let mut snapshot = FailedNativeSnapshot {
        artifacts: Vec::new(),
        skipped: Vec::new(),
    };
    collect(
        source,
        Path::new(""),
        &mut files,
        &mut snapshot.skipped,
        &mut 0,
    )?;
    files.sort_by_key(|p| (!p.ends_with(".log") && !p.contains("receipt"), p.clone()));
    let mut remaining = maximum;
    for path in files {
        let saved = (|| {
            let mut manifest = native_manifest(
                source,
                &path,
                remaining,
                "failed native attempt; closed snapshot, partial files are opaque; no qualification implied",
            )?;
            copy_verified(source, &target, &manifest)?;
            manifest.path = format!("failed-native/{path}");
            Ok::<_, Error>(manifest)
        })();
        match saved {
            Ok(artifact) => {
                remaining -= artifact.bytes;
                snapshot.artifacts.push(artifact);
            }
            Err(error) => snapshot.skipped.push(SkippedNativeFile {
                path,
                reason: error.to_string(),
            }),
        }
    }
    snapshot.skipped.sort_by(|a, b| a.path.cmp(&b.path));
    sync_directories(&target)?;
    Ok(snapshot)
}
#[derive(Debug, Serialize)]
pub struct Job {
    pub id: String,
    pub plan_digest: String,
    pub state: String,
    pub unit: String,
    pub invocation_id: Option<String>,
    pub exit_code: Option<i32>,
    pub created_at: i64,
    pub error: Option<String>,
}
pub struct Store {
    pub connection: Connection,
    pub root: PathBuf,
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        private_dir(root)?;
        let db = safe_path(root, "jobs.sqlite3")?;
        let connection = Connection::open(db)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
          CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY, idem TEXT UNIQUE NOT NULL, digest TEXT NOT NULL,
            plan TEXT NOT NULL, state TEXT NOT NULL, unit TEXT NOT NULL, invocation TEXT, exit_code INTEGER,
            created INTEGER NOT NULL, error TEXT);
          CREATE TABLE IF NOT EXISTS events(seq INTEGER PRIMARY KEY AUTOINCREMENT, job TEXT NOT NULL,
            time INTEGER NOT NULL, kind TEXT NOT NULL, message TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS artifacts(job TEXT NOT NULL, path TEXT NOT NULL, manifest TEXT NOT NULL,
            PRIMARY KEY(job,path));
          CREATE TABLE IF NOT EXISTS job_profiles(job TEXT PRIMARY KEY, digest TEXT NOT NULL, profile TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS job_executions(job TEXT PRIMARY KEY, digest TEXT NOT NULL, binding TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS job_authorizations(job TEXT PRIMARY KEY, digest TEXT NOT NULL, authorization TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS studies(id TEXT PRIMARY KEY, idem TEXT UNIQUE NOT NULL, intent TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS designs(id TEXT PRIMARY KEY, idem TEXT UNIQUE NOT NULL, intent TEXT NOT NULL, state TEXT NOT NULL, error TEXT);
          CREATE TABLE IF NOT EXISTS design_candidates(design TEXT NOT NULL, idx INTEGER NOT NULL, candidate TEXT NOT NULL, PRIMARY KEY(design,idx));")?;
        Ok(Self {
            connection,
            root: root.into(),
        })
    }
    pub fn submit(&self, plan: &ExecutionPlan, key: &str) -> Result<Job> {
        self.submit_inner(plan, key, None, None, None)
    }
    pub fn submit_with_profile(
        &self,
        plan: &ExecutionPlan,
        key: &str,
        profile: &HostExecutionProfile,
    ) -> Result<Job> {
        self.submit_inner(plan, key, Some(profile), None, None)
    }
    pub fn submit_for_execution(
        &self,
        plan: &ExecutionPlan,
        key: &str,
        profile: &HostExecutionProfile,
        binding: &ExecutionBinding,
    ) -> Result<Job> {
        self.submit_inner(plan, key, Some(profile), Some(binding), None)
    }
    pub fn submit_authorized(
        &self,
        plan: &ExecutionPlan,
        key: &str,
        profile: &HostExecutionProfile,
        binding: &ExecutionBinding,
        authorization: &ExecutionAuthorization,
    ) -> Result<Job> {
        authorization.verify(plan, profile, binding)?;
        self.submit_inner(plan, key, Some(profile), Some(binding), Some(authorization))
    }
    pub fn existing_submission(
        &self,
        plan: &ExecutionPlan,
        key: &str,
        profile: &HostExecutionProfile,
    ) -> Result<Option<Job>> {
        let row: Option<(String, String)> = self
            .connection
            .query_row("SELECT id,digest FROM jobs WHERE idem=?1", [key], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        if let Some((id, prior)) = row {
            if prior != plan.id()? || digest(&self.job_profile(&id)?)? != digest(profile)? {
                return Err(Error::IdempotencyConflict);
            }
            return Ok(Some(self.job(&id)?));
        }
        Ok(None)
    }
    fn submit_inner(
        &self,
        plan: &ExecutionPlan,
        key: &str,
        profile: Option<&HostExecutionProfile>,
        binding: Option<&ExecutionBinding>,
        authorization: Option<&ExecutionAuthorization>,
    ) -> Result<Job> {
        plan.validate()?;
        if (plan.source.is_some()
            || plan.retained_cooling.is_some()
            || plan.cad_variant.is_some()
            || plan.cad_sheet_variant.is_some()
            || plan.cad_source.is_some()
            || plan.atmospheric_transport.is_some())
            && (profile.is_none() || binding.is_none() || authorization.is_none())
        {
            return Err(invalid(
                "source-bound submissions require immutable profile, execution and authorization",
            ));
        }
        if !token(key) {
            return Err(invalid("bounded idempotency key"));
        }
        let digest = plan.id()?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        if let Some((id, prior)) = tx
            .query_row("SELECT id,digest FROM jobs WHERE idem=?1", [key], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .optional()?
        {
            if prior != digest {
                return Err(Error::IdempotencyConflict);
            }
            if let Some(profile) = profile {
                let prior: Option<String> = tx
                    .query_row("SELECT digest FROM job_profiles WHERE job=?1", [&id], |r| {
                        r.get(0)
                    })
                    .optional()?;
                if prior.as_deref() != Some(&crate::contracts::digest(profile)?) {
                    return Err(Error::IdempotencyConflict);
                }
            }
            tx.commit()?;
            return self.job(&id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let unit = format!("harbor-cad-job-{id}.service");
        tx.execute("INSERT INTO jobs(id,idem,digest,plan,state,unit,created) VALUES(?1,?2,?3,?4,'queued',?5,?6)",
            params![id,key,digest,serde_json::to_string(plan)?,unit,now()])?;
        if let Some(profile) = profile {
            tx.execute(
                "INSERT INTO job_profiles(job,digest,profile) VALUES(?1,?2,?3)",
                params![
                    id,
                    crate::contracts::digest(profile)?,
                    serde_json::to_string(profile)?
                ],
            )?;
        }
        if let Some(binding) = binding {
            binding.verify(
                plan,
                profile.ok_or_else(|| invalid("execution profile required"))?,
            )?;
            crate::retention::prepare(
                &self.root,
                &id,
                binding,
                profile.is_some_and(|p| p.service_mode == "systemd"),
            )?;
            tx.execute(
                "INSERT INTO job_executions(job,digest,binding) VALUES(?1,?2,?3)",
                params![
                    id,
                    crate::contracts::digest(binding)?,
                    serde_json::to_string(binding)?
                ],
            )?;
        }
        if let Some(authorization) = authorization {
            tx.execute(
                "INSERT INTO job_authorizations(job,digest,authorization) VALUES(?1,?2,?3)",
                params![
                    id,
                    crate::contracts::digest(authorization)?,
                    serde_json::to_string(authorization)?
                ],
            )?;
        }
        crate::presentation::retain(self, &id, plan)?;
        crate::frames::retain(self, &id, plan)?;
        crate::cad_source::retain(self, &id, plan)?;
        crate::cad_variant::retain(self, &id, plan)?;
        crate::cad_transport::retain(self, &id, plan)?;
        crate::atmospheric_transport::retain(self, &id, plan)?;
        crate::cooling_execution::retain(self, &id, plan)?;
        tx.execute(
            "INSERT INTO events(job,time,kind,message) VALUES(?1,?2,'submitted',?3)",
            params![id, now(), digest],
        )?;
        tx.commit()?;
        self.job(&id)
    }
    pub fn job_profile(&self, id: &str) -> Result<HostExecutionProfile> {
        self.job(id)?;
        let row: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT digest,profile FROM job_profiles WHERE job=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (expected, data) = row.ok_or_else(|| {
            Error::Unqualified(
                "legacy job has no immutable host profile; no automatic relaunch".into(),
            )
        })?;
        let profile: HostExecutionProfile = serde_json::from_str(&data)?;
        if digest(&profile)? != expected {
            return Err(invalid("persisted host profile digest mismatch"));
        }
        Ok(profile)
    }
    pub fn execution_binding(&self, id: &str) -> Result<ExecutionBinding> {
        self.job(id)?;
        let row: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT digest,binding FROM job_executions WHERE job=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (expected, data) = row.ok_or_else(|| {
            Error::Unqualified("legacy job has no execution binding; no automatic relaunch".into())
        })?;
        let binding: ExecutionBinding = serde_json::from_str(&data)?;
        if digest(&binding)? != expected {
            return Err(invalid("persisted execution binding digest mismatch"));
        }
        Ok(binding)
    }
    pub fn execution_authorization(&self, id: &str) -> Result<Option<ExecutionAuthorization>> {
        self.job(id)?;
        let row: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT digest,authorization FROM job_authorizations WHERE job=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(expected, data)| {
            let authorization: ExecutionAuthorization = serde_json::from_str(&data)?;
            if digest(&authorization)? != expected {
                return Err(invalid("persisted execution authorization digest mismatch"));
            }
            Ok(authorization)
        })
        .transpose()
    }
    pub fn cleanup_retention(&self, mut closed: impl FnMut(&Job) -> Result<bool>) -> Result<()> {
        let parent = safe_path(&self.root, "retentions")?;
        if !parent.exists() {
            return Ok(());
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| invalid("retention job name"))?;
            if uuid::Uuid::parse_str(&id).is_err() || !entry.file_type()?.is_dir() {
                return Err(invalid("unexpected retention directory"));
            }
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1)",
                [&id],
                |r| r.get(0),
            )?;
            if !exists {
                crate::presentation::recover_orphan(self, &id)?;
                crate::cad_source::recover_orphan(self, &id)?;
                crate::cad_variant::recover_orphan(self, &id)?;
                crate::cad_transport::recover_orphan(self, &id)?;
                crate::atmospheric_transport::recover_orphan(self, &id)?;
                crate::cooling_execution::recover_orphan(self, &id)?;
                crate::retention::release(&self.root, &id, None)?;
            } else {
                let job = self.job(&id)?;
                if ["queued", "starting", "running", "cancelling"].contains(&job.state.as_str()) {
                    continue;
                }
                if closed(&job)? {
                    crate::retention::release(
                        &self.root,
                        &id,
                        Some(&self.execution_binding(&id)?),
                    )?;
                    self.event(
                        &id,
                        "runtime_released",
                        "terminal job and complete execution tree verified closed",
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn try_start(
        &self,
        id: &str,
        capacity: &HostExecutionProfile,
        retained_disk: u64,
    ) -> Result<bool> {
        let plan = self.plan(id)?;
        let disk = plan.disk_reservation()?;
        if plan.peak_ram() > capacity.max_ram_bytes || disk > capacity.max_disk_bytes {
            return Err(Error::Resource(
                "plan exceeds available RAM/staging capacity; parameters preserved".into(),
            ));
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        // Conservative default: one full-plan reservation per state root. A
        // terminal result may precede service-tree teardown, so its durable
        // runtime retention also owns capacity until verified cleanup.
        let active: i64 = tx.query_row(
            "SELECT count(*) FROM jobs WHERE state IN ('starting','running','cancelling')",
            [],
            |r| r.get(0),
        )?;
        if active != 0
            || retained_disk
                .checked_add(disk)
                .is_none_or(|n| n > capacity.max_disk_bytes)
        {
            return Ok(false);
        }
        let retentions = safe_path(&self.root, "retentions")?;
        if retentions.exists() {
            for entry in fs::read_dir(retentions)? {
                let entry = entry?;
                let retained_id = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| invalid("retention job name"))?;
                if uuid::Uuid::parse_str(&retained_id).is_err() || !entry.file_type()?.is_dir() {
                    return Err(invalid("unexpected retention directory"));
                }
                let state: Option<String> = tx
                    .query_row("SELECT state FROM jobs WHERE id=?1", [&retained_id], |r| {
                        r.get(0)
                    })
                    .optional()?;
                if state.is_some_and(|s| s != "queued") {
                    return Ok(false);
                }
            }
        }
        let started = tx.execute(
            "UPDATE jobs SET state='starting',error=NULL WHERE id=?1 AND state='queued'",
            [id],
        )? == 1;
        tx.commit()?;
        Ok(started)
    }
    pub fn job(&self, id: &str) -> Result<Job> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(invalid("job UUID required"));
        }
        let job: Job = self.connection.query_row(
            "SELECT id,digest,state,unit,invocation,exit_code,created,error FROM jobs WHERE id=?1",
            [id],
            |r| {
                Ok(Job {
                    id: r.get(0)?,
                    plan_digest: r.get(1)?,
                    state: r.get(2)?,
                    unit: r.get(3)?,
                    invocation_id: r.get(4)?,
                    exit_code: r.get(5)?,
                    created_at: r.get(6)?,
                    error: r.get(7)?,
                })
            },
        )?;
        if job.unit != format!("harbor-cad-job-{}.service", job.id) {
            return Err(invalid("persisted unit does not match owned job identity"));
        }
        Ok(job)
    }
    pub fn plan(&self, id: &str) -> Result<ExecutionPlan> {
        let plan = self.recorded_plan(id)?;
        plan.validate()?;
        Ok(plan)
    }
    pub(crate) fn recorded_plan(&self, id: &str) -> Result<ExecutionPlan> {
        let job = self.job(id)?;
        let data: String =
            self.connection
                .query_row("SELECT plan FROM jobs WHERE id=?1", [id], |r| r.get(0))?;
        let plan: ExecutionPlan = serde_json::from_str(&data)?;
        if plan.id()? != job.plan_digest {
            return Err(invalid("persisted immutable plan digest mismatch"));
        }
        Ok(plan)
    }
    pub fn event(&self, id: &str, kind: &str, message: &str) -> Result<()> {
        if message.len() > 4096 {
            return Err(invalid("event limit"));
        }
        self.connection.execute(
            "INSERT INTO events(job,time,kind,message) VALUES(?1,?2,?3,?4)",
            params![id, now(), kind, message],
        )?;
        Ok(())
    }
    pub fn transition(&self, id: &str, from: &str, to: &str, error: Option<&str>) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE jobs SET state=?1,error=?2 WHERE id=?3 AND state=?4",
            params![to, error, id, from],
        )? == 1)
    }
    pub fn finish(&self, id: &str, code: i32, error: Option<&str>) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        tx.execute("UPDATE jobs SET state=CASE WHEN state='cancelling' THEN 'cancelled' WHEN ?1=0 THEN 'succeeded' ELSE 'failed' END,exit_code=?1,error=?2 WHERE id=?3 AND state IN ('running','starting','cancelling')", params![code,error,id])?;
        tx.execute(
            "INSERT INTO events(job,time,kind,message) VALUES(?1,?2,'exit',?3)",
            params![id, now(), code.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn active(&self) -> Result<Vec<String>> {
        let mut stmt = self.connection.prepare("SELECT id FROM jobs WHERE state IN ('queued','starting','running','cancelling') ORDER BY created,rowid")?;
        Ok(stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub(crate) fn artifact_record(&self, id: &str, path: &str) -> Result<Option<ArtifactManifest>> {
        self.job(id)?;
        safe_path(&self.job_dir(id)?, path)?;
        let data: Option<String> = self
            .connection
            .query_row(
                "SELECT manifest FROM artifacts WHERE job=?1 AND path=?2",
                params![id, path],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|data| {
            if data.len() > 16384 {
                return Err(invalid("artifact descriptor exceeds 16 KiB"));
            }
            Ok(serde_json::from_str(&data)?)
        })
        .transpose()
    }
    pub fn artifacts(&self, id: &str) -> Result<Vec<ArtifactManifest>> {
        self.job(id)?;
        let mut stmt = self
            .connection
            .prepare("SELECT manifest FROM artifacts WHERE job=?1 ORDER BY path")?;
        let rows = stmt
            .query_map([id], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.iter().map(|s| Ok(serde_json::from_str(s)?)).collect()
    }
    pub fn add_artifact(&self, id: &str, manifest: &ArtifactManifest) -> Result<()> {
        self.job(id)?;
        if manifest.path.len() > 4096 {
            return Err(invalid("artifact path limit"));
        }
        safe_path(&self.root, &manifest.path)?;
        let data = serde_json::to_string(manifest)?;
        if data.len() > 16384 {
            return Err(invalid(
                "artifact descriptor limit is 16 KiB; retain extended metadata in companion files",
            ));
        }
        self.connection.execute(
            "INSERT INTO artifacts(job,path,manifest) VALUES(?1,?2,?3)",
            params![id, manifest.path, data],
        )?;
        Ok(())
    }
    pub(crate) fn add_artifacts(&self, id: &str, manifests: &[ArtifactManifest]) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        for manifest in manifests {
            self.add_artifact(id, manifest)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn artifact_page(&self, id: &str, after: Option<&str>, limit: u32) -> Result<ArtifactPage> {
        self.job(id)?;
        if limit == 0 || limit > 100 {
            return Err(invalid("artifact page limit 1..100"));
        }
        if let Some(cursor) = after {
            if cursor.len() > 4096 {
                return Err(invalid("artifact cursor limit"));
            }
            safe_path(&self.root, cursor)?;
        }
        let total: i64 = self.connection.query_row(
            "SELECT count(*) FROM artifacts WHERE job=?1",
            [id],
            |r| r.get(0),
        )?;
        let mut stmt = self.connection.prepare(
            "SELECT path,manifest FROM artifacts WHERE job=?1 AND path>?2 ORDER BY path LIMIT ?3",
        )?;
        let mut rows = stmt.query(params![id, after.unwrap_or(""), limit + 1])?;
        let mut items = Vec::new();
        let mut bytes = 0;
        let mut more = false;
        while let Some(row) = rows.next()? {
            let path: String = row.get(0)?;
            let data: String = row.get(1)?;
            if data.len() > 16384 {
                return Err(invalid("persisted artifact descriptor exceeds limit"));
            }
            if items.len() == limit as usize || bytes + data.len() > 24576 {
                more = true;
                break;
            }
            let item: ArtifactManifest = serde_json::from_str(&data)?;
            if item.path != path {
                return Err(invalid("persisted artifact path mismatch"));
            }
            bytes += data.len();
            items.push(item);
        }
        let next_after = if more {
            items.last().map(|a| a.path.clone())
        } else {
            None
        };
        Ok(ArtifactPage {
            items,
            total: total as u64,
            next_after,
        })
    }
    pub fn export(&self, id: &str, destination: &Path) -> Result<usize> {
        let job = self.job(id)?;
        if !["succeeded", "failed", "cancelled"].contains(&job.state.as_str()) {
            return Err(invalid("only terminal committed jobs may be exported"));
        }
        match fs::symlink_metadata(destination) {
            Ok(_) => return Err(invalid("export destination already exists")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let name = destination
            .file_name()
            .ok_or_else(|| invalid("export destination name"))?;
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = fs::canonicalize(parent)?;
        let destination = parent.join(name);
        let partial = parent.join(format!(".harbor-cad-export-{}", uuid::Uuid::new_v4()));
        private_dir(&partial)?;
        let result = (|| {
            let mut artifacts = self.artifacts(id)?;
            let registered_count = artifacts.len();
            let source = self.job_dir(id)?;
            for artifact in &artifacts {
                copy_verified(&source, &partial, artifact)?;
            }
            let profile = match self.job_profile(id) {
                Ok(profile) => Some(profile),
                Err(Error::Unqualified(_)) => None, // legacy jobs predate profile binding
                Err(error) => return Err(error),
            };
            let plan = self.recorded_plan(id)?;
            let binding = match self.execution_binding(id) {
                Ok(binding) => Some(binding),
                Err(Error::Unqualified(_)) => None,
                Err(error) => return Err(error),
            };
            let current = plan.validate();
            artifacts.push(commit_artifact(&partial, "execution.json", &serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version":1,"job":job,"plan":plan,"host_profile":profile,"execution_binding":binding,
                "execution_authorization":self.execution_authorization(id)?,
                "current_plan_check":{"accepted":current.is_ok(),"diagnostic":current.err().map(|e| e.diagnostic())},
                "physical_validation":"unqualified","registered_artifacts":registered_count,
                "completeness":"registered records only; native failure report identifies quarantined omissions"
            }))?, "json", "terminal execution status; export does not imply scientific qualification")?);
            commit_artifact(
                &partial,
                "manifest.json",
                &serde_json::to_vec_pretty(&artifacts)?,
                "json",
                "portable verified export",
            )?;
            sync_directories(&partial)?;
            publish_directory(&partial, &destination)?;
            fs::File::open(&parent)?.sync_all()?;
            Ok(registered_count)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&partial);
        }
        result
    }
    pub fn job_dir(&self, id: &str) -> Result<PathBuf> {
        self.job(id)?;
        let dir = safe_path(&self.root, &format!("artifacts/{id}"))?;
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }
    pub fn logs(&self, id: &str, after: u64, limit: u32) -> Result<serde_json::Value> {
        self.job(id)?;
        if limit == 0 || limit > 100 {
            return Err(invalid("log limit 1..100"));
        }
        let mut stmt = self.connection.prepare("SELECT seq,time,kind,message FROM events WHERE job=?1 AND seq>?2 ORDER BY seq LIMIT ?3")?;
        let after = i64::try_from(after).map_err(|_| invalid("log cursor outside SQLite range"))?;
        let rows = stmt.query_map(params![id,after,limit], |r| Ok(serde_json::json!({"seq":r.get::<_,i64>(0)?,"time":r.get::<_,i64>(1)?,"kind":r.get::<_,String>(2)?,"message":r.get::<_,String>(3)?})))?;
        Ok(serde_json::Value::Array(
            rows.collect::<std::result::Result<Vec<_>, _>>()?,
        ))
    }
}
