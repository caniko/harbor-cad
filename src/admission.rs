//! Same-user admission journal. Reservations outlive the scheduling worker.
use crate::{
    Error, Result,
    authority::HostAuthority,
    contracts::{digest, invalid},
    storage::{Job, Store, private_dir, safe_path},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs::{self, OpenOptions},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

pub fn shared_root() -> Result<PathBuf> {
    // HOME/XDG variables must not create separate same-user capacity pools.
    let mut buffer = vec![0u8; 65536];
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            libc::geteuid(),
            entry.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() {
        return Err(Error::Unqualified(
            "OS account home required for durable same-user admission".into(),
        ));
    }
    let entry = unsafe { entry.assume_init() };
    if entry.pw_dir.is_null() {
        return Err(invalid("OS account home missing"));
    }
    let home = PathBuf::from(std::ffi::OsStr::from_bytes(
        unsafe { std::ffi::CStr::from_ptr(entry.pw_dir) }.to_bytes(),
    ));
    if !home.is_absolute()
        || fs::canonicalize(&home)? != home
        || fs::metadata(&home)?.uid() != unsafe { libc::geteuid() }
    {
        return Err(invalid("canonical owned OS account home required"));
    }
    safe_path(&home, ".local/state/harbor-cad/admission")
}

/// The installed same-user KFD exclusion receipt, if any. Read-only over the
/// authoritative admission policy — the same trusted store `authority install`
/// writes — so a second live KFD GPU is admitted by operator policy alone,
/// never by an ad-hoc file beside a job. Absent store/policy yields `None`.
pub fn installed_kfd_exclusion() -> Result<Option<crate::devices::KfdExclusion>> {
    let root = shared_root()?;
    let db = root.join("admission.sqlite3");
    if !db.exists() {
        return Ok(None);
    }
    private_dir(&root)?;
    regular_private(&safe_path(&root, "admission.sqlite3")?)?;
    let connection = Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let data: Option<String> = connection
        .query_row("SELECT authority FROM policy WHERE id=1", [], |r| r.get(0))
        .optional()?;
    let Some(data) = data else {
        return Ok(None);
    };
    let authority: HostAuthority = serde_json::from_str(&data)?;
    Ok(authority.kfd_exclusion)
}

#[derive(Debug, Serialize, Deserialize)]
struct Reservation {
    root: String,
    id: String,
    authorization_digest: String,
    ram: u64,
    disk: u64,
    filesystem: u64,
    cards: BTreeMap<String, u64>,
}

pub struct Admission {
    connection: Connection,
    authority: HostAuthority,
    filesystems: BTreeMap<u64, usize>,
}

fn sum(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b)
        .ok_or_else(|| invalid("aggregate resource accounting overflow"))
}

fn regular_private(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file()
        || m.uid() != unsafe { libc::geteuid() }
        || m.permissions().mode() & 0o077 != 0
        || m.nlink() != 1
    {
        return Err(invalid(
            "admission database must be an owned private regular inode",
        ));
    }
    Ok(())
}

impl Admission {
    pub fn install(root: &Path, authority: &HostAuthority) -> Result<()> {
        let db = root.join("admission.sqlite3");
        if !db.exists() {
            Self::open(root, authority)?;
            return Ok(());
        }
        private_dir(root)?;
        regular_private(&safe_path(root, "admission.sqlite3")?)?;
        let connection =
            Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let data: String =
            connection.query_row("SELECT authority FROM policy WHERE id=1", [], |r| r.get(0))?;
        drop(connection);
        let recorded = serde_json::from_str(&data)?;
        let mut ledger = Self::open(root, &recorded)?;
        authority.validate()?;
        let mut filesystems = BTreeMap::new();
        for (index, budget) in authority.filesystems.iter().enumerate() {
            let path = Path::new(&budget.root);
            if fs::canonicalize(path)? != path
                || !path.is_dir()
                || filesystems
                    .insert(fs::metadata(path)?.dev(), index)
                    .is_some()
            {
                return Err(invalid(
                    "one canonical authority root/budget per filesystem required",
                ));
            }
        }
        ledger.authority = authority.clone();
        ledger.filesystems = filesystems;
        let tx = rusqlite::Transaction::new_unchecked(
            &ledger.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        if !Self::reservations(&tx)?.is_empty() {
            return Err(Error::Resource(
                "authority promotion requires verified release of every active reservation".into(),
            ));
        }
        let mut retained = BTreeMap::<u64, u64>::new();
        for state in Self::roots(&tx)? {
            let device = ledger.filesystem(&state)?;
            let total = retained.entry(device).or_default();
            *total = sum(*total, tree_bytes(&state, device, &mut 0, 0)?)?;
            if *total > authority.filesystems[ledger.filesystems[&device]].max_bytes {
                return Err(Error::Resource(
                    "retained data exceeds proposed filesystem budget".into(),
                ));
            }
        }
        tx.execute(
            "UPDATE policy SET digest=?1,authority=?2 WHERE id=1",
            params![digest(authority)?, serde_json::to_string(authority)?],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn verify_policy(&self, connection: &Connection) -> Result<()> {
        let current: String =
            connection.query_row("SELECT digest FROM policy WHERE id=1", [], |r| r.get(0))?;
        if current != digest(&self.authority)? {
            return Err(invalid(
                "same-user authority changed; bound jobs are not automatically upgraded",
            ));
        }
        Ok(())
    }
    pub fn open(root: &Path, authority: &HostAuthority) -> Result<Self> {
        authority.validate()?;
        private_dir(root)?;
        let mut filesystems = BTreeMap::new();
        for (index, budget) in authority.filesystems.iter().enumerate() {
            let path = Path::new(&budget.root);
            if fs::canonicalize(path)? != path
                || !path.is_dir()
                || filesystems
                    .insert(fs::metadata(path)?.dev(), index)
                    .is_some()
            {
                return Err(invalid(
                    "one canonical authority root/budget per filesystem required",
                ));
            }
        }
        // A WAL mode transition during first creation can return SQLITE_BUSY
        // without invoking SQLite's busy handler. Serialize bootstrap across
        // workers; later resource transactions still use the SQLite writer lease.
        let anchor = safe_path(root, "initialize.lock")?;
        let initialization = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&anchor)?;
        regular_private(&anchor)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match fs2::FileExt::try_lock_exclusive(&initialization) {
                Ok(()) => break,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    return Err(Error::Resource(
                        "shared admission initialization is busy".into(),
                    ));
                }
                Err(error) => return Err(error.into()),
            }
        }
        let db = safe_path(root, "admission.sqlite3")?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&db)?;
        regular_private(&db)?;
        for suffix in ["-wal", "-shm", "-journal"] {
            let path = safe_path(root, &format!("admission.sqlite3{suffix}"))?;
            if path.exists() {
                regular_private(&path)?;
            }
        }
        let connection = Connection::open(&db)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS policy(id INTEGER PRIMARY KEY CHECK(id=1), digest TEXT NOT NULL, authority TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS roots(path TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS reservations(id TEXT PRIMARY KEY, digest TEXT NOT NULL, record TEXT NOT NULL);")?;
        let tx = rusqlite::Transaction::new_unchecked(
            &connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let expected = digest(authority)?;
        let prior: Option<(String, String)> = tx
            .query_row("SELECT digest,authority FROM policy WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        match prior {
            Some((hash, data)) => {
                let recorded: HostAuthority = serde_json::from_str(&data)?;
                if hash != expected || digest(&recorded)? != hash {
                    return Err(invalid(
                        "conflicting same-user authority; refusing split capacity pools",
                    ));
                }
            }
            None => {
                tx.execute(
                    "INSERT INTO policy VALUES(1,?1,?2)",
                    params![expected, serde_json::to_string(authority)?],
                )?;
            }
        }
        tx.commit()?;
        file.sync_all()?;
        fs::File::open(root)?.sync_all()?;
        Ok(Self {
            connection,
            authority: authority.clone(),
            filesystems,
        })
    }

    fn filesystem(&self, path: &Path) -> Result<u64> {
        if !path.exists() {
            return Err(invalid(
                "registered state root missing; admission ownership is ambiguous",
            ));
        }
        private_dir(path)?;
        if fs::canonicalize(path)? != path {
            return Err(invalid("canonical admission state root required"));
        }
        let device = fs::metadata(path)?.dev();
        let index = self
            .filesystems
            .get(&device)
            .ok_or_else(|| invalid("state filesystem is not authorized"))?;
        if !path.starts_with(&self.authority.filesystems[*index].root) {
            return Err(invalid("state root outside authoritative filesystem scope"));
        }
        Ok(device)
    }

    pub fn register_state(&self, root: &Path) -> Result<()> {
        self.filesystem(root)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.verify_policy(&tx)?;
        let roots = Self::roots(&tx)?;
        if roots
            .iter()
            .any(|p| p != root && (p.starts_with(root) || root.starts_with(p)))
            || roots.len() >= 128 && !roots.contains(&root.to_path_buf())
        {
            return Err(invalid(
                "bounded nonoverlapping admission state roots required",
            ));
        }
        tx.execute(
            "INSERT OR IGNORE INTO roots(path) VALUES(?1)",
            [root.to_str().ok_or_else(|| invalid("UTF-8 state root"))?],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn roots(connection: &Connection) -> Result<Vec<PathBuf>> {
        let mut query = connection.prepare("SELECT path FROM roots ORDER BY path LIMIT 129")?;
        let roots = query
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if roots.len() > 128 {
            return Err(invalid("admission root limit"));
        }
        Ok(roots.into_iter().map(PathBuf::from).collect())
    }

    fn reservations(connection: &Connection) -> Result<Vec<Reservation>> {
        let mut query =
            connection.prepare("SELECT digest,record FROM reservations ORDER BY id LIMIT 4097")?;
        let rows = query
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if rows.len() > 4096 {
            return Err(invalid("admission reservation limit"));
        }
        rows.into_iter()
            .map(|(hash, data)| {
                let record: Reservation = serde_json::from_str(&data)?;
                if digest(&record)? != hash {
                    return Err(invalid("admission reservation digest mismatch"));
                }
                Ok(record)
            })
            .collect()
    }

    pub fn reserve(&self, store: &Store, id: &str) -> Result<bool> {
        let plan = store.plan(id)?;
        let profile = store.job_profile(id)?;
        let binding = store.execution_binding(id)?;
        let authorization = store.execution_authorization(id)?.ok_or_else(|| {
            invalid("shared admission requires immutable execution authorization")
        })?;
        authorization.verify(&plan, &profile, &binding)?;
        if digest(&authorization.authority)? != digest(&self.authority)? {
            return Err(invalid(
                "job authority differs from same-user admission policy",
            ));
        }
        let root = fs::canonicalize(&store.root)?;
        self.register_state(&root)?;
        let filesystem = self.filesystem(&root)?;
        let mut cards = BTreeMap::<String, u64>::new();
        for stage in &plan.stages {
            if let Some(selection) = &stage.selection {
                cards
                    .entry(selection.pci.clone())
                    .and_modify(|n| *n = (*n).max(stage.vram_bytes))
                    .or_insert(stage.vram_bytes);
            }
        }
        let request = Reservation {
            root: root
                .to_str()
                .ok_or_else(|| invalid("UTF-8 state root"))?
                .into(),
            id: id.into(),
            authorization_digest: digest(&authorization)?,
            ram: plan.peak_ram(),
            disk: plan.disk_reservation()?,
            filesystem,
            cards,
        };
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut ram = request.ram;
        self.verify_policy(&tx)?;
        let mut disk = request.disk;
        for active in Self::reservations(&tx)? {
            if active.id == id {
                if digest(&active)? != digest(&request)? {
                    return Err(invalid("admission identity changed"));
                }
                return Ok(true);
            }
            if active
                .cards
                .keys()
                .any(|key| request.cards.contains_key(key))
            {
                return Ok(false);
            }
            ram = sum(ram, active.ram)?;
            if active.filesystem == filesystem {
                disk = sum(disk, active.disk)?;
            }
        }
        if ram > self.authority.max_ram_bytes - self.authority.ram_headroom_bytes
            || sum(ram, self.authority.ram_headroom_bytes)? > available_ram()?
        {
            return Ok(false);
        }
        let budget = &self.authority.filesystems[self.filesystems[&filesystem]];
        let mut retained = 0;
        for state in Self::roots(&tx)? {
            if self.filesystem(&state)? == filesystem {
                retained = sum(retained, tree_bytes(&state, filesystem, &mut 0, 0)?)?;
            }
        }
        if sum(retained, disk)? > budget.max_bytes
            || sum(disk, budget.free_headroom_bytes)? > available_disk(&root)?
        {
            return Ok(false);
        }
        let keys: Vec<_> = request.cards.keys().cloned().collect();
        let _anchors = if keys.is_empty() {
            Vec::new()
        } else {
            let Some(anchors) = crate::resources::try_lock_cards(
                &crate::resources::card_reservation_root()?,
                &keys,
            )?
            else {
                return Ok(false);
            };
            anchors
        };
        for (pci, requested) in &request.cards {
            let card = self
                .authority
                .cards
                .iter()
                .find(|c| &c.pci == pci)
                .ok_or_else(|| invalid("card capacity missing"))?;
            let device = Path::new("/sys/bus/pci/devices").join(pci);
            let total: u64 = fs::read_to_string(device.join("mem_info_vram_total"))?
                .trim()
                .parse()
                .map_err(|_| invalid("VRAM total telemetry"))?;
            let used: u64 = fs::read_to_string(device.join("mem_info_vram_used"))?
                .trim()
                .parse()
                .map_err(|_| invalid("VRAM usage telemetry"))?;
            if card.max_vram_bytes > total {
                return Err(invalid(
                    "authority VRAM capacity exceeds observed physical card",
                ));
            }
            if sum(sum(*requested, card.headroom_bytes)?, used)? > total {
                return Ok(false);
            }
        }
        if store.job(id)?.state != "queued" {
            return Err(invalid(
                "only queued jobs may acquire new pre-launch admission",
            ));
        }
        tx.execute(
            "INSERT INTO reservations VALUES(?1,?2,?3)",
            params![id, digest(&request)?, serde_json::to_string(&request)?],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Serialize immutable input staging against the same filesystem accounting
    /// used for launch. The writer lease ends as soon as the copies are committed.
    pub(crate) fn retain_inputs<T>(
        &self,
        store: &Store,
        bytes: u64,
        commit: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let root = fs::canonicalize(&store.root)?;
        self.register_state(&root)?;
        let device = self.filesystem(&root)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.verify_policy(&tx)?;
        let budget = &self.authority.filesystems[self.filesystems[&device]];
        let mut occupied = bytes;
        for state in Self::roots(&tx)? {
            if self.filesystem(&state)? == device {
                occupied = sum(occupied, tree_bytes(&state, device, &mut 0, 0)?)?;
            }
        }
        for reservation in Self::reservations(&tx)? {
            if reservation.filesystem == device {
                occupied = sum(occupied, reservation.disk)?;
            }
        }
        if occupied > budget.max_bytes
            || sum(bytes, budget.free_headroom_bytes)? > available_disk(&root)?
        {
            return Err(Error::Resource(
                "insufficient authoritative capacity to retain immutable source inputs".into(),
            ));
        }
        let result = commit()?;
        tx.commit()?;
        Ok(result)
    }

    pub fn verify(&self, store: &Store, id: &str) -> Result<()> {
        self.verify_policy(&self.connection)?;
        let authorization = store
            .execution_authorization(id)?
            .ok_or_else(|| invalid("execution authorization missing"))?;
        let recorded = Self::reservations(&self.connection)?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| invalid("durable pre-launch admission missing"))?;
        if recorded.root
            != fs::canonicalize(&store.root)?
                .to_str()
                .ok_or_else(|| invalid("UTF-8 state root"))?
            || recorded.authorization_digest != digest(&authorization)?
            || digest(&authorization.authority)? != digest(&self.authority)?
        {
            return Err(invalid("durable admission ownership mismatch"));
        }
        Ok(())
    }

    pub fn reconcile(&self, mut closed: impl FnMut(&Store, &Job) -> Result<bool>) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        for record in Self::reservations(&tx)? {
            let root = Path::new(&record.root);
            self.filesystem(root)?;
            let store = Store::open(root)?;
            let job = store.job(&record.id)?;
            self.verify(&store, &job.id)?;
            if !["queued", "starting", "running", "cancelling"].contains(&job.state.as_str())
                && closed(&store, &job)?
            {
                tx.execute("DELETE FROM reservations WHERE id=?1", [&job.id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Unregister state roots that no longer exist. Runs inside the same SQLite
    /// immediate transaction (writer lease) as every admission mutation; a dry
    /// run reads one consistent snapshot through a read-only connection. Existing
    /// roots are never touched. A missing root is refused while any durable
    /// reservation references it or any live same-user process still holds an
    /// open file or working directory beneath it (for example an unlinked
    /// `worker.lock`).
    pub fn prune(root: &Path, dry_run: bool) -> Result<PruneReport> {
        let mut report = PruneReport {
            dry_run,
            ..Default::default()
        };
        let db = root.join("admission.sqlite3");
        if !db.exists() {
            return Ok(report);
        }
        private_dir(root)?;
        regular_private(&safe_path(root, "admission.sqlite3")?)?;
        let read = Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        read.busy_timeout(std::time::Duration::from_secs(5))?;
        let Some(data) = read
            .query_row("SELECT authority FROM policy WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
        else {
            return Ok(report);
        };
        let ledger = if dry_run {
            None
        } else {
            Some(Self::open(root, &serde_json::from_str(&data)?)?)
        };
        let (connection, behavior) = match &ledger {
            Some(ledger) => (&ledger.connection, rusqlite::TransactionBehavior::Immediate),
            None => (&read, rusqlite::TransactionBehavior::Deferred),
        };
        let tx = rusqlite::Transaction::new_unchecked(connection, behavior)?;
        let reservations = Self::reservations(&tx)?;
        let open = live_paths()?;
        let mut opaque: Vec<_> = open
            .iter()
            .filter(|(_, p)| p.is_none())
            .map(|(pid, _)| pid)
            .collect();
        opaque.dedup();
        for state in Self::roots(&tx)? {
            let reason = match fs::symlink_metadata(&state) {
                Ok(_) => {
                    report
                        .kept
                        .push(PruneEntry::new(&state, "state root exists"));
                    continue;
                }
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    format!("state root cannot be inspected: {e}")
                }
                Err(_) => {
                    if reservations
                        .iter()
                        .any(|r| Path::new(&r.root).starts_with(&state))
                    {
                        "durable admission reservation references this root".into()
                    } else if let Some((pid, _)) = open
                        .iter()
                        .find(|(_, p)| p.as_ref().is_some_and(|p| p.starts_with(&state)))
                    {
                        format!("live process {pid} holds an open file beneath this root")
                    } else if !opaque.is_empty() {
                        format!("lock-holding processes {opaque:?} cannot be inspected")
                    } else {
                        if !dry_run {
                            tx.execute(
                                "DELETE FROM roots WHERE path=?1",
                                [state.to_str().ok_or_else(|| invalid("UTF-8 state root"))?],
                            )?;
                        }
                        report.pruned.push(PruneEntry::new(
                            &state,
                            "registered state root missing; no reservation or live holder",
                        ));
                        continue;
                    }
                }
            };
            report.refused.push(PruneEntry::new(&state, &reason));
        }
        tx.commit()?;
        Ok(report)
    }
}

#[derive(Debug, Default, Serialize)]
pub struct PruneReport {
    pub dry_run: bool,
    pub pruned: Vec<PruneEntry>,
    pub kept: Vec<PruneEntry>,
    pub refused: Vec<PruneEntry>,
}

#[derive(Debug, Serialize)]
pub struct PruneEntry {
    pub root: PathBuf,
    pub reason: String,
}

impl PruneEntry {
    fn new(root: &Path, reason: &str) -> Self {
        Self {
            root: root.into(),
            reason: reason.into(),
        }
    }
}

/// Paths open (descriptors and working directory) in live same-user processes;
/// `None` marks a lock holder (`/proc/locks`) whose descriptors are not
/// inspectable. Non-dumpable processes that hold no lock cannot own a state-root
/// lock and are skipped.
// ponytail: OFD locks report pid -1 and are not attributed; harbor-cad uses flock.
fn live_paths() -> Result<Vec<(u32, Option<PathBuf>)>> {
    let missing = |e: &std::io::Error| e.kind() == std::io::ErrorKind::NotFound;
    let locked: Vec<u32> = fs::read_to_string("/proc/locks")?
        .lines()
        .filter_map(|line| {
            line.split_whitespace()
                .filter(|t| *t != "->")
                .nth(4)?
                .parse()
                .ok()
        })
        .collect();
    let uid = unsafe { libc::geteuid() };
    let mut open = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse().ok()) else {
            continue;
        };
        let dir = entry.path();
        match fs::metadata(&dir) {
            Ok(m) if m.uid() == uid => {}
            Ok(_) => continue,
            Err(e) if missing(&e) => continue,
            Err(e) => return Err(e.into()),
        }
        let mut links = vec![dir.join("cwd")];
        match fs::read_dir(dir.join("fd")) {
            Ok(fds) => {
                for fd in fds {
                    match fd {
                        Ok(fd) => links.push(fd.path()),
                        Err(e) if missing(&e) => {}
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            Err(e) if missing(&e) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                if locked.contains(&pid) {
                    open.push((pid, None));
                }
                continue;
            }
            Err(e) => return Err(e.into()),
        }
        for link in links {
            match fs::read_link(&link) {
                Ok(target) => {
                    let target = target.to_string_lossy();
                    let target = target.strip_suffix(" (deleted)").unwrap_or(&target);
                    open.push((pid, Some(PathBuf::from(target))));
                }
                Err(e) if missing(&e) => {}
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    if locked.contains(&pid) {
                        open.push((pid, None));
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(open)
}

pub(crate) fn cards_reserved(root: &Path, pcis: &[String]) -> Result<bool> {
    if pcis.is_empty() || !root.exists() {
        return Ok(false);
    }
    private_dir(root)?;
    let db = safe_path(root, "admission.sqlite3")?;
    if !db.exists() {
        return Ok(false);
    }
    regular_private(&db)?;
    let connection = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    let keys = pcis
        .iter()
        .map(|p| crate::resources::pci_key(p))
        .collect::<Result<Vec<_>>>()?;
    Ok(Admission::reservations(&connection)?
        .iter()
        .any(|r| keys.iter().any(|p| r.cards.contains_key(p))))
}

fn tree_bytes(root: &Path, device: u64, entries: &mut usize, depth: usize) -> Result<u64> {
    if depth > 64 {
        return Err(invalid("admission tree depth limit"));
    }
    let mut bytes = 0;
    for entry in fs::read_dir(root)? {
        *entries += 1;
        if *entries > 65536 {
            return Err(invalid("admission filesystem inventory limit"));
        }
        let entry = entry?;
        let m = fs::symlink_metadata(entry.path())?;
        if m.dev() != device {
            return Err(invalid("nested mount crosses admitted filesystem"));
        }
        bytes = sum(
            bytes,
            if m.is_dir() {
                tree_bytes(&entry.path(), device, entries, depth + 1)?
            } else {
                m.len()
            },
        )?;
    }
    Ok(bytes)
}

fn available_ram() -> Result<u64> {
    let data = fs::read_to_string("/proc/meminfo")?;
    let value = data
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .ok_or_else(|| invalid("MemAvailable telemetry required"))?;
    let mut fields = value.split_whitespace();
    let bytes = fields
        .next()
        .and_then(|n| n.parse::<u64>().ok())
        .and_then(|n| n.checked_mul(1024))
        .ok_or_else(|| invalid("RAM telemetry overflow"))?;
    if fields.next() != Some("kB") || fields.next().is_some() {
        return Err(invalid("RAM telemetry units"));
    }
    Ok(bytes)
}

fn available_disk(path: &Path) -> Result<u64> {
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| invalid("filesystem path"))?;
    let mut value = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), value.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let value = unsafe { value.assume_init() };
    value
        .f_bavail
        .checked_mul(value.f_frsize)
        .ok_or_else(|| Error::Resource("filesystem free-space overflow".into()))
}
