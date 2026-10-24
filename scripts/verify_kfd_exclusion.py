"""Direct-evidence exclusion of a foreign KFD GPU from the HIP sandbox.

Extends ``scripts/verify_kfd_vm.py``'s method: raw amdkfd ioctls in a fresh
bubblewrap namespace on the running kernel, with no HIP runtime. Where that
fixture proves the *selected* card is reachable, this one proves a *second*
live KFD GPU is not: inside the exact HIP sandbox (``/dev/kfd`` plus only the
selected dGPU render node) it reads KFD topology, finds the foreign GPU's
``gpu_id`` and attempts ``ACQUIRE_VM`` (no fd, and the visible dGPU fd),
``ALLOC_MEMORY_OF_GPU`` and ``CREATE_QUEUE`` against it -- every attempt must
fail with ``EINVAL`` -- while the same sequence on the selected card acquires
its VM and allocates (positive control). It also checks the foreign render
node is absent and, when a probe is supplied, that ``hipGetDeviceCount``
reports exactly one device.

Grounding (running kernel 7.2.8-cachyos-lto, CachyOS release cachyos-7.2.8-1,
drivers/gpu/drm/amd/amdkfd/kfd_chardev.c):
  * ``kfd_ioctl_acquire_vm`` ``fget(drm_fd)`` -> NULL gives ``-EINVAL``; it then
    looks the ``gpu_id`` up with ``kfd_process_device_data_by_id`` (NULL ->
    ``-EINVAL``) before ``kfd_process_device_init_vm`` binds that card's pdd.
  * ``kfd_ioctl_alloc_memory_of_gpu`` and ``kfd_ioctl_create_queue`` resolve the
    same per-id pdd; without the foreign render node no pdd exists for it
    (``kfd_init_apertures`` skips devices ``kfd_devcgroup_check_permission``
    denies, and the sandbox exposes only the selected render node), so each
    returns ``-EINVAL``.
The per-device ``drm_file``/VM requirement is why withholding the render node
is sufficient: KFD cannot acquire or use a card whose DRM fd is unreachable.

The probe cannot run today (the iGPU is quarantined on vfio-pci, not amdgpu),
so the orchestration is best-effort and the load-bearing logic lives in pure,
mockable functions exercised by ``python/tests/test_kfd_exclusion.py``.
"""

import argparse
import errno
import gzip
import hashlib
import json
import struct
import sys
import time
from pathlib import Path

# ioctl encodings (asm-generic/ioctl.h), matched to amdkfd AMDKFD_IO* macros.
_IOC_NRBITS = 8
_IOC_TYPEBITS = 8
_IOC_SIZEBITS = 14
_IOC_NONE = 0
_IOC_WRITE = 1
_IOC_READ = 2
_KFD = ord("K")

ACQUIRE_VM_SIZE = 8  # struct kfd_ioctl_acquire_vm_args {u32 drm_fd, gpu_id}
ALLOC_SIZE = 40  # 4*u64 + 2*u32
CREATE_QUEUE_SIZE = 96  # struct kfd_ioctl_create_queue_args
CREATE_QUEUE_GPU_ID_OFFSET = 36

# KFD_IOC_ALLOC_MEM_FLAGS_GTT | KFD_IOC_ALLOC_MEM_FLAGS_WRITABLE
ALLOC_FLAGS_GTT_WRITABLE = (1 << 1) | (1 << 31)

ATTEMPTS = ("acquire_vm", "alloc_memory_of_gpu", "create_queue")
EXPECTED_FOREIGN_ERRNO = errno.EINVAL
SANDBOX_POLICY = "harbor-cad-native-hip-single-kfd-v1"
EXCLUSION_POLICY = "harbor-cad-native-hip-kfd-exclusion-v1"


def _ioc(direction, size):
    return (
        (direction << (_IOC_NRBITS + _IOC_TYPEBITS + _IOC_SIZEBITS))
        | (size << (_IOC_NRBITS + _IOC_TYPEBITS))
        | (_KFD << _IOC_NRBITS)
    )


GET_VERSION = _ioc(_IOC_READ, ACQUIRE_VM_SIZE) | 0x01
ACQUIRE_VM = _ioc(_IOC_WRITE, ACQUIRE_VM_SIZE) | 0x15
ALLOC_MEMORY_OF_GPU = _ioc(_IOC_READ | _IOC_WRITE, ALLOC_SIZE) | 0x16
CREATE_QUEUE = _ioc(_IOC_READ | _IOC_WRITE, CREATE_QUEUE_SIZE) | 0x02

REQUESTS = {
    "acquire_vm": ACQUIRE_VM,
    "alloc_memory_of_gpu": ALLOC_MEMORY_OF_GPU,
    "create_queue": CREATE_QUEUE,
}


def pack_acquire_vm(drm_fd, gpu_id):
    return struct.pack("=II", drm_fd & 0xFFFFFFFF, gpu_id)


def pack_alloc(gpu_id):
    # va_addr, size, handle, mmap_offset (u64) then gpu_id, flags (u32).
    return struct.pack("=QQQQII", 0, 4096, 0, 0, gpu_id, ALLOC_FLAGS_GTT_WRITABLE)


def pack_create_queue(gpu_id):
    buffer = bytearray(CREATE_QUEUE_SIZE)
    struct.pack_into("=I", buffer, CREATE_QUEUE_GPU_ID_OFFSET, gpu_id)
    return bytes(buffer)


def payload_for(name, gpu_id, drm_fd):
    if name == "acquire_vm":
        return pack_acquire_vm(drm_fd, gpu_id)
    if name == "alloc_memory_of_gpu":
        return pack_alloc(gpu_id)
    return pack_create_queue(gpu_id)


def attempt(ioctl, kfd_fd, request, payload):
    """Run one ioctl; return 0 on success or its errno on failure."""
    try:
        ioctl(kfd_fd, request, bytearray(payload), True)
        return 0
    except OSError as error:
        return error.errno if error.errno else errno.EIO


def probe_device(ioctl, kfd_fd, gpu_id, drm_fd):
    """Attempt the three VM/memory/queue ioctls against one gpu_id."""
    return {
        name: attempt(ioctl, kfd_fd, REQUESTS[name], payload_for(name, gpu_id, drm_fd))
        for name in ATTEMPTS
    }


def evaluate_exclusion(
    foreign_no_fd, foreign_dgpu_fd, selected, render_absent, device_count
):
    """Return the list of exclusion-contract violations; empty means pass."""
    problems = []
    for label, results in (
        ("foreign_no_fd", foreign_no_fd),
        ("foreign_dgpu_fd", foreign_dgpu_fd),
    ):
        for name in ATTEMPTS:
            observed = results.get(name)
            if observed != EXPECTED_FOREIGN_ERRNO:
                problems.append(
                    f"{label}.{name} errno {observed} != EINVAL "
                    f"({EXPECTED_FOREIGN_ERRNO}); foreign GPU not excluded"
                )
    # Positive control: the selected card must acquire its VM and allocate,
    # otherwise a foreign EINVAL could be a broken sandbox, not an exclusion.
    for name in ("acquire_vm", "alloc_memory_of_gpu"):
        observed = selected.get(name)
        if observed != 0:
            problems.append(f"selected.{name} errno {observed} != success")
    if not render_absent:
        problems.append("foreign render node reachable inside the sandbox")
    if device_count not in (None, 1):
        problems.append(f"hipGetDeviceCount {device_count} != 1 selected device")
    return problems


def _sha256_file(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read_kernel_identity():
    release = Path("/proc/sys/kernel/osrelease").read_text().strip()
    proc_version = Path("/proc/version").read_bytes()
    config_digest = None
    config = Path("/proc/config.gz")
    if config.exists():
        config_digest = hashlib.sha256(gzip.decompress(config.read_bytes())).hexdigest()
    return {
        "release": release,
        "proc_version": proc_version.decode().strip(),
        "proc_version_sha256": hashlib.sha256(proc_version).hexdigest(),
        "kernel_config_sha256": config_digest,
    }


def read_topology_gpus(root):
    """Map canonical PCI -> {gpu_id, drm_render_minor} for live KFD GPUs."""
    gpus = {}
    for node in sorted((root / "nodes").iterdir()):
        gpu_id = int((node / "gpu_id").read_text())
        properties = dict(
            line.split()
            for line in (node / "properties").read_text().splitlines()
            if line.split()
        )
        simd = int(properties.get("simd_count", "0"))
        if gpu_id == 0 and simd == 0:
            continue
        domain = int(properties["domain"])
        location = int(properties["location_id"])
        pci = (
            f"{domain:04x}:{(location >> 8) & 0xFF:02x}:"
            f"{(location >> 3) & 31:02x}.{location & 7}"
        )
        gpus[pci] = {
            "gpu_id": gpu_id,
            "drm_render_minor": int(properties.get("drm_render_minor", "-1")),
        }
    return gpus


def build_receipt(args, kernel, gpus, probes, device_count, problems):
    return {
        "scope": (
            "Direct raw-ioctl exclusion of the foreign KFD GPU from the HIP "
            "sandbox on this exact running kernel. No claim beyond ACQUIRE_VM, "
            "ALLOC_MEMORY_OF_GPU and CREATE_QUEUE, this kernel build, and these "
            "two cards; not a durable or numerical GPU qualification."
        ),
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "sandbox_policy": SANDBOX_POLICY,
        "exclusion_policy": EXCLUSION_POLICY,
        "kernel": kernel,
        "amdkfd_source": {
            "path": str(args.kfd_source),
            "sha256": _sha256_file(args.kfd_source),
        },
        "selected": {
            "pci": args.selected_pci,
            "gpu_id": gpus.get(args.selected_pci, {}).get("gpu_id"),
        },
        "foreign": [
            {"pci": pci, "gpu_id": gpus.get(pci, {}).get("gpu_id")}
            for pci in args.foreign_pci
        ],
        "probes": probes,
        "hip_device_count": device_count,
        "expected_foreign_errno": EXPECTED_FOREIGN_ERRNO,
        "violations": problems,
        "passed": not problems,
    }


def run_child(gpu_id, drm_fd):
    """Sandbox-side raw probe: validate KFD UAPI, probe one gpu_id, print JSON.

    Invoked inside the bubblewrap namespace that exposes /dev/kfd and only the
    selected render node. ``drm_fd`` is a file descriptor already open in the
    child (the selected render node, or -1 for no fd).
    """
    import fcntl  # only the sandbox child needs it

    with open("/dev/kfd", "rb", buffering=0) as kfd:
        version = bytearray(ACQUIRE_VM_SIZE)
        fcntl.ioctl(kfd, GET_VERSION, version, True)
        major, _ = struct.unpack("=II", version)
        if major != 1:
            raise ValueError("unsupported KFD UAPI major version")
        results = probe_device(fcntl.ioctl, kfd, gpu_id, drm_fd)
    print(json.dumps(results))


def _load_results(path):
    if path is None:
        return None
    return json.loads(Path(path).read_text())


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--selected-pci", required=True)
    parser.add_argument("--foreign-pci", action="append", required=True)
    parser.add_argument(
        "--topology",
        type=Path,
        default=Path("/sys/devices/virtual/kfd/kfd/topology"),
    )
    parser.add_argument(
        "--kfd-source",
        type=Path,
        required=True,
        help="amdkfd source actually read for the ioctl mechanism",
    )
    parser.add_argument("--output", type=Path, required=True)
    # Sandbox-child probe JSON (from run_child). Absent while on security hold.
    parser.add_argument("--foreign-no-fd-results", type=Path)
    parser.add_argument("--foreign-dgpu-fd-results", type=Path)
    parser.add_argument("--selected-results", type=Path)
    parser.add_argument("--hip-device-count", type=int)
    parser.add_argument("--foreign-render-absent", action="store_true")
    args = parser.parse_args(argv)

    kernel = read_kernel_identity()
    gpus = read_topology_gpus(args.topology) if args.topology.exists() else {}

    foreign_no_fd = _load_results(args.foreign_no_fd_results)
    foreign_dgpu_fd = _load_results(args.foreign_dgpu_fd_results)
    selected = _load_results(args.selected_results)

    if foreign_no_fd and foreign_dgpu_fd and selected:
        problems = evaluate_exclusion(
            foreign_no_fd,
            foreign_dgpu_fd,
            selected,
            args.foreign_render_absent,
            args.hip_device_count,
        )
        probes = {
            "foreign_no_fd": foreign_no_fd,
            "foreign_dgpu_fd": foreign_dgpu_fd,
            "selected": selected,
        }
    else:
        # The sandboxed child cannot run while the foreign GPU is on vfio-pci;
        # see the module docstring. The receipt records a security hold rather
        # than a pass, and the exclusion logic stays unit-tested with a mock.
        problems = ["security_hold: foreign GPU on vfio-pci, probe not executed"]
        probes = None

    receipt = build_receipt(args, kernel, gpus, probes, args.hip_device_count, problems)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2))
    print(json.dumps(receipt, indent=2))
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    if len(sys.argv) >= 2 and sys.argv[1] == "--child":
        run_child(int(sys.argv[2]), int(sys.argv[3]))
    else:
        sys.exit(main(sys.argv[1:]))
