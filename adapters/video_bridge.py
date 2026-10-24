"""Fixed hardware encoding plus real decode, without any codec fallback."""

import hashlib
import json
import math
import os
import stat
import subprocess
import sys
from itertools import pairwise
from pathlib import Path

POLICY = "harbor-cad-media-drm-single-node-v1"


def drm_sandbox(pci, dev=Path("/dev")):
    """Worker DRM sandbox: only the selected render node, no KFD, no other card."""
    if os.environ.get("HARBOR_CAD_DRM_POLICY") != POLICY:
        raise RuntimeError("exact single-node DRM sandbox policy required")
    alias = dev / "dri/by-path" / f"pci-{pci}-render"
    node = alias.resolve() if alias.is_symlink() else None
    visible = sorted(p.name for p in (dev / "dri").iterdir())
    by_path = sorted(p.name for p in (dev / "dri/by-path").iterdir())
    if (
        node is None
        or not node.is_char_device()
        or visible != sorted(["by-path", node.name])
        or by_path != [alias.name]
        or (dev / "kfd").exists()
    ):
        raise RuntimeError("sandbox must expose only the selected DRM render node")
    return {
        "policy": POLICY,
        "render_node": str(node),
        "visible_dri": visible,
        "visible_by_path": by_path,
        "kfd_visible": False,
    }


def closed_record(path, limit):
    """Read bounded regular records without following frame/manifest symlinks."""
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode) or not 0 < metadata.st_size <= limit:
            raise ValueError("bounded closed regular frame record required")
        data = stream.read(limit + 1)
        if len(data) != metadata.st_size:
            raise ValueError("frame record changed or exceeded its bound")
        return data


def frame_sequence(work, plan):
    expected = plan["observation"]["retained_times_s"]
    if (
        not 0 < len(expected) <= 1024
        or any(
            isinstance(t, bool)
            or not isinstance(t, (int, float))
            or not math.isfinite(t)
            for t in expected
        )
        or any(a >= b for a, b in pairwise(expected))
    ):
        raise ValueError("bounded ordered approved physical-time sequence required")
    data = closed_record(work / "frame-sequence.json", 2 * 1024 * 1024)
    sequence = json.loads(data)
    records = sequence["frames"]
    names = [f"frame{i:04d}.png" for i in range(len(expected))]
    discovered = sorted(p.name for p in work.glob("frame*.png"))
    if (
        sequence["schema_version"] != 1
        or len(records) != len(expected)
        or discovered != names
        or [r["path"] for r in records] != names
        or [r["requested_s"] for r in records] != expected
    ):
        raise ValueError("frames must match every approved presentation time exactly")
    for record in records:
        observed = record["observed_s"]
        if (
            isinstance(observed, bool)
            or not isinstance(observed, (int, float))
            or not math.isfinite(observed)
            or observed < 0
        ):
            raise ValueError("finite observed physical time required")
        payload = closed_record(work / record["path"], 256 * 1024 * 1024)
        if (
            len(payload) != record["bytes"]
            or hashlib.sha256(payload).hexdigest() != record["sha256"]
        ):
            raise ValueError("rendered frame checksum/size mismatch")
    sequence_digest = hashlib.sha256(data).hexdigest()
    if sequence["source"] == "rendered_fields":
        receipt = json.loads(
            closed_record(work / "render-receipt.json", 2 * 1024 * 1024)
        )
        if (
            receipt["adapter"] != "ParaView"
            or receipt["executed"] is not True
            or receipt["software_fallback"] is not False
            or receipt["frame_sequence_sha256"] != sequence_digest
            or receipt["physical_times_s"] != expected
            or receipt["frames"] != records
        ):
            raise ValueError("frame sequence differs from completed render receipt")
        binding = field_binding()
        original_render = os.environ.get("HARBOR_CAD_FRAME_SOURCE_EXECUTION_ID")
        if original_render:
            binding.pop("presentation_execution_id", None)
            if "presentation_execution_id" in sequence:
                binding["presentation_execution_id"] = original_render
            elif sequence.get("execution_id") != original_render:
                raise ValueError("original B1 render execution identity mismatch")
        if any(sequence.get(k) != v or receipt.get(k) != v for k, v in binding.items()):
            raise ValueError(
                "frame sequence differs from exact retained scientific snapshot"
            )
    elif (
        sequence["source"] != "synthetic_fixture"
        or plan["case"]["geometry"]["synthetic"] is not True
        or any(s["operation"] == "render" for s in plan["stages"])
        or any(r["requested_s"] != r["observed_s"] for r in records)
    ):
        raise ValueError(
            "explicit synthetic encoder fixture or completed render required"
        )
    return records, sequence_digest, sequence["source"]


def field_binding():
    path = os.environ.get("HARBOR_CAD_FIELD_SNAPSHOT")
    if not path:
        return {}
    data = closed_record(Path(path), 2 * 1024 * 1024)
    snapshot = json.loads(data)
    if snapshot["schema_version"] != 1:
        raise ValueError("unsupported retained-field snapshot")
    binding = {
        "field_snapshot_sha256": hashlib.sha256(data).hexdigest(),
        "field_artifact_id": snapshot["artifact_id"],
        "science_id": snapshot["science_id"],
        "execution_id": snapshot["execution_id"],
    }
    execution = os.environ.get("HARBOR_CAD_PRESENTATION_EXECUTION_ID")
    if execution:
        binding["presentation_execution_id"] = execution
    return binding


def main():
    if sys.argv[1] != "video":
        raise ValueError("video operation required")
    plan = json.loads(Path(sys.argv[2]).read_text())
    selection = next(
        s["selection"] for s in plan["stages"] if s["operation"] == "video"
    )
    if selection["backend"] != "vaapi":
        raise ValueError("only explicitly selected VAAPI adapter implemented")
    frame_root = Path(os.environ.get("HARBOR_CAD_FRAMES_DIRECTORY", "/work"))
    frames, sequence_digest, source = frame_sequence(frame_root, plan)
    original_render = os.environ.get("HARBOR_CAD_FRAME_SOURCE_EXECUTION_ID")
    node = f"/dev/dri/by-path/pci-{selection['pci']}-render"
    sandbox = drm_sandbox(selection["pci"])
    command = [
        "@ffmpeg@",
        "-nostdin",
        "-v",
        "verbose",
        "-init_hw_device",
        f"vaapi=media:{node}",
        "-filter_hw_device",
        "media",
        "-framerate",
        "24",
        "-i",
        str(frame_root / "frame%04d.png"),
        "-vf",
        "format=nv12,hwupload",
        "-c:v",
        "h264_vaapi",
        "-frames:v",
        str(len(frames)),
        "-f",
        "mp4",
        "/work/video.partial",
    ]
    # Logs flow to the worker's bounded native log, never a full captured array.
    subprocess.run(command, check=True)
    subprocess.run(
        [
            "@ffmpeg@",
            "-nostdin",
            "-v",
            "error",
            "-i",
            "/work/video.partial",
            "-f",
            "null",
            "-",
        ],
        check=True,
    )
    metadata = json.loads(
        subprocess.check_output(
            [
                "@ffprobe@",
                "-v",
                "error",
                "-count_frames",
                "-show_frames",
                "-show_entries",
                "stream=codec_name,pix_fmt,nb_read_frames,r_frame_rate,width,height:frame=best_effort_timestamp_time",
                "-of",
                "json",
                "/work/video.partial",
            ]
        )
    )
    stream = metadata["streams"][0]
    if int(stream["nb_read_frames"]) != len(frames):
        raise RuntimeError("encoded/decoded frame count mismatch")
    presentation = plan["case"]["presentation"]
    if (
        stream["width"] != presentation["width"]
        or stream["height"] != presentation["height"]
    ):
        raise RuntimeError("encoded image dimensions differ from approved presentation")
    timestamps = [float(f["best_effort_timestamp_time"]) for f in metadata["frames"]]
    if len(timestamps) != len(frames) or any(
        not math.isfinite(t) or abs(t - i / 24) > 1e-5 for i, t in enumerate(timestamps)
    ):
        raise RuntimeError(
            "decoded presentation timestamps differ from fixed 24 fps sequence"
        )
    Path("/work/video.partial").replace("/work/video.mp4")
    receipt = {
        **field_binding(),
        **({"source_render_execution_id": original_render} if original_render else {}),
        "adapter": "FFmpeg",
        "backend": "vaapi",
        "encoder": "h264_vaapi",
        "pci": selection["pci"],
        "render_node": node,
        "sandbox": sandbox,
        "executed": True,
        "software_fallback": False,
        "decode": "CPU verification",
        "frames": len(frames),
        "metadata": stream,
        "scientific_arrays": "native fields retained independently of lossy video",
        "physical_times_s": [r["requested_s"] for r in frames],
        "observed_physical_times_s": [r["observed_s"] for r in frames],
        "frame_sequence_sha256": sequence_digest,
        "frame_source": source,
        "presentation_fps": 24,
        "presentation_timestamps_s": timestamps,
        "physical_time_labels": (
            "rendered into frames; presentation clock is independent"
            if source == "rendered_fields"
            else "unqualified: synthetic encoder fixture has no scientific time labels"
        ),
    }
    Path("/work/video-receipt.json.partial").write_text(json.dumps(receipt, indent=2))
    Path("/work/video-receipt.json.partial").replace("/work/video-receipt.json")


if __name__ == "__main__":
    main()
