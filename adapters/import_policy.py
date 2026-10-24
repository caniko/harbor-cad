"""Fixed importer environment checks; no document-supplied operations or paths."""

import errno
import os
import resource
import socket
import time
from pathlib import Path

POLICY = "harbor-cad-importer-v1"


def verify_mounts(mounts, closure, operation):
    readonly = {"/plan.json", "/import-runtime-closure.txt", *closure}
    if operation == "cad_inspect":
        readonly.add("/input.FCStd")
    if any("ro" not in mounts.get(path, set()) for path in readonly):
        raise RuntimeError("importer plan/input/package mounts must be read-only")
    visible = {p for p in mounts if p.startswith("/nix/store/")}
    if "/nix/store" in mounts or visible != closure:
        raise RuntimeError("importer may see only its declared package closure")
    if "rw" not in mounts.get("/work", set()):
        raise RuntimeError("private writable importer output required")


def verify_privileges(status):
    if status.get("NoNewPrivs") != "1" or int(status.get("CapEff", "-1"), 16) != 0:
        raise RuntimeError("importer effective privileges differ from approved policy")


def readonly_open(path):
    try:
        descriptor = os.open(path, os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK)
    except OSError as error:
        if error.errno != errno.EROFS:
            raise RuntimeError(
                "read-only mount enforcement was not demonstrated"
            ) from error
    else:
        os.close(descriptor)
        raise RuntimeError("approved input unexpectedly opened for writing")


def qualification_probes():
    root = os.environ.get("HARBOR_CAD_IMPORT_PROBE_ROOT")
    port = os.environ.get("HARBOR_CAD_IMPORT_PROBE_PORT")
    if root is None and port is None:
        return {"scope": "operator canaries not requested"}
    if root is None or port is None:
        raise RuntimeError("complete operator qualification canaries required")
    root = Path(root)
    if not root.is_absolute() or ".." in root.parts or not 1024 <= int(port) <= 65535:
        raise RuntimeError(
            "bounded absolute canary root and host-loopback port required"
        )
    if (
        root / "credential.canary"
    ).exists() or "HARBOR_CAD_CREDENTIAL_SENTINEL" in os.environ:
        raise RuntimeError("host credential canary leaked into importer")
    for family, endpoint in [
        (socket.AF_UNIX, str(root / "session.sock")),
        (socket.AF_INET, ("127.0.0.1", int(port))),
    ]:
        with socket.socket(family) as connection:
            connection.settimeout(0.25)
            try:
                connection.connect(endpoint)
            except OSError:
                pass
            else:
                raise RuntimeError(
                    "host session/network canary reachable from importer"
                )
    child = os.fork()
    if child == 0:
        # Bounded benign detached descendant, entirely inside private scratch.
        # NativeProcess/systemd must terminate it when the importer returns.
        os.setsid()
        deadline = time.monotonic() + 60
        try:
            while time.monotonic() < deadline:
                fd = os.open(
                    "/work/import-descendant-heartbeat",
                    os.O_WRONLY | os.O_CREAT | os.O_APPEND,
                    0o600,
                )
                os.write(fd, b"x")
                os.close(fd)
                time.sleep(0.05)
        finally:
            os._exit(0)
    return {
        "scope": "live operator-owned host canaries and bounded detached descendant",
        "credential_canary_hidden": True,
        "credential_environment_hidden": True,
        "session_socket_denied": True,
        "host_loopback_denied": True,
        "descendant_namespace_pid": child,
        "descendant_duration_limit_s": 60,
    }


def verify_import_environment(operation):
    if (
        operation not in {"cad_inspect", "cad_fixture", "enclosure_fixture"}
        or os.environ.get("HARBOR_CAD_IMPORT_POLICY") != POLICY
    ):
        raise RuntimeError("explicit operation-specific importer policy required")
    closure = set(Path("/import-runtime-closure.txt").read_text().splitlines())
    mounts = {}
    for line in Path("/proc/self/mountinfo").read_text().splitlines():
        fields = line.split()
        mount = fields[4]
        for encoded, decoded in [(r"\040", " "), (r"\011", "\t"), (r"\134", "\\")]:
            mount = mount.replace(encoded, decoded)
        mounts[mount] = set(fields[5].split(","))
    verify_mounts(mounts, closure, operation)
    readonly_open("/plan.json")
    if operation == "cad_inspect":
        readonly_open("/input.FCStd")
    status = dict(
        line.split(":", 1)
        for line in Path("/proc/self/status").read_text().splitlines()
        if ":" in line
    )
    status = {key: value.strip() for key, value in status.items()}
    verify_privileges(status)
    denied_env = [
        "DBUS_SESSION_BUS_ADDRESS",
        "XDG_RUNTIME_DIR",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "SSH_AUTH_SOCK",
    ]
    if any(name in os.environ for name in denied_env) or Path("/run/user").exists():
        raise RuntimeError("host session environment/runtime must be absent")
    if Path("/dev/dri").exists() or list(Path("/dev").glob("nvidia*")):
        raise RuntimeError("importer must not have GPU devices")
    netns = os.readlink("/proc/self/ns/net")
    if (
        not os.environ.get("HARBOR_CAD_HOST_NETNS")
        or netns == os.environ["HARBOR_CAD_HOST_NETNS"]
    ):
        raise RuntimeError("importer host network namespace isolation required")
    if {name for _, name in socket.if_nameindex()} != {"lo"}:
        raise RuntimeError("importer network must expose only isolated loopback")
    file_limit = resource.getrlimit(resource.RLIMIT_FSIZE)
    if (
        file_limit[0] <= 0
        or file_limit[0] > file_limit[1]
        or resource.RLIM_INFINITY in file_limit
    ):
        raise RuntimeError("bounded importer output resource limit required")
    return {
        "schema_version": 1,
        "policy": POLICY,
        "operation": operation,
        "store_mounts": sorted(closure),
        "package_mounts_read_only": True,
        "plan_read_only": True,
        "input_read_only": operation == "cad_inspect",
        "gpu_devices_absent": True,
        "host_session_environment_absent": True,
        "net_namespace": netns,
        "host_net_namespace": os.environ["HARBOR_CAD_HOST_NETNS"],
        "network_interfaces": ["lo"],
        "no_new_privileges": True,
        "effective_capabilities": status["CapEff"],
        "file_size_limit_bytes": list(file_limit),
        "qualification_probes": qualification_probes(),
    }
