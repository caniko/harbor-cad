"""Mocked-ioctl checks for the KFD foreign-GPU exclusion gate.

The real gate cannot run while the iGPU is on vfio-pci, so a fake KFD that
encodes the running kernel's documented behaviour (foreign gpu_id -> EINVAL at
``kfd_process_device_data_by_id``; the selected card acquires its VM and
allocates) drives the pure probe/evaluation logic.
"""

import errno
import importlib.util
import struct
from pathlib import Path

import pytest


def gate():
    path = Path(__file__).parents[2] / "scripts/verify_kfd_exclusion.py"
    spec = importlib.util.spec_from_file_location("verify_kfd_exclusion", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _gpu_id_of(mod, request, buffer):
    if request == mod.ACQUIRE_VM:
        _, gpu_id = struct.unpack("=II", buffer)
        return gpu_id
    if request == mod.ALLOC_MEMORY_OF_GPU:
        return struct.unpack_from("=I", buffer, 32)[0]
    return struct.unpack_from("=I", buffer, mod.CREATE_QUEUE_GPU_ID_OFFSET)[0]


class FakeKfd:
    """A KFD whose per-id pdd exists only for the selected card's drm_fd."""

    def __init__(self, mod, selected_gpu_id, selected_drm_fd):
        self.mod = mod
        self.selected = selected_gpu_id
        self.drm = selected_drm_fd
        self.acquired = set()

    def ioctl(self, _fd, request, buffer, _mutate):
        if request == self.mod.GET_VERSION:
            struct.pack_into("=II", buffer, 0, 1, 23)
            return buffer
        gpu_id = _gpu_id_of(self.mod, request, buffer)
        if gpu_id != self.selected:
            # kfd_process_device_data_by_id returns NULL -> -EINVAL.
            raise OSError(errno.EINVAL, "no such gpu_id")
        if request == self.mod.ACQUIRE_VM:
            drm_fd = struct.unpack("=II", buffer)[0]
            if drm_fd != self.drm:
                raise OSError(errno.EINVAL, "fget failed / wrong drm fd")
            self.acquired.add(gpu_id)
            return buffer
        if gpu_id not in self.acquired:
            raise OSError(errno.EINVAL, "no acquired VM for this card")
        return buffer


SELECTED = 4672
FOREIGN = 4800
DRM_FD = 7
NO_FD = 0xFFFFFFFF


def test_ioctl_numbers_match_running_kernel_uapi():
    mod = gate()
    assert mod.GET_VERSION == 0x80084B01
    assert mod.ACQUIRE_VM == 0x40084B15
    assert mod.ALLOC_MEMORY_OF_GPU == 0xC0284B16
    assert mod.CREATE_QUEUE == 0xC0604B02


def test_packers_place_gpu_id_at_the_uapi_offsets():
    mod = gate()
    assert struct.unpack("=II", mod.pack_acquire_vm(DRM_FD, SELECTED)) == (
        DRM_FD,
        SELECTED,
    )
    assert struct.unpack_from("=I", mod.pack_alloc(SELECTED), 32)[0] == SELECTED
    assert len(mod.pack_create_queue(SELECTED)) == mod.CREATE_QUEUE_SIZE
    assert struct.unpack_from("=I", mod.pack_create_queue(SELECTED), 36)[0] == SELECTED


def test_foreign_gpu_is_excluded_while_the_selected_card_passes():
    mod = gate()
    fake = FakeKfd(mod, SELECTED, DRM_FD)
    foreign_no_fd = mod.probe_device(fake.ioctl, 0, FOREIGN, NO_FD)
    foreign_dgpu_fd = mod.probe_device(fake.ioctl, 0, FOREIGN, DRM_FD)
    selected = mod.probe_device(fake.ioctl, 0, SELECTED, DRM_FD)

    for name in mod.ATTEMPTS:
        assert foreign_no_fd[name] == errno.EINVAL
        assert foreign_dgpu_fd[name] == errno.EINVAL
    assert selected["acquire_vm"] == 0
    assert selected["alloc_memory_of_gpu"] == 0
    assert selected["create_queue"] == 0

    problems = mod.evaluate_exclusion(
        foreign_no_fd, foreign_dgpu_fd, selected, render_absent=True, device_count=1
    )
    assert problems == []


def test_a_reachable_foreign_gpu_or_render_node_fails_the_gate():
    mod = gate()
    selected = {name: 0 for name in mod.ATTEMPTS}
    # Foreign acquire_vm unexpectedly succeeded -> not excluded.
    leaky = {name: errno.EINVAL for name in mod.ATTEMPTS}
    leaky["acquire_vm"] = 0
    problems = mod.evaluate_exclusion(
        leaky,
        {n: errno.EINVAL for n in mod.ATTEMPTS},
        selected,
        render_absent=True,
        device_count=1,
    )
    assert any("foreign_no_fd.acquire_vm" in p for p in problems)

    # Foreign render node present and two HIP devices visible.
    clean = {name: errno.EINVAL for name in mod.ATTEMPTS}
    problems = mod.evaluate_exclusion(
        clean, clean, selected, render_absent=False, device_count=2
    )
    assert any("render node reachable" in p for p in problems)
    assert any("!= 1 selected device" in p for p in problems)


def test_selected_card_that_cannot_acquire_fails_the_positive_control():
    mod = gate()
    clean = {name: errno.EINVAL for name in mod.ATTEMPTS}
    broken_selected = {name: errno.EINVAL for name in mod.ATTEMPTS}
    problems = mod.evaluate_exclusion(
        clean, clean, broken_selected, render_absent=True, device_count=1
    )
    assert any("selected.acquire_vm" in p for p in problems)


def test_wrong_drm_fd_cannot_acquire_the_selected_vm():
    mod = gate()
    fake = FakeKfd(mod, SELECTED, DRM_FD)
    # Selected gpu_id but a foreign/absent drm fd still fails, as acquire_vm
    # binds the per-device VM only for the matching render node.
    assert (
        mod.attempt(fake.ioctl, 0, mod.ACQUIRE_VM, mod.pack_acquire_vm(NO_FD, SELECTED))
        == errno.EINVAL
    )


@pytest.mark.parametrize("name", ["acquire_vm", "alloc_memory_of_gpu", "create_queue"])
def test_payload_for_dispatches_each_attempt(name):
    mod = gate()
    payload = mod.payload_for(name, SELECTED, DRM_FD)
    assert isinstance(payload, (bytes, bytearray))
    assert len(payload) > 0
