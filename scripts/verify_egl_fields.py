"""Opt-in retained CPU fields → selected EGL surfaces → selected VAAPI video.

Uses the production DRM binding, shared card reservation and native containment
via examples/native_stage_probe.rs. This adapter probe does not qualify GPU
OpenLB, worker B1, GPU numerical filters or physical engineering conclusions.
"""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import time
import uuid
from pathlib import Path

from verify_native_cpu import verify_manifest
from verify_vaapi import packaged

POLICIES = {
    "render": "harbor-cad-render-drm-single-node-v1",
    "video": "harbor-cad-media-drm-single-node-v1",
}


def single_node(receipt, policy, pci):
    """The adapter saw only the selected render node and no KFD."""
    node = Path(f"/dev/dri/by-path/pci-{pci}-render").resolve(strict=True)
    assert receipt["sandbox"] == {
        "policy": policy,
        "render_node": str(node),
        "visible_dri": ["by-path", node.name],
        "visible_by_path": [f"pci-{pci}-render"],
        "kfd_visible": False,
    }


def records(root):
    return {
        str(p.relative_to(root)): {
            "bytes": p.stat().st_size,
            "sha256": hashlib.sha256(p.read_bytes()).hexdigest(),
        }
        for p in sorted(root.rglob("*"))
        if p.is_file()
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--render", type=Path, required=True)
    parser.add_argument("--media", type=Path, required=True)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--pci", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    runtime, render, media = map(packaged, (args.runtime, args.render, args.media))
    bwrap = packaged(Path(json.loads(runtime.read_text())["bwrap"]))
    source = args.bundle.resolve(strict=True)
    verify_manifest(source)
    execution = json.loads((source / "execution.json").read_text())
    assert execution["job"]["state"] == "succeeded"
    root = args.output.resolve()
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    work = root / "work"
    work.mkdir(mode=0o700)
    shutil.copytree(source / "tmp", work / "tmp")
    shutil.copyfile(source / "openlb-receipt.json", work / "openlb-receipt.json")
    plan = json.loads((source / "native-plan.json").read_text())
    original_science = {k: v for k, v in plan["case"].items() if k != "presentation"}
    # An explicit presentation-only recipe for the labelled reference dimensions.
    assert plan["case"]["geometry"]["synthetic"]
    assert plan["case"]["applicability"]["formulation"] == "periodic_forced_channel"
    assert plan["case"]["length"] == {"value": 0.02, "unit": "m"}
    assert plan["case"]["channel_height"] == {"value": 0.01, "unit": "m"}
    plan["case"]["presentation"].update(camera=[0.04, 0.025, 0.03], range=[0.0, 0.0015])
    assert original_science == {
        k: v for k, v in plan["case"].items() if k != "presentation"
    }
    plan["stages"] = [
        {
            "operation": "render",
            "selection": {"role": "render", "backend": "egl", "pci": args.pci},
        },
        {
            "operation": "video",
            "selection": {"role": "media", "backend": "vaapi", "pci": args.pci},
        },
    ]
    plan_path = root / "plan.json"
    plan_path.write_text(json.dumps(plan, indent=2) + "\n")
    before = records(work)
    probe = root / "native-stage-probe-tested"
    expected = hashlib.sha256(args.probe.read_bytes()).hexdigest()
    shutil.copyfile(args.probe, probe)
    probe.chmod(0o500)
    assert hashlib.sha256(probe.read_bytes()).hexdigest() == expected
    environment = {
        k: os.environ[k]
        for k in ("HOME", "PATH", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS")
        if k in os.environ
    }
    services = []
    for stage, executable in (("render", render), ("video", media)):
        unit = f"harbor-cad-egl-fields-{uuid.uuid4()}.service"
        command = [
            "systemd-run",
            "--user",
            "--wait",
            "--pipe",
            "--collect",
            f"--unit={unit}",
            "--property=MemoryMax=2147483648",
            "--property=CPUQuota=100%",
            "--property=TasksMax=128",
            "--property=KillMode=control-group",
            "--property=NoNewPrivileges=yes",
            "--property=RuntimeMaxSec=200",
            str(probe),
            str(bwrap),
            str(executable),
            args.pci,
            str(work),
            str(plan_path),
            stage,
        ]
        log_path = root / f"{stage}-service.log"
        try:
            start = time.monotonic()
            with log_path.open("w") as log:
                result = subprocess.run(
                    command,
                    env=environment,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    timeout=215,
                    check=False,
                )
            if result.returncode:
                raise RuntimeError(
                    f"{stage} failed; inspect {log_path} and {work / (stage + '-native.log')}"
                )
            resource = next(
                json.loads(line)
                for line in log_path.read_text().splitlines()
                if line.startswith('{"binding":')
            )
            assert resource["memory_peak_bytes"] > 0 and resource["cpu_usage_usec"] > 0
            assert resource["binding"]["pci"] == args.pci
            assert resource["sandbox_policy"] == POLICIES[stage]
            assert resource["cgroup"].endswith("/" + unit)
            services.append(
                {
                    "stage": stage,
                    "unit": unit,
                    "elapsed_s": time.monotonic() - start,
                    "resource": resource,
                }
            )
        finally:
            subprocess.run(
                ["systemctl", "--user", "stop", unit],
                env=environment,
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=15,
            )
        after = records(work)
        assert all(after[path] == record for path, record in before.items())
        assert {k: v for k, v in after.items() if k.startswith("tmp/")} == {
            k: v for k, v in before.items() if k.startswith("tmp/")
        }
    egl = json.loads((work / "render-receipt.json").read_text())
    video = json.loads((work / "video-receipt.json").read_text())
    for receipt, backend, stage in ((egl, "egl", "render"), (video, "vaapi", "video")):
        assert receipt["executed"] and not receipt["software_fallback"]
        assert receipt["backend"] == backend and receipt["pci"] == args.pci
        single_node(receipt, POLICIES[stage], args.pci)
    assert egl["observed_render_node"] == str(
        Path(f"/dev/dri/by-path/pci-{args.pci}-render").resolve(strict=True)
    )
    assert egl["physical_times_s"] == [0, 10, 20] and egl["units"] == "m/s"
    assert egl["observed_camera"] == plan["case"]["presentation"]["camera"]
    assert egl["fixed_range"] == plan["case"]["presentation"]["range"]
    assert egl["representation"] == "Surface" and egl["color_component"] == "Magnitude"
    for i, frame in enumerate(egl["frames"]):
        assert frame["path"] == f"frame{i:04d}.png"
        assert frame["requested_s"] == i * 10
        assert frame["label"] == f"Synthetic channel | physical time {i * 10} s"
    assert len(egl["frames"]) == video["frames"] == 3
    sequence_path = work / "frame-sequence.json"
    sequence = json.loads(sequence_path.read_text())
    sequence_digest = hashlib.sha256(sequence_path.read_bytes()).hexdigest()
    assert sequence["source"] == video["frame_source"] == "rendered_fields"
    assert sequence["schema_version"] == 1 and sequence["frames"] == egl["frames"]
    assert (
        sequence_digest
        == egl["frame_sequence_sha256"]
        == video["frame_sequence_sha256"]
    )
    assert video["physical_times_s"] == [0, 10, 20]
    assert video["observed_physical_times_s"] == [
        r["observed_s"] for r in egl["frames"]
    ]
    for frame in sequence["frames"]:
        data = (work / frame["path"]).read_bytes()
        assert (
            len(data) == frame["bytes"]
            and hashlib.sha256(data).hexdigest() == frame["sha256"]
        )
    assert video["metadata"]["width"] == 640 and video["metadata"]["height"] == 480
    assert (
        video["metadata"]["codec_name"] == "h264"
        and video["metadata"]["pix_fmt"] == "yuv420p"
    )
    assert len(video["presentation_timestamps_s"]) == 3
    assert all(
        abs(t - i / 24) < 1e-5 for i, t in enumerate(video["presentation_timestamps_s"])
    )
    report = {
        "scope": "retained CPU OpenLB fields, selected packaged EGL surface rendering and VAAPI encoding/CPU decode; production DRM/process/reservation helpers; not GPU OpenLB or worker B1",
        "source_bundle": str(source),
        "runtime": str(runtime),
        "render": str(render),
        "media": str(media),
        "renderer_sha256": hashlib.sha256(render.read_bytes()).hexdigest(),
        "media_sha256": hashlib.sha256(media.read_bytes()).hexdigest(),
        "frozen_probe_sha256": expected,
        "services": services,
        "source_fields_unchanged": True,
        "egl": egl,
        "video": video,
        "records": records(work),
        "physical_validation": "unqualified",
        "vram_peak": "unqualified; per-process GPU accounting not established",
        "physical_time_label_pixel_inspection": "pending independent image inspection",
    }
    (root / "verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps(
            {
                "verification": str(root / "verification.json"),
                "services": services,
                "frames": 3,
                "source_fields_unchanged": True,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
