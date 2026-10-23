"""Numerical filtering is deliberately separate from this EGL render stage."""

import ctypes as c
import hashlib
import json
import os
import sys
from pathlib import Path

POLICY = "harbor-cad-render-drm-single-node-v1"


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


def egl_library():
    egl = c.CDLL("@libegl@")
    egl.eglGetProcAddress.argtypes = [c.c_char_p]
    egl.eglGetProcAddress.restype = c.c_void_p
    return egl


def egl_proc(egl, name, result, *arguments):
    address = egl.eglGetProcAddress(name)
    if not address:
        raise RuntimeError(f"required EGL entry point unavailable: {name.decode()}")
    return c.CFUNCTYPE(result, *arguments)(address)


def egl_device(render_node):
    egl = egl_library()
    query = egl_proc(
        egl,
        b"eglQueryDevicesEXT",
        c.c_uint,
        c.c_int,
        c.POINTER(c.c_void_p),
        c.POINTER(c.c_int),
    )
    name = egl_proc(egl, b"eglQueryDeviceStringEXT", c.c_char_p, c.c_void_p, c.c_int)
    count = c.c_int()
    if not query(0, None, c.byref(count)) or not 0 < count.value <= 32:
        raise RuntimeError("bounded EGL device enumeration failed")
    capacity = count.value
    devices = (c.c_void_p * capacity)()
    if not query(capacity, devices, c.byref(count)) or count.value != capacity:
        raise RuntimeError("EGL device inventory changed during enumeration")
    matches = []
    for i in range(count.value):
        node = name(devices[i], 0x3377)  # EGL_DRM_RENDER_NODE_FILE_EXT
        if node and os.path.realpath(node.decode()) == os.path.realpath(render_node):
            matches.append(i)
    if len(matches) != 1:
        raise RuntimeError("ambiguous/stale EGL device-to-DRM identity")
    return matches[0]


def egl_context_device(render_node):
    """Verify the initialized display, rather than VTK's requested device index."""
    egl = egl_library()
    egl.eglGetCurrentDisplay.argtypes = []
    egl.eglGetCurrentDisplay.restype = c.c_void_p
    display = egl.eglGetCurrentDisplay()
    if not display:
        raise RuntimeError("no current EGL display after rendering")
    query = egl_proc(
        egl,
        b"eglQueryDisplayAttribEXT",
        c.c_uint,
        c.c_void_p,
        c.c_int,
        c.POINTER(c.c_ssize_t),
    )
    device = c.c_ssize_t()
    # EGL_EXT_device_query defines EGLAttrib as intptr_t, not EGLint.
    if not query(display, 0x322C, c.byref(device)) or not device.value:
        raise RuntimeError("initialized EGL display device could not be queried")
    name = egl_proc(egl, b"eglQueryDeviceStringEXT", c.c_char_p, c.c_void_p, c.c_int)
    node = name(device.value, 0x3377)
    if not node or os.path.realpath(node.decode()) != os.path.realpath(render_node):
        raise RuntimeError("actual EGL display device differs from selected DRM node")
    return os.path.realpath(node.decode())


def velocity_collection(work):
    # The pinned OpenLB driver also emits geometry.pvd on a separate index clock.
    fields = work / "tmp/vtkData/channel.pvd"
    if fields.is_symlink() or not fields.is_file():
        raise ValueError("authoritative OpenLB channel time collection required")
    return fields


def main():
    if sys.argv[1] != "render":
        raise ValueError("render operation required")
    plan = json.loads(Path(sys.argv[2]).read_text())
    field_binding = {}
    snapshot_path = os.environ.get("HARBOR_CAD_FIELD_SNAPSHOT")
    if snapshot_path:
        snapshot_bytes = Path(snapshot_path).read_bytes()
        if not 0 < len(snapshot_bytes) <= 2 * 1024 * 1024:
            raise ValueError("bounded retained-field snapshot required")
        snapshot = json.loads(snapshot_bytes)
        field_binding = {
            "field_snapshot_sha256": hashlib.sha256(snapshot_bytes).hexdigest(),
            "field_artifact_id": snapshot["artifact_id"],
            "science_id": snapshot["science_id"],
            "execution_id": snapshot["execution_id"],
        }
        execution = os.environ.get("HARBOR_CAD_PRESENTATION_EXECUTION_ID")
        if execution:
            field_binding["presentation_execution_id"] = execution
    if plan["case"]["presentation"]["field"] != "velocity":
        raise ValueError("only explicitly selected velocity rendering implemented")
    stage = next(s for s in plan["stages"] if s["operation"] == "render")
    pci = stage["selection"]["pci"]
    node = f"/dev/dri/by-path/pci-{pci}-render"
    sandbox = drm_sandbox(pci)
    # With one visible node the EGL index can map only to it; the current
    # display is still checked against the node after every render.
    index = egl_device(node)
    os.environ["VTK_EGL_DEVICE_INDEX"] = str(index)
    os.environ["VTK_DEFAULT_OPENGL_WINDOW"] = "vtkEGLRenderWindow"
    from paraview import simple as pv

    # The pinned simple.Render resets the camera on its first call by default.
    pv._DisableFirstRenderCameraReset()
    fields = velocity_collection(Path("/work"))
    source = pv.OpenDataFile(str(fields))
    source.UpdatePipelineInformation()
    view = pv.CreateView("RenderView")
    view.ViewSize = [
        plan["case"]["presentation"]["width"],
        plan["case"]["presentation"]["height"],
    ]
    view.CameraPosition = plan["case"]["presentation"]["camera"]
    display = pv.Show(source, view)
    display.Representation = "Surface"
    pv.ColorBy(display, ("POINTS", "physVelocity", "Magnitude"))
    lookup = pv.GetColorTransferFunction("physVelocity")
    lookup.AutomaticRescaleRangeMode = "Never"
    lookup.RescaleTransferFunction(*plan["case"]["presentation"]["range"])
    display.SetScalarBarVisibility(view, True)
    scalar_bar = pv.GetScalarBar(lookup, view)
    scalar_bar.Title = "Velocity (m/s)"
    scalar_bar.ComponentTitle = "Magnitude"
    label = pv.Text()
    pv.Show(label, view)
    retained = plan["observation"]["retained_times_s"]
    native_receipt = json.loads(Path("/work/openlb-receipt.json").read_text())
    time_mapping = {t["requested_s"]: t for t in native_receipt["retained_times"]}
    available = list(getattr(source, "TimestepValues", []))
    if any(
        t not in time_mapping or time_mapping[t]["step"] not in available
        for t in retained
    ):
        raise ValueError(
            "requested physical times were not retained; solving is not a render operation"
        )
    frames = []
    for i, time_s in enumerate(retained):
        label.Text = f"Synthetic channel | physical time {time_mapping[time_s]['observed_s']:.9g} s"
        view.ViewTime = time_mapping[time_s]["step"]
        source.UpdatePipeline(view.ViewTime)
        pv.Render(view)
        window = view.GetClientSideObject().GetRenderWindow()
        capabilities = window.ReportCapabilities()
        if (
            window.GetClassName() != "vtkEGLRenderWindow"
            or window.GetDeviceIndex() != index
        ):
            raise RuntimeError(
                "observed EGL context device differs from selected device"
            )
        if any(
            x in capabilities.lower()
            for x in ["llvmpipe", "softpipe", "swrast", "software rasterizer"]
        ):
            raise RuntimeError("software graphics fallback rejected")
        window.MakeCurrent()
        observed_node = egl_context_device(node)
        filename = f"frame{i:04d}.png"
        pv.SaveScreenshot(f"/work/{filename}", view)
        observed_camera = list(view.CameraPosition)
        observed_range = [lookup.RGBPoints[0], lookup.RGBPoints[-4]]
        if (
            observed_camera != plan["case"]["presentation"]["camera"]
            or observed_range != plan["case"]["presentation"]["range"]
            or display.Representation.GetData() != "Surface"
        ):
            raise RuntimeError("rendered camera, representation or fixed scale drift")
        payload = Path(f"/work/{filename}").read_bytes()
        frames.append(
            {
                "path": filename,
                "bytes": len(payload),
                "sha256": hashlib.sha256(payload).hexdigest(),
                "label": label.Text,
                **time_mapping[time_s],
            }
        )
    sequence = json.dumps(
        {
            "schema_version": 1,
            "source": "rendered_fields",
            "frames": frames,
            **field_binding,
        },
        indent=2,
    ).encode()
    Path("/work/frame-sequence.json.partial").write_bytes(sequence)
    Path("/work/frame-sequence.json.partial").replace("/work/frame-sequence.json")
    receipt = {
        **field_binding,
        "adapter": "ParaView",
        "backend": "egl",
        "pci": pci,
        "egl_device_index": index,
        "sandbox": sandbox,
        "observed_render_node": observed_node,
        "device_evidence": "EGL_DEVICE_EXT queried from current initialized EGL display",
        "context": capabilities,
        "executed": True,
        "software_fallback": False,
        "physical_times_s": retained,
        "numerical_filter": False,
        "time_mapping": list(time_mapping.values()),
        "fixed_range": plan["case"]["presentation"]["range"],
        "observed_camera": observed_camera,
        "camera_focal_point": list(view.CameraFocalPoint),
        "camera_view_up": list(view.CameraViewUp),
        "representation": display.Representation.GetData(),
        "color_field": "physVelocity",
        "color_component": "Magnitude",
        "frames": frames,
        "frame_sequence_sha256": hashlib.sha256(sequence).hexdigest(),
        "units": "m/s",
        "physical_time_labels": True,
        "physical_validation": "unqualified",
    }
    Path("/work/render-receipt.json.partial").write_text(json.dumps(receipt, indent=2))
    Path("/work/render-receipt.json.partial").replace("/work/render-receipt.json")


if __name__ == "__main__":
    main()
