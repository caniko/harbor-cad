//! Standalone presentation binds registered science, never an arbitrary file path.
use crate::{
    Result,
    contracts::*,
    fields::{self, FieldSnapshot},
    storage::*,
};
use std::{fs, path::PathBuf};

pub fn source(store: &Store, id: &str) -> Result<(PathBuf, FieldSnapshot, RetainedSource)> {
    let job = store.job(id)?;
    if job.state != "succeeded" {
        return Err(invalid(
            "presentation source must be a completed committed job",
        ));
    }
    let plan = store.plan(id)?;
    if plan.source.is_some()
        || !plan
            .stages
            .iter()
            .any(|s| matches!(s.operation, StageOperation::Openlb))
    {
        return Err(invalid("registered solver snapshot source required"));
    }
    let binding = store.execution_binding(id)?;
    let authorization = store
        .execution_authorization(id)?
        .ok_or_else(|| invalid("source execution authorization required"))?;
    authorization.verify(&plan, &store.job_profile(id)?, &binding)?;
    let (root, snapshot, hash) = fields::registered(store, id)?;
    if snapshot.execution_id != plan.id()?
        || snapshot.science_id != plan.science_id()?
        || snapshot.execution_binding_digest != digest(&binding)?
        || snapshot
            .times
            .iter()
            .map(|t| t.requested_s)
            .collect::<Vec<_>>()
            != plan.observation.retained_times_s
    {
        return Err(invalid(
            "source snapshot differs from original approved science/execution",
        ));
    }
    let bytes = fields::manifests(&root, &snapshot)?
        .iter()
        .try_fold(0u64, |sum, file| {
            sum.checked_add(file.bytes)
                .ok_or_else(|| invalid("source byte count overflow"))
        })?;
    let source = RetainedSource {
        job_id: id.into(),
        plan_digest: plan.id()?,
        execution_binding_digest: digest(&binding)?,
        authorization_digest: digest(&authorization)?,
        snapshot_sha256: hash,
        artifact_id: snapshot.artifact_id.clone(),
        science_id: snapshot.science_id.clone(),
        bytes,
    };
    Ok((root, snapshot, source))
}

pub fn plan(store: &Store, request: PresentationRequest, policy: String) -> Result<ExecutionPlan> {
    let (_, _, bound) = source(store, &request.source_job)?;
    ExecutionPlan::presentation(&store.plan(&request.source_job)?, bound, request, policy)
}

/// Called under the submission writer transaction, before acknowledgment. A
/// dependent job receives distinct verified inodes and its own registry records.
pub(crate) fn retain(store: &Store, id: &str, plan: &ExecutionPlan) -> Result<()> {
    let Some(expected) = &plan.source else {
        return Ok(());
    };
    let (root, snapshot, observed) = source(store, &expected.job_id)?;
    if digest(&observed)? != digest(expected)? {
        return Err(invalid("approved presentation source identity changed"));
    }
    let original = store.plan(&expected.job_id)?;
    if plan
        .observation
        .retained_times_s
        .iter()
        .any(|t| !original.observation.retained_times_s.contains(t))
    {
        return Err(invalid("presentation requests an unretained source time"));
    }
    let destination = store.job_dir(id)?.join("retained-fields");
    if destination.exists() {
        return Err(invalid("immutable source destination already exists"));
    }
    let intent = commit_artifact(
        &store.job_dir(id)?,
        "source-retention.json",
        &serde_json::to_vec(expected)?,
        "json",
        "durable immutable-input staging intent; unpublished copies are recoverable after submission interruption",
    )?;
    let staging = destination.with_file_name(format!(".retained-source-{}", uuid::Uuid::new_v4()));
    private_dir(&staging)?;
    // Failed submission copies remain under the durable intent. Recovery
    // removes them only after re-verifying the original authorized source.
    (|| {
        let records = fields::manifests(&root, &snapshot)?;
        if records
            .last()
            .is_none_or(|r| r.sha256 != expected.snapshot_sha256)
        {
            return Err(invalid("approved source snapshot changed during staging"));
        }
        for mut record in records.clone() {
            record.path = record
                .path
                .strip_prefix("retained-fields/")
                .ok_or_else(|| invalid("source record path"))?
                .into();
            copy_verified(&root, &staging, &record)?;
        }
        fields::verify(&staging, &snapshot)?;
        let manifest = native_manifest(&staging, "snapshot.json", 2 * 1024 * 1024, "verify")?;
        if manifest.sha256 != expected.snapshot_sha256 {
            return Err(invalid(
                "staged snapshot differs from exact approved source",
            ));
        }
        sync_directories(&staging)?;
        publish_directory(&staging, &destination)?;
        fs::File::open(
            destination
                .parent()
                .ok_or_else(|| invalid("source destination parent"))?,
        )?
        .sync_all()?;
        for record in records {
            store.add_artifact(id, &record)?;
        }
        let provenance = serde_json::json!({
            "source_job":expected.job_id, "plan":original,
            "host_profile":store.job_profile(&expected.job_id)?,
            "execution_binding":store.execution_binding(&expected.job_id)?,
            "execution_authorization":store.execution_authorization(&expected.job_id)?
        });
        store.add_artifact(
            id,
            &commit_artifact(
                &store.job_dir(id)?,
                "source-execution.json",
                &serde_json::to_vec_pretty(&provenance)?,
                "json",
                "original source approvals and execution authorization; retained without upgrades",
            )?,
        )?;
        store.add_artifact(id, &intent)?;
        Ok(())
    })()
}

/// A dropped submission transaction has no runnable job. Delete its unpublished
/// copies only after proving the original authorized source is still intact.
/// Otherwise retain the staging intent and bytes as quarantined recovery data.
pub(crate) fn recover_orphan(store: &Store, id: &str) -> Result<()> {
    let root = safe_path(&store.root, &format!("artifacts/{id}"))?;
    let intent = safe_path(&root, "source-retention.json")?;
    if !intent.exists() {
        return Ok(());
    }
    let data = crate::worker::read_bounded(&intent, MAX_MESSAGE)?;
    let expected: RetainedSource = serde_json::from_slice(&data)?;
    if let Ok((_, _, original)) = source(store, &expected.job_id)
        && digest(&original)? == digest(&expected)?
        && crate::frames::orphan_source_intact(store, &root)?
    {
        fs::remove_dir_all(&root)?;
        fs::File::open(root.parent().ok_or_else(|| invalid("orphan parent"))?)?.sync_all()?;
    }
    Ok(())
}

pub(crate) fn verify(plan: &ExecutionPlan, snapshot: &FieldSnapshot, hash: &str) -> Result<()> {
    let expected = plan
        .source
        .as_ref()
        .ok_or_else(|| invalid("presentation source missing"))?;
    if expected.snapshot_sha256 != hash
        || expected.artifact_id != snapshot.artifact_id
        || expected.science_id != snapshot.science_id
        || expected.plan_digest != snapshot.execution_id
        || expected.execution_binding_digest != snapshot.execution_binding_digest
        || plan
            .observation
            .retained_times_s
            .iter()
            .any(|t| !snapshot.times.iter().any(|s| s.requested_s == *t))
    {
        return Err(invalid(
            "retained presentation differs from approved source science/execution/times",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        authority::{ExecutionAuthorization, HostAuthority},
        execution::{ExecutionBinding, FileIdentity},
    };

    fn archived_source(store: &Store) -> String {
        // An archived-record fixture for storage/authorization checks. This is
        // not native solver or hardware qualification evidence.
        let mut case = CaseSpec::reference();
        case.length.value = 0.02;
        case.acceleration.value = 0.001;
        case.applicability.formulation = "periodic_forced_channel".into();
        let plan = ExecutionPlan::openlb_reference(case, "research".into()).unwrap();
        let profile = HostExecutionProfile {
            schema_version: 1,
            policy: "research".into(),
            allowed_input_root: store.root.to_string_lossy().into(),
            max_ram_bytes: 2 * 1024 * 1024 * 1024,
            max_disk_bytes: 1024 * 1024 * 1024,
            threads: 1,
            timeout_seconds: 60,
            native_runtime: None,
            service_mode: "systemd".into(),
        };
        let job = store
            .submit_with_profile(&plan, "archived-source", &profile)
            .unwrap();
        let binding = ExecutionBinding {
            schema_version: 1,
            runner_protocol: 1,
            sandbox_policy: crate::execution::SANDBOX_POLICY.into(),
            plan_digest: plan.id().unwrap(),
            host_profile_digest: digest(&profile).unwrap(),
            runner: FileIdentity {
                path: "/nix/store/archived-fixture/bin/runner".into(),
                sha256: "a".repeat(64),
                bytes: 1,
            },
            native_runtime: None,
            native_files: Default::default(),
        };
        let authority = HostAuthority {
            schema_version: 1,
            fleetix_revision: FLEETIX_REV.into(),
            fleetix_contract_digest: fleetix_digest(),
            max_ram_bytes: profile.max_ram_bytes,
            ram_headroom_bytes: 0,
            filesystems: vec![],
            cards: vec![],
            routes: vec![],
            allowed_devices: vec![],
            overrides: vec![],
            native_runtimes: vec![],
            allowed_input_roots: vec![profile.allowed_input_root.clone()],
            kfd_exclusion: None,
        };
        let authorization =
            ExecutionAuthorization::capture(&plan, &profile, &binding, &authority).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO job_executions VALUES(?1,?2,?3)",
                rusqlite::params![
                    job.id,
                    digest(&binding).unwrap(),
                    serde_json::to_string(&binding).unwrap()
                ],
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO job_authorizations VALUES(?1,?2,?3)",
                rusqlite::params![
                    job.id,
                    digest(&authorization).unwrap(),
                    serde_json::to_string(&authorization).unwrap()
                ],
            )
            .unwrap();
        let root = store.job_dir(&job.id).unwrap().join("retained-fields");
        private_dir(&root).unwrap();
        let field = commit_artifact(
            &root,
            "opaque-fixture.bin",
            b"archived closed bytes",
            "binary",
            "opaque registry fixture",
        )
        .unwrap();
        let snapshot = FieldSnapshot {
            schema_version: 1,
            science_id: plan.science_id().unwrap(),
            execution_id: plan.id().unwrap(),
            execution_binding_digest: digest(&binding).unwrap(),
            artifact_id: digest(&vec![field.clone()]).unwrap(),
            collection: "tmp/vtkData/channel.pvd".into(),
            times: plan
                .observation
                .retained_times_s
                .iter()
                .map(|t| crate::fields::RetainedTime {
                    requested_s: *t,
                    observed_s: *t,
                    step: 0,
                    multiblock: "fixture".into(),
                    shards: vec![],
                })
                .collect(),
            files: vec![field],
            field_units: serde_json::json!({}),
            physical_validation: "unqualified".into(),
        };
        commit_artifact(
            &root,
            "snapshot.json",
            &serde_json::to_vec(&snapshot).unwrap(),
            "json",
            "archived fixture",
        )
        .unwrap();
        store
            .add_artifacts(&job.id, &fields::manifests(&root, &snapshot).unwrap())
            .unwrap();
        store
            .transition(&job.id, "queued", "starting", None)
            .unwrap();
        store.finish(&job.id, 0, None).unwrap();
        job.id
    }

    #[test]
    fn committed_source_requires_original_authorization_and_unchanged_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("state")).unwrap();
        let id = archived_source(&store);
        let (root, _, _) = source(&store, &id).unwrap();
        store
            .connection
            .execute(
                "UPDATE job_authorizations SET digest=?1 WHERE job=?2",
                rusqlite::params!["f".repeat(64), id],
            )
            .unwrap();
        assert!(source(&store, &id).is_err());
        fs::write(root.join("snapshot.json"), b"{}").unwrap();
        assert!(fields::registered(&store, &id).is_err());
    }

    #[test]
    fn interrupted_unpublished_copies_are_removed_only_with_intact_original_source() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("state")).unwrap();
        let id = archived_source(&store);
        let (original, _, bound) = source(&store, &id).unwrap();
        for intact in [true, false] {
            let orphan = uuid::Uuid::new_v4().to_string();
            let directory = store.root.join(format!("artifacts/{orphan}"));
            private_dir(&directory).unwrap();
            commit_artifact(
                &directory,
                "source-retention.json",
                &serde_json::to_vec(&bound).unwrap(),
                "json",
                "interrupted staging intent",
            )
            .unwrap();
            fs::write(directory.join("partial-copy"), b"unpublished duplicate").unwrap();
            if !intact {
                fs::write(original.join("opaque-fixture.bin"), b"changed").unwrap();
            }
            recover_orphan(&store, &orphan).unwrap();
            assert_eq!(directory.exists(), !intact);
        }
    }

    #[test]
    fn orphan_recovery_preserves_copied_frames_without_an_intact_original_rendering() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("state")).unwrap();
        let id = archived_source(&store);
        let (_, _, bound) = source(&store, &id).unwrap();
        let orphan = uuid::Uuid::new_v4().to_string();
        let directory = store.root.join(format!("artifacts/{orphan}"));
        private_dir(&directory).unwrap();
        commit_artifact(
            &directory,
            "source-retention.json",
            &serde_json::to_vec(&bound).unwrap(),
            "json",
            "orphan fixture",
        )
        .unwrap();
        let missing = FrameSource {
            job_id: uuid::Uuid::new_v4().to_string(),
            plan_digest: "a".repeat(64),
            execution_binding_digest: "b".repeat(64),
            authorization_digest: "c".repeat(64),
            sequence_sha256: "d".repeat(64),
            bytes: 1234,
        };
        commit_artifact(
            &directory,
            "frame-retention.json",
            &serde_json::to_vec(&missing).unwrap(),
            "json",
            "orphan fixture",
        )
        .unwrap();
        fs::write(
            directory.join("only-retained-frame"),
            b"sole retained frame bytes",
        )
        .unwrap();
        recover_orphan(&store, &orphan).unwrap();
        assert!(directory.join("only-retained-frame").exists());
    }
}
