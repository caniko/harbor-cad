//! Registered, closed rendered frames for independent hardware encoding.
use crate::{Result, contracts::*, fields, storage::*};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn checked_record(root: &Path, record: &ArtifactManifest, limit: u64) -> Result<Vec<u8>> {
    let observed = native_manifest(root, &record.path, limit, "verify frame source")?;
    if observed.sha256 != record.sha256 || observed.bytes != record.bytes {
        return Err(invalid("registered frame record changed"));
    }
    let data = crate::worker::read_bounded(&safe_path(root, &record.path)?, limit)?;
    if data.len() as u64 != record.bytes
        || format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&data)) != record.sha256
    {
        return Err(invalid("frame record changed during inspection"));
    }
    Ok(data)
}

pub(crate) fn sequence(
    root: &Path,
    records: &[ArtifactManifest],
    render: &ExecutionPlan,
    source: &RetainedSource,
    expected_hash: Option<&str>,
) -> Result<Vec<ArtifactManifest>> {
    let records: BTreeMap<_, _> = records.iter().map(|r| (r.path.as_str(), r)).collect();
    let manifest = records
        .get("frame-sequence.json")
        .ok_or_else(|| invalid("registered rendered frame sequence required"))?;
    let data = checked_record(root, manifest, 2 * 1024 * 1024)?;
    if expected_hash.is_some_and(|h| manifest.sha256 != h) {
        return Err(invalid("approved frame sequence changed"));
    }
    let value: serde_json::Value = serde_json::from_slice(&data)?;
    let rows = value["frames"]
        .as_array()
        .ok_or_else(|| invalid("frame list"))?;
    let rendered_id = if render.schema_version == 1 {
        &value["execution_id"]
    } else {
        &value["presentation_execution_id"]
    };
    if value["schema_version"] != 1
        || value["source"] != "rendered_fields"
        || value["field_snapshot_sha256"] != source.snapshot_sha256
        || value["field_artifact_id"] != source.artifact_id
        || value["science_id"] != source.science_id
        || value["execution_id"] != source.plan_digest
        || rendered_id != &render.id()?
        || rows.is_empty()
        || rows.len() > 1024
        || rows.len() != render.observation.retained_times_s.len()
    {
        return Err(invalid(
            "rendered sequence differs from exact science, rendering execution or retained times",
        ));
    }
    let mut selected = vec![(*manifest).clone()];
    for (index, row) in rows.iter().enumerate() {
        let name = format!("frame{index:04}.png");
        let record = records
            .get(name.as_str())
            .ok_or_else(|| invalid("registered frame missing"))?;
        if row["path"] != name
            || row["sha256"] != record.sha256
            || row["bytes"] != record.bytes
            || row["requested_s"].as_f64() != Some(render.observation.retained_times_s[index])
            || row["observed_s"]
                .as_f64()
                .is_none_or(|t| !t.is_finite() || t < 0.)
        {
            return Err(invalid("frame identity or physical-time mapping mismatch"));
        }
        // Only the fixed PNG product from the renderer is accepted. Bound the
        // decoded dimensions before the encoder can allocate frame buffers.
        let pixels = u64::from(render.channel_case()?.presentation.width)
            * u64::from(render.channel_case()?.presentation.height);
        let bytes = checked_record(root, record, pixels * 8 + 1024 * 1024)?;
        if bytes.len() < 33
            || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
            || &bytes[12..16] != b"IHDR"
            || bytes[16..20] != render.channel_case()?.presentation.width.to_be_bytes()
            || bytes[20..24] != render.channel_case()?.presentation.height.to_be_bytes()
        {
            return Err(invalid(
                "bounded PNG dimensions differ from approved rendering",
            ));
        }
        selected.push((*record).clone());
    }
    let receipt = records
        .get("render-receipt.json")
        .ok_or_else(|| invalid("registered completed render receipt required"))?;
    let proof: serde_json::Value =
        serde_json::from_slice(&checked_record(root, receipt, 2 * 1024 * 1024)?)?;
    if proof["adapter"] != "ParaView"
        || proof["executed"] != true
        || proof["software_fallback"] != false
        || proof["frame_sequence_sha256"] != manifest.sha256
        || proof["frames"] != value["frames"]
        || proof["physical_times_s"] != serde_json::to_value(&render.observation.retained_times_s)?
        || proof["observed_camera"]
            != serde_json::to_value(render.channel_case()?.presentation.camera)?
        || [
            "field_snapshot_sha256",
            "field_artifact_id",
            "science_id",
            "execution_id",
            "presentation_execution_id",
        ]
        .iter()
        .any(|k| proof[k] != value[k])
    {
        return Err(invalid(
            "frame sequence lacks matching executed rendering receipt",
        ));
    }
    selected.push((*receipt).clone());
    Ok(selected)
}

fn verify_times(
    root: &Path,
    selected: &[ArtifactManifest],
    snapshot: &fields::FieldSnapshot,
) -> Result<()> {
    let manifest = selected
        .first()
        .ok_or_else(|| invalid("frame sequence missing"))?;
    let sequence: serde_json::Value =
        serde_json::from_slice(&checked_record(root, manifest, 2 * 1024 * 1024)?)?;
    for row in sequence["frames"]
        .as_array()
        .ok_or_else(|| invalid("frame rows"))?
    {
        if !snapshot.times.iter().any(|t| {
            row["requested_s"].as_f64() == Some(t.requested_s)
                && row["observed_s"].as_f64() == Some(t.observed_s)
                && row["step"].as_u64() == Some(t.step)
        }) {
            return Err(invalid(
                "rendered frame time differs from retained scientific snapshot",
            ));
        }
    }
    Ok(())
}

pub(crate) fn source(
    store: &Store,
    id: &str,
) -> Result<(
    ExecutionPlan,
    RetainedSource,
    FrameSource,
    Vec<ArtifactManifest>,
)> {
    if store.job(id)?.state != "succeeded" {
        return Err(invalid(
            "frame source must be a completed committed rendering job",
        ));
    }
    let render = store.plan(id)?;
    if render.frames.is_some()
        || !render
            .stages
            .iter()
            .any(|s| matches!(s.operation, StageOperation::Render))
    {
        return Err(invalid("frame source requires an actual rendering stage"));
    }
    let binding = store.execution_binding(id)?;
    let authorization = store
        .execution_authorization(id)?
        .ok_or_else(|| invalid("frame source authorization required"))?;
    authorization.verify(&render, &store.job_profile(id)?, &binding)?;
    let fields_job = render.source.as_ref().map_or(id, |s| s.job_id.as_str());
    let (_, _, fields) = crate::presentation::source(store, fields_job)?;
    if let Some(selected) = &render.source
        && digest(selected)? != digest(&fields)?
    {
        return Err(invalid("original rendering source identity changed"));
    }
    let (_, snapshot, copied_hash) = fields::registered(store, id)?;
    if copied_hash != fields.snapshot_sha256 {
        return Err(invalid("rendered field snapshot changed"));
    }
    let records = sequence(
        &store.job_dir(id)?,
        &store.artifacts(id)?,
        &render,
        &fields,
        None,
    )?;
    verify_times(&store.job_dir(id)?, &records, &snapshot)?;
    let bytes = records.iter().try_fold(0u64, |total, r| {
        total
            .checked_add(r.bytes)
            .ok_or_else(|| invalid("frame source byte count overflow"))
    })?;
    let source = FrameSource {
        job_id: id.into(),
        plan_digest: render.id()?,
        execution_binding_digest: digest(&binding)?,
        authorization_digest: digest(&authorization)?,
        sequence_sha256: records[0].sha256.clone(),
        bytes,
    };
    Ok((render, fields, source, records))
}

pub fn plan(store: &Store, request: VideoRequest, policy: String) -> Result<ExecutionPlan> {
    let (render, fields, frames, _) = source(store, &request.source_job)?;
    ExecutionPlan::video(&render, fields, frames, request, policy)
}

pub(crate) fn retain(store: &Store, id: &str, plan: &ExecutionPlan) -> Result<()> {
    let Some(expected) = &plan.frames else {
        return Ok(());
    };
    let (render, fields, observed, records) = source(store, &expected.job_id)?;
    if digest(&observed)? != digest(expected)?
        || digest(
            plan.source
                .as_ref()
                .ok_or_else(|| invalid("video source"))?,
        )? != digest(&fields)?
        || render.observation.retained_times_s != plan.observation.retained_times_s
        || digest(&render.case)? != digest(&plan.case)?
    {
        return Err(invalid(
            "approved independent video differs from registered source rendering",
        ));
    }
    let root = store.job_dir(id)?;
    let destination = root.join("retained-frames");
    let staging = root.join(format!(".retained-frames-{}", uuid::Uuid::new_v4()));
    let intent = commit_artifact(
        &root,
        "frame-retention.json",
        &serde_json::to_vec(expected)?,
        "json",
        "immutable rendered-input staging intent",
    )?;
    private_dir(&staging)?;
    // A rollback can leave the sole copy if the source disappears mid-submit.
    // The persisted intent lets orphan recovery prove removability later.
    (|| {
        for record in &records {
            copy_verified(&store.job_dir(&expected.job_id)?, &staging, record)?;
        }
        sequence(
            &staging,
            &records,
            &render,
            &fields,
            Some(&expected.sequence_sha256),
        )?;
        sync_directories(&staging)?;
        publish_directory(&staging, &destination)?;
        fs::File::open(&root)?.sync_all()?;
        for mut record in records {
            record.path = format!("retained-frames/{}", record.path);
            store.add_artifact(id, &record)?;
        }
        let provenance = serde_json::json!({"job_id":expected.job_id,"plan":render,"host_profile":store.job_profile(&expected.job_id)?,
            "execution_binding":store.execution_binding(&expected.job_id)?,"execution_authorization":store.execution_authorization(&expected.job_id)?});
        store.add_artifact(
            id,
            &commit_artifact(
                &root,
                "source-rendering.json",
                &serde_json::to_vec_pretty(&provenance)?,
                "json",
                "original rendering approval and execution authorization; no upgrade",
            )?,
        )?;
        store.add_artifact(id, &intent)?;
        Ok(())
    })()
}

pub(crate) fn registered(
    store: &Store,
    id: &str,
    plan: &ExecutionPlan,
) -> Result<(PathBuf, Vec<ArtifactManifest>)> {
    let expected = plan
        .frames
        .as_ref()
        .ok_or_else(|| invalid("independent video frame source missing"))?;
    let root = store.job_dir(id)?.join("retained-frames");
    let records: Vec<_> = store
        .artifacts(id)?
        .into_iter()
        .filter_map(|mut r| {
            r.path = r.path.strip_prefix("retained-frames/")?.into();
            Some(r)
        })
        .collect();
    // The copied provenance is a registered record; bind its original execution
    // to the approved descriptor, then inspect the closed frame graph again.
    let all = store.artifacts(id)?;
    let record = all
        .iter()
        .find(|r| r.path == "source-rendering.json")
        .ok_or_else(|| invalid("rendering provenance missing"))?;
    let provenance: serde_json::Value =
        serde_json::from_slice(&checked_record(&store.job_dir(id)?, record, MAX_MESSAGE)?)?;
    let render: ExecutionPlan = serde_json::from_value(provenance["plan"].clone())?;
    let authorization: crate::authority::ExecutionAuthorization =
        serde_json::from_value(provenance["execution_authorization"].clone())?;
    let binding: crate::execution::ExecutionBinding =
        serde_json::from_value(provenance["execution_binding"].clone())?;
    let profile: HostExecutionProfile = serde_json::from_value(provenance["host_profile"].clone())?;
    authorization.verify(&render, &profile, &binding)?;
    if render.id()? != expected.plan_digest
        || digest(&binding)? != expected.execution_binding_digest
        || digest(&authorization)? != expected.authorization_digest
        || provenance["job_id"] != expected.job_id
        || digest(&render.case)? != digest(&plan.case)?
        || render.observation.retained_times_s != plan.observation.retained_times_s
    {
        return Err(invalid(
            "copied frame-source provenance differs from approved rendering",
        ));
    }
    let selected = sequence(
        &root,
        &records,
        &render,
        plan.source
            .as_ref()
            .ok_or_else(|| invalid("source science"))?,
        Some(&expected.sequence_sha256),
    )?;
    let (_, snapshot, hash) = fields::registered(store, id)?;
    crate::presentation::verify(plan, &snapshot, &hash)?;
    verify_times(&root, &selected, &snapshot)?;
    let bytes: u64 = selected.iter().map(|r| r.bytes).sum();
    if bytes != expected.bytes {
        return Err(invalid("copied frame-source byte count mismatch"));
    }
    Ok((root, selected))
}

pub(crate) fn orphan_source_intact(store: &Store, root: &Path) -> Result<bool> {
    let intent = safe_path(root, "frame-retention.json")?;
    if !intent.exists() {
        return Ok(true);
    }
    let expected: FrameSource =
        serde_json::from_slice(&crate::worker::read_bounded(&intent, MAX_MESSAGE)?)?;
    Ok(source(store, &expected.job_id)
        .is_ok_and(|(_, _, observed, _)| digest(&observed).ok() == digest(&expected).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path) -> (ExecutionPlan, RetainedSource, Vec<ArtifactManifest>) {
        let original = ExecutionPlan::reference(CaseSpec::reference()).unwrap();
        let fields = RetainedSource {
            job_id: uuid::Uuid::new_v4().to_string(),
            plan_digest: original.id().unwrap(),
            execution_binding_digest: "a".repeat(64),
            authorization_digest: "b".repeat(64),
            snapshot_sha256: "c".repeat(64),
            artifact_id: "d".repeat(64),
            science_id: original.science_id().unwrap(),
            bytes: 4096,
        };
        let render = ExecutionPlan::presentation(
            &original,
            fields.clone(),
            PresentationRequest {
                source_job: fields.job_id.clone(),
                times_s: vec![0.],
                presentation: original.channel_case().unwrap().presentation.clone(),
                render: GpuSelection {
                    role: Role::Render,
                    backend: "egl".into(),
                    pci: "0000:03:00.0".into(),
                    backend_uuid: None,
                },
                media: None,
            },
            "research".into(),
        )
        .unwrap();
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(
            render
                .channel_case()
                .unwrap()
                .presentation
                .width
                .to_be_bytes(),
        );
        png.extend(
            render
                .channel_case()
                .unwrap()
                .presentation
                .height
                .to_be_bytes(),
        );
        png.resize(33, 0); // Header-only fixture; never passed to a native encoder.
        let frame = commit_artifact(
            root,
            "frame0000.png",
            &png,
            "png",
            "PNG-header contract fixture",
        )
        .unwrap();
        let rows = serde_json::json!([{"path":frame.path,"bytes":frame.bytes,"sha256":frame.sha256,"requested_s":0.,"observed_s":0.,"step":0}]);
        let binding = serde_json::json!({"schema_version":1,"source":"rendered_fields","frames":rows,
            "field_snapshot_sha256":fields.snapshot_sha256,"field_artifact_id":fields.artifact_id,
            "science_id":fields.science_id,"execution_id":fields.plan_digest,"presentation_execution_id":render.id().unwrap()});
        let manifest = commit_artifact(
            root,
            "frame-sequence.json",
            &serde_json::to_vec(&binding).unwrap(),
            "json",
            "contract fixture",
        )
        .unwrap();
        let mut receipt = binding;
        receipt["adapter"] = "ParaView".into();
        receipt["executed"] = true.into();
        receipt["software_fallback"] = false.into();
        receipt["frame_sequence_sha256"] = manifest.sha256.clone().into();
        receipt["physical_times_s"] = serde_json::json!([0.]);
        receipt["observed_camera"] =
            serde_json::to_value(render.channel_case().unwrap().presentation.camera).unwrap();
        let receipt = commit_artifact(
            root,
            "render-receipt.json",
            &serde_json::to_vec(&receipt).unwrap(),
            "json",
            "contract fixture",
        )
        .unwrap();
        (render, fields, vec![frame, manifest, receipt])
    }

    #[test]
    fn registered_frames_reject_changed_bytes_source_execution_times_and_dimensions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let (render, fields, records) = fixture(root);
        let selected = sequence(root, &records, &render, &fields, None).unwrap();
        assert_eq!(selected.len(), 3);
        assert!(sequence(root, &records, &render, &fields, Some(&"f".repeat(64))).is_err());
        let mut wrong_science = fields.clone();
        wrong_science.science_id = "f".repeat(64);
        assert!(sequence(root, &records, &render, &wrong_science, None).is_err());
        let mut wrong_time = render.clone();
        wrong_time.observation.retained_times_s = vec![5.];
        assert!(sequence(root, &records, &wrong_time, &fields, None).is_err());
        let mut wrong_camera = render.clone();
        wrong_camera.case.as_mut().unwrap().presentation.camera[0] += 1.;
        assert!(sequence(root, &records, &wrong_camera, &fields, None).is_err());
        let mut bytes = fs::read(root.join("frame0000.png")).unwrap();
        bytes[16..20].copy_from_slice(&16384u32.to_be_bytes());
        fs::write(root.join("frame0000.png"), bytes).unwrap();
        assert!(sequence(root, &records, &render, &fields, None).is_err());
    }

    #[test]
    fn registered_frames_reject_links_and_undeclared_science() {
        let temp = tempfile::tempdir().unwrap();
        let (render, fields, records) = fixture(temp.path());
        fs::rename(
            temp.path().join("frame0000.png"),
            temp.path().join("alias.png"),
        )
        .unwrap();
        std::os::unix::fs::symlink("alias.png", temp.path().join("frame0000.png")).unwrap();
        assert!(sequence(temp.path(), &records, &render, &fields, None).is_err());
        assert!(sequence(temp.path(), &records[..1], &render, &fields, None).is_err());
    }

    #[test]
    fn self_consistent_frame_metadata_cannot_authorize_oversized_decoded_images() {
        let temp = tempfile::tempdir().unwrap();
        let (render, fields, mut records) = fixture(temp.path());
        let frame = temp.path().join("frame0000.png");
        let mut png = fs::read(&frame).unwrap();
        png[16..20].copy_from_slice(&16384u32.to_be_bytes());
        fs::write(&frame, png).unwrap();
        records[0] = native_manifest(
            temp.path(),
            "frame0000.png",
            1024,
            "forged self-consistent metadata",
        )
        .unwrap();
        let path = temp.path().join("frame-sequence.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["frames"][0]["sha256"] = records[0].sha256.clone().into();
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        records[1] = native_manifest(
            temp.path(),
            "frame-sequence.json",
            65536,
            "forged self-consistent metadata",
        )
        .unwrap();
        let path = temp.path().join("render-receipt.json");
        let mut receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        receipt["frames"] = manifest["frames"].clone();
        receipt["frame_sequence_sha256"] = records[1].sha256.clone().into();
        fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        records[2] = native_manifest(
            temp.path(),
            "render-receipt.json",
            65536,
            "forged self-consistent metadata",
        )
        .unwrap();
        let error = sequence(temp.path(), &records, &render, &fields, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("PNG dimensions"), "{error}");
    }
}
