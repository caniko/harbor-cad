import ctypes as c
import importlib.util
import os
from pathlib import Path
from types import SimpleNamespace

import pytest


def bridge(name="paraview_bridge.py"):
    path = Path(__file__).parents[2] / "adapters" / name
    spec = importlib.util.spec_from_file_location(name.removesuffix(".py"), path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_velocity_collection_does_not_confuse_material_output_with_scientific_time(
    tmp_path,
):
    module = bridge()
    fields = tmp_path / "tmp/vtkData"
    fields.mkdir(parents=True)
    (fields / "geometry.pvd").write_text("material-only time collection")
    with pytest.raises(ValueError):
        module.velocity_collection(tmp_path)
    channel = fields / "channel.pvd"
    channel.write_text("authoritative velocity/pressure time collection")
    assert module.velocity_collection(tmp_path) == channel
    channel.unlink()
    channel.symlink_to(fields / "geometry.pvd")
    with pytest.raises(ValueError):
        module.velocity_collection(tmp_path)


@pytest.mark.parametrize(
    "display,node,accepted",
    [
        (100, b"/dev/dri/renderD128", True),
        (100, b"/dev/dri/renderD129", False),
        (100, None, False),
        (None, b"/dev/dri/renderD128", False),
    ],
)
def test_actual_egl_display_must_resolve_to_selected_render_node(
    monkeypatch, display, node, accepted
):
    module = bridge()
    # Simulate only the external EGL boundary; the adapter's rejection policy
    # executes normally. Actual context creation is a separate native gate.
    egl = SimpleNamespace(eglGetCurrentDisplay=lambda: display)
    monkeypatch.setattr(module, "egl_library", lambda: egl)

    def query(_display, _attribute, out):
        c.cast(out, c.POINTER(c.c_ssize_t))[0] = 42
        return 1

    functions = {
        b"eglQueryDisplayAttribEXT": query,
        b"eglQueryDeviceStringEXT": lambda *_: node,
    }
    monkeypatch.setattr(module, "egl_proc", lambda _egl, name, *_: functions[name])
    if accepted:
        assert module.egl_context_device("/dev/dri/renderD128") == "/dev/dri/renderD128"
    else:
        with pytest.raises(RuntimeError):
            module.egl_context_device("/dev/dri/renderD128")


@pytest.mark.parametrize(
    "window_class,index,capabilities,accepted",
    [
        (
            "vtkEGLRenderWindow",
            3,
            "OpenGL renderer string: AMD Radeon RX 7900 XTX (radeonsi, navi31)",
            True,
        ),
        # Stock ParaView builds the X window too; selecting EGL is the contract.
        ("vtkXOpenGLRenderWindow", 3, "OpenGL renderer string: navi31", False),
        ("vtkEGLRenderWindow", 4, "OpenGL renderer string: navi31", False),
        ("vtkEGLRenderWindow", 3, "OpenGL renderer string: llvmpipe (LLVM 20)", False),
        ("vtkEGLRenderWindow", 3, "Renderer: softpipe", False),
        ("vtkEGLRenderWindow", 3, "Renderer: SWRAST", False),
        ("vtkEGLRenderWindow", 3, "Software Rasterizer", False),
    ],
)
def test_render_window_must_be_the_selected_egl_device_without_software(
    window_class, index, capabilities, accepted
):
    module = bridge()
    window = SimpleNamespace(
        GetClassName=lambda: window_class,
        GetDeviceIndex=lambda: index,
        ReportCapabilities=lambda: capabilities,
    )
    if accepted:
        assert module.require_egl_window(window, 3) == capabilities
    else:
        with pytest.raises(RuntimeError):
            module.require_egl_window(window, 3)


@pytest.mark.parametrize(
    "nodes,matched,index",
    [
        (["renderD128"], "renderD128", 0),
        (["renderD128", "renderD129"], "renderD129", 1),
        (["renderD128"], "renderD129", None),
        (["renderD129", "renderD129"], "renderD129", None),
        (["renderD128", None], "renderD128", 0),
    ],
)
def test_selected_render_node_maps_to_exactly_one_egl_device(
    monkeypatch, tmp_path, nodes, matched, index
):
    module = bridge()
    nodes = [tmp_path / node if node else None for node in nodes]

    def query(capacity, devices, count):
        if not capacity:
            count._obj.value = len(nodes)
            return 1
        assert capacity == len(nodes)
        for i, _node in enumerate(nodes):
            devices[i] = 100 + i
        return 1

    def node_name(device, _attribute):
        node = nodes[device - 100]
        return os.fsencode(node) if node else None

    functions = {
        b"eglQueryDevicesEXT": query,
        b"eglQueryDeviceStringEXT": node_name,
    }
    monkeypatch.setattr(module, "egl_library", lambda: SimpleNamespace())
    monkeypatch.setattr(module, "egl_proc", lambda _egl, name, *_: functions[name])
    if index is None:
        with pytest.raises(RuntimeError):
            module.egl_device(str(tmp_path / matched))
    else:
        assert module.egl_device(str(tmp_path / matched)) == index


@pytest.mark.parametrize("count", [0, 33])
def test_egl_enumeration_is_bounded(monkeypatch, count):
    module = bridge()

    def query(_capacity, _devices, out):
        out._obj.value = count
        return 1

    functions = {
        b"eglQueryDevicesEXT": query,
        b"eglQueryDeviceStringEXT": lambda *_: b"/dev/dri/renderD128",
    }
    monkeypatch.setattr(module, "egl_library", lambda: SimpleNamespace())
    monkeypatch.setattr(module, "egl_proc", lambda _egl, name, *_: functions[name])
    with pytest.raises(RuntimeError):
        module.egl_device("/dev/dri/renderD128")


@pytest.mark.parametrize(
    "adapter,policy",
    [
        ("paraview_bridge.py", "harbor-cad-render-drm-single-node-v1"),
        ("video_bridge.py", "harbor-cad-media-drm-single-node-v1"),
    ],
)
@pytest.mark.parametrize(
    "damage",
    [None, "policy", "other_node", "other_alias", "kfd", "regular_node", "no_alias"],
)
def test_render_and_media_see_only_their_selected_drm_node(
    monkeypatch, tmp_path, adapter, policy, damage
):
    module = bridge(adapter)
    pci = "0000:03:00.0"
    dev = tmp_path / "dev"
    (dev / "dri/by-path").mkdir(parents=True)
    # /dev/null stands in for the selected character device; the listing is
    # what the bwrap mount namespace would expose.
    (dev / "dri/null").touch()
    alias = dev / "dri/by-path" / f"pci-{pci}-render"
    alias.symlink_to("/dev/null")
    monkeypatch.setenv("HARBOR_CAD_DRM_POLICY", policy)
    if damage == "policy":
        monkeypatch.setenv("HARBOR_CAD_DRM_POLICY", "harbor-cad-native-v2")
    elif damage == "other_node":
        (dev / "dri/renderD129").touch()
    elif damage == "other_alias":
        (dev / "dri/by-path/pci-0000:7a:00.0-render").symlink_to("/dev/null")
    elif damage == "kfd":
        (dev / "kfd").touch()
    elif damage == "regular_node":
        alias.unlink()
        alias.symlink_to(dev / "dri/null")
    elif damage == "no_alias":
        alias.unlink()
    if damage is None:
        assert module.drm_sandbox(pci, dev) == {
            "policy": policy,
            "render_node": "/dev/null",
            "visible_dri": ["by-path", "null"],
            "visible_by_path": [alias.name],
            "kfd_visible": False,
        }
    else:
        with pytest.raises(RuntimeError):
            module.drm_sandbox(pci, dev)
