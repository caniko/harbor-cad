use crate::{Result, contracts::invalid, lifecycle::NativeProcess};
use roxmltree::{Document, ParsingOptions};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_XML: usize = 64 * 1024 * 1024;
const LFS_PREFIX: &[u8] = b"version https://git-lfs.github.com/spec/v1";
const ARCHIVE_METHOD: &str = "bounded_archive_unzip_roxmltree_xml_inventory_only";
const LFS_METHOD: &str = "blocked_lfs_pointer_no_xml_parse";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    ArchiveInventory,
    LfsPointer,
}

/// Archive/XML inventory only; no native CAD geometry or parametric proof.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Inventory {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub audit_method: String,
    pub object_types: BTreeMap<String, u64>,
    pub xml_sha256: Option<String>,
    pub status: Status,
    pub native_opened: bool,
    pub geometry_qualified: bool,
    pub parametric_qualified: bool,
}

struct Source {
    path: String,
    sha256: String,
    bytes: u64,
    lfs: bool,
}

pub fn audit(path: &Path) -> Result<Inventory> {
    let before = inspect(path)?;
    if before.lfs {
        unchanged(&before)?;
        return Ok(inventory(
            &before,
            LFS_METHOD,
            Status::LfsPointer,
            None,
            BTreeMap::new(),
        ));
    }
    let xml = document_xml(&before)?;
    let document = parse(&xml)?;
    unchanged(&before)?;
    Ok(inventory(
        &before,
        ARCHIVE_METHOD,
        Status::ArchiveInventory,
        Some(format!("{:x}", Sha256::digest(&xml))),
        document,
    ))
}

fn inventory(
    source: &Source,
    method: &str,
    status: Status,
    xml_sha256: Option<String>,
    object_types: BTreeMap<String, u64>,
) -> Inventory {
    Inventory {
        path: source.path.clone(),
        sha256: source.sha256.clone(),
        bytes: source.bytes,
        audit_method: method.into(),
        object_types,
        xml_sha256,
        status,
        native_opened: false,
        geometry_qualified: false,
        parametric_qualified: false,
    }
}

fn inspect(path: &Path) -> Result<Source> {
    let path = fs::canonicalize(path)?;
    let path_text = path
        .to_str()
        .ok_or_else(|| invalid("archive inventory requires a UTF-8 path"))?
        .to_owned();
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_ARCHIVE {
        return Err(invalid(
            "archive inventory requires a regular file of at most 512 MiB",
        ));
    }
    let mut hash = Sha256::new();
    let mut prefix = Vec::with_capacity(LFS_PREFIX.len());
    let mut bytes = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > metadata.len() {
            return Err(invalid("source changed while hashing"));
        }
        if prefix.len() < LFS_PREFIX.len() {
            let keep = count.min(LFS_PREFIX.len() - prefix.len());
            prefix.extend_from_slice(&buffer[..keep]);
        }
        hash.update(&buffer[..count]);
    }
    if bytes != metadata.len() {
        return Err(invalid("source changed while hashing"));
    }
    Ok(Source {
        sha256: format!("{:x}", hash.finalize()),
        path: path_text,
        bytes,
        lfs: prefix.starts_with(LFS_PREFIX),
    })
}

fn unchanged(before: &Source) -> Result<()> {
    let after = inspect(Path::new(&before.path))?;
    if after.sha256 != before.sha256 || after.bytes != before.bytes {
        return Err(invalid(
            "source changed during read-only archive/XML inventory",
        ));
    }
    Ok(())
}

fn document_xml(source: &Source) -> Result<Vec<u8>> {
    let unzip = option_env!("HARBOR_CAD_UNZIP").unwrap_or("unzip");
    let mut command = Command::new(unzip);
    command
        .arg("-p")
        .arg(&source.path)
        .arg("Document.xml")
        .env_remove("UNZIP")
        .env_remove("UNZIPOPT")
        .env_remove("ZIPINFO")
        .env_remove("ZIPINFOOPT");
    extract_xml(command, Duration::from_secs(10))
}

fn extract_xml(mut command: Command, timeout: Duration) -> Result<Vec<u8>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut process = NativeProcess::spawn(command)?;
    let stdout = process
        .take_stdout()
        .ok_or_else(|| invalid("archive extractor stdout unavailable"))?;
    let exceeded = Arc::new(AtomicBool::new(false));
    let reader_exceeded = exceeded.clone();
    let reader = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        stdout.take(MAX_XML as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_XML {
            reader_exceeded.store(true, Ordering::Relaxed);
        }
        Ok(bytes)
    });
    let status = process.wait(timeout, || {
        if exceeded.load(Ordering::Relaxed) {
            Err(invalid("Document.xml exceeds the 64 MiB XML bound"))
        } else {
            Ok(())
        }
    });
    let bytes = reader
        .join()
        .map_err(|_| invalid("Document.xml reader failed"))??;
    if exceeded.load(Ordering::Relaxed) || bytes.len() > MAX_XML {
        return Err(invalid("Document.xml exceeds the 64 MiB XML bound"));
    }
    let status = status?;
    if !status.success() {
        return Err(invalid(
            "archive lacks a readable Document.xml; archive/XML inventory only",
        ));
    }
    Ok(bytes)
}

fn parse(xml: &[u8]) -> Result<BTreeMap<String, u64>> {
    let text = std::str::from_utf8(xml).map_err(|_| invalid("Document.xml is not valid UTF-8"))?;
    let document = Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: false,
            ..Default::default()
        },
    )
    .map_err(|error| invalid(format!("Document.xml is not valid XML: {error}")))?;
    let root = document.root_element();
    if root.tag_name().name() != "Document" {
        return Err(invalid("Document.xml root must be Document"));
    }
    let mut declarations = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Objects");
    let objects = declarations
        .next()
        .ok_or_else(|| invalid("Document.xml requires an Objects declaration"))?;
    if declarations.next().is_some() {
        return Err(invalid("Document.xml has ambiguous Objects declarations"));
    }
    let mut types = BTreeMap::new();
    let mut count = 0u64;
    for object in objects
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Object")
    {
        let kind = object
            .attribute("type")
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .ok_or_else(|| invalid("Document.xml Object requires a non-empty type"))?;
        *types.entry(kind.into()).or_default() += 1;
        count += 1;
    }
    if count == 0 {
        return Err(invalid(
            "Document.xml requires typed Objects/Object declarations",
        ));
    }
    Ok(types)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn extractor_timeout_closes_descendant_stdout_and_joins_reader() {
        let mut command = Command::new("python3");
        command.args(["-c", "import os,time; os.fork(); time.sleep(30)"]);
        let start = Instant::now();
        let error = extract_xml(command, Duration::from_millis(50)).unwrap_err();
        assert!(
            error.to_string().contains("time budget exhausted"),
            "{error}"
        );
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn failed_extractor_cannot_supply_a_successful_inventory() {
        let mut command = Command::new("python3");
        command.args(["-c", "import sys; print('<Document/>'); sys.exit(2)"]);
        let error = extract_xml(command, Duration::from_secs(3)).unwrap_err();
        assert!(
            error.to_string().contains("readable Document.xml"),
            "{error}"
        );
    }
}
