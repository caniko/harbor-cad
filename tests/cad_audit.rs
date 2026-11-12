use harbor_cad::cad_audit::{self, Status};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
};

fn fixture(directory: &Path, xml: &str) -> PathBuf {
    let path = directory.join("source.FCStd");
    let script = r#"
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w", zipfile.ZIP_STORED) as archive:
    archive.writestr("Document.xml", sys.argv[2])
    archive.writestr("GuiDocument.xml", "<GuiDocument/>")
"#;
    let output = Command::new("python3")
        .args(["-c", script])
        .arg(&path)
        .arg(xml)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    path
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sentinel(directory: &Path) -> PathBuf {
    let path = directory.join("unrelated.txt");
    fs::write(&path, b"preserve these bytes\n").unwrap();
    path
}

#[test]
fn cli_reports_bounded_archive_inventory_only() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("executed");
    let xml = format!(
        r#"<Document SchemaVersion="4"><Objects Count="3">
<Object type="Part::Feature" name="housing"/>
<Object type="Spreadsheet::Sheet" name="Params"/>
<Object type="PartDesign::FeaturePython" name="embedded"/>
</Objects><ObjectData><Object name="embedded"><String value="__import__('pathlib').Path('{}').write_text('ran')"/></Object></ObjectData></Document>"#,
        marker.display()
    );
    let source = fixture(directory.path(), &xml);
    let sentinel = sentinel(directory.path());
    let original = fs::read(&source).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_harbor-cad"))
        .args(["cad", "audit"])
        .arg(&source)
        .env("UNZIP", "-Z")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "status={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "archive_inventory");
    assert_eq!(
        report["path"],
        source.canonicalize().unwrap().to_string_lossy().as_ref()
    );
    assert_eq!(report["sha256"], digest(&original));
    assert_eq!(report["bytes"], original.len() as u64);
    assert!(
        report["audit_method"]
            .as_str()
            .unwrap()
            .contains("inventory")
    );
    assert_eq!(report["object_types"]["Part::Feature"], 1);
    assert_eq!(report["object_types"]["Spreadsheet::Sheet"], 1);
    assert_eq!(report["object_types"]["PartDesign::FeaturePython"], 1);
    assert_eq!(report["native_opened"], false);
    assert_eq!(report["geometry_qualified"], false);
    assert_eq!(report["parametric_qualified"], false);
    assert!(!marker.exists());
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(fs::read(&sentinel).unwrap(), b"preserve these bytes\n");
}

#[test]
fn archive_inventory_counts_objects_and_never_executes_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("executed");
    let xml = format!(
        r#"<Document SchemaVersion="4"><Objects Count="3">
<Object type="Part::Feature" name="housing"/>
<Object type="Part::Feature" name="lid"/>
<Object type="Spreadsheet::Sheet" name="Params"/>
<Object type="PartDesign::FeaturePython" name="embedded"/>
</Objects><ObjectData><Object name="embedded"><String value="__import__('pathlib').Path('{}').write_text('ran')"/></Object></ObjectData></Document>"#,
        marker.display()
    );
    let source = fixture(directory.path(), &xml);
    let sentinel = sentinel(directory.path());
    let original = fs::read(&source).unwrap();

    let report = cad_audit::audit(&source).unwrap();
    assert_eq!(report.status, Status::ArchiveInventory);
    assert_eq!(
        report.path,
        source.canonicalize().unwrap().to_string_lossy()
    );
    assert_eq!(report.sha256, digest(&original));
    assert_eq!(report.bytes, original.len() as u64);
    assert_eq!(report.xml_sha256, Some(digest(xml.as_bytes())));
    assert!(report.audit_method.contains("archive"));
    assert!(report.audit_method.contains("xml"));
    assert_eq!(
        report.object_types,
        [
            ("Part::Feature".into(), 2),
            ("PartDesign::FeaturePython".into(), 1),
            ("Spreadsheet::Sheet".into(), 1),
        ]
        .into()
    );
    assert!(!report.native_opened);
    assert!(!report.geometry_qualified);
    assert!(!report.parametric_qualified);
    assert!(!marker.exists());
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(fs::read(&sentinel).unwrap(), b"preserve these bytes\n");
}

#[test]
fn lfs_pointer_is_reported_without_fetch_or_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("pointer.FCStd");
    let pointer = [
        b"version https://git-lfs.github.com/spec/v1\noid sha256:".as_slice(),
        "a".repeat(64).as_bytes(),
        b"\nsize 9999\n",
    ]
    .concat();
    fs::write(&source, &pointer).unwrap();
    let sentinel = sentinel(directory.path());

    let report = cad_audit::audit(&source).unwrap();
    assert_eq!(report.status, Status::LfsPointer);
    assert_eq!(
        report.path,
        source.canonicalize().unwrap().to_string_lossy()
    );
    assert_eq!(report.sha256, digest(&pointer));
    assert_eq!(report.bytes, pointer.len() as u64);
    assert!(report.audit_method.contains("blocked"));
    assert!(report.audit_method.contains("lfs"));
    assert!(report.audit_method.contains("no_xml_parse"));
    assert!(report.object_types.is_empty());
    assert_eq!(report.xml_sha256, None);
    assert!(!report.native_opened);
    assert!(!report.geometry_qualified);
    assert!(!report.parametric_qualified);
    assert_eq!(fs::read(&source).unwrap(), pointer);
    assert_eq!(fs::read(&sentinel).unwrap(), b"preserve these bytes\n");
}

#[test]
fn invalid_archives_and_documents_are_rejected_without_touching_files() {
    let directory = tempfile::tempdir().unwrap();
    let sentinel = sentinel(directory.path());
    for (name, xml) in [
        ("malformed", "<Document><Objects>"),
        (
            "wrong-root",
            r#"<NotDocument><Objects><Object type="Part::Feature"/></Objects></NotDocument>"#,
        ),
        (
            "untyped",
            r#"<Document><Objects><Object name="housing"/></Objects></Document>"#,
        ),
        (
            "metadata-impostor",
            r#"<Document><Objects/><ObjectData><Object type="Part::Feature"/></ObjectData></Document>"#,
        ),
        (
            "dtd",
            r#"<!DOCTYPE Document [<!ENTITY x "y">]><Document><Objects><Object type="Part::Feature"/></Objects></Document>"#,
        ),
    ] {
        let case = directory.path().join(name);
        fs::create_dir(&case).unwrap();
        let source = fixture(&case, xml);
        let original = fs::read(&source).unwrap();
        assert!(cad_audit::audit(&source).is_err(), "{name}");
        assert_eq!(fs::read(&source).unwrap(), original, "{name}");
    }

    let invalid = directory.path().join("invalid.FCStd");
    fs::write(&invalid, b"not a zip archive\n").unwrap();
    assert!(cad_audit::audit(&invalid).is_err());
    assert_eq!(fs::read(&invalid).unwrap(), b"not a zip archive\n");

    let oversize = directory.path().join("oversize.FCStd");
    File::create(&oversize)
        .unwrap()
        .set_len(512 * 1024 * 1024 + 1)
        .unwrap();
    assert!(cad_audit::audit(&oversize).is_err());
    assert_eq!(fs::read(&sentinel).unwrap(), b"preserve these bytes\n");
}

#[test]
fn compressed_xml_overflow_is_refused_without_changing_source() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("oversize-xml.FCStd");
    let output = Command::new("python3")
        .args([
            "-c",
            r#"
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w", zipfile.ZIP_DEFLATED) as archive:
    with archive.open("Document.xml", "w") as stream:
        for _ in range(65):
            stream.write(b"x" * 1024**2)
"#,
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(source.metadata().unwrap().len() < 100_000);
    let original = fs::read(&source).unwrap();
    let error = cad_audit::audit(&source).unwrap_err();
    assert!(error.to_string().contains("64 MiB XML bound"), "{error}");
    assert_eq!(fs::read(&source).unwrap(), original);
}

#[test]
fn fifo_source_is_refused_without_waiting_for_a_writer() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("fifo.FCStd");
    assert!(
        Command::new("mkfifo")
            .arg(&source)
            .status()
            .unwrap()
            .success()
    );
    assert!(cad_audit::audit(&source).is_err());
}
