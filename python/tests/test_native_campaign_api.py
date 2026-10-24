"""Qualification entrypoints must call the real shared lifecycle API/signatures."""

import ast
import importlib.util
import inspect
from pathlib import Path


def test_native_worker_campaigns_use_existing_shared_methods_and_valid_signatures(
    monkeypatch,
):
    scripts = Path(__file__).parents[2] / "scripts"
    monkeypatch.syspath_prepend(str(scripts))
    spec = importlib.util.spec_from_file_location(
        "native_worker_campaign", scripts / "native_worker_campaign.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    for name in (
        "verify_spectral_worker.py",
        "verify_freezing_worker.py",
        "verify_atmosphere_worker.py",
        "verify_atmospheric_transport_worker.py",
        "verify_snow_worker.py",
        "verify_study_worker.py",
        "verify_equal_accuracy_cpu.py",
        "verify_cad_variant_worker.py",
        "verify_coupled_thermal_results.py",
    ):
        tree = ast.parse((scripts / name).read_text())
        for node in ast.walk(tree):
            if (
                not isinstance(node, ast.Call)
                or not isinstance(node.func, ast.Attribute)
                or not isinstance(node.func.value, ast.Name)
                or node.func.value.id != "campaign"
            ):
                continue
            method = getattr(module.WorkerCampaign, node.func.attr, None)
            assert callable(method), (
                f"{name}:{node.lineno} calls missing shared lifecycle method {node.func.attr}"
            )
            # Each argument is supplied by the entrypoint at runtime; binding
            # proves its arity/keyword contract without manufacturing native work.
            inspect.signature(method).bind(
                None,
                *([None] * len(node.args)),
                **{k.arg: None for k in node.keywords if k.arg is not None},
            )


def test_native_campaigns_call_registered_mcp_tool_names():
    repo = Path(__file__).parents[2]
    server = ast.parse((repo / "python/harbor_cad_mcp/server.py").read_text())
    registered = {
        node.name
        for node in ast.walk(server)
        if isinstance(node, ast.AsyncFunctionDef)
        and any(
            isinstance(decorator, ast.Call)
            and isinstance(decorator.func, ast.Attribute)
            and decorator.func.attr == "tool"
            for decorator in node.decorator_list
        )
    }
    for filename in (
        "verify_snow_worker.py",
        "verify_atmospheric_transport_worker.py",
        "verify_atmosphere_worker.py",
        "verify_spectral_worker.py",
        "verify_study_worker.py",
        "verify_cad_variant_worker.py",
        "verify_coupled_thermal_results.py",
    ):
        for node in ast.walk(ast.parse((repo / "scripts" / filename).read_text())):
            if not isinstance(node, ast.Call):
                continue
            tool = None
            if isinstance(node.func, ast.Name) and node.func.id == "mcp_call":
                tool = node.args[1]
            elif isinstance(node.func, ast.Attribute) and node.func.attr in {
                "call_tool",
                "mcp_call",
            }:
                tool = node.args[0]
            if isinstance(tool, ast.Constant) and isinstance(tool.value, str):
                assert tool.value in registered, (
                    f"{filename}:{node.lineno}: unregistered MCP tool {tool.value}"
                )


def test_shared_packaged_stdio_call_preserves_worker_payload_and_expected_refusal(
    tmp_path, monkeypatch
):
    import json
    import os
    import subprocess
    import time

    import pytest

    repo = Path(__file__).parents[2]
    monkeypatch.syspath_prepend(str(repo / "scripts"))
    from native_worker_campaign import WorkerCampaign

    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    campaign = object.__new__(WorkerCampaign)
    campaign.mcp = repo / "python/.venv/bin/harbor-cad-mcp"
    campaign.endpoint = tmp_path / "state/worker.sock"
    campaign.environment = dict(os.environ)
    worker = subprocess.Popen(
        [
            binary,
            "worker",
            "--state",
            str(tmp_path / "state"),
            "--profile",
            str(repo / "profiles/ci.json"),
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        deadline = time.monotonic() + 5
        while not campaign.endpoint.exists():
            assert worker.poll() is None and time.monotonic() < deadline
            time.sleep(0.01)
        case = json.loads(subprocess.check_output([binary, "case", "init"]))
        plan = campaign.mcp_call("case_plan", {"case": case})
        expected = json.loads(
            subprocess.check_output(
                [binary, "case", "plan", "/dev/stdin"], input=json.dumps(case).encode()
            )
        )
        assert plan == expected["plan"]
        request = json.loads((repo / "examples/cad-variant.json").read_text())
        rejected = campaign.mcp_call(
            "cad_plan_variant", {"variant": request}, profile="cad", expect_error=True
        )
        assert "unqualified:" in str(rejected)
        with pytest.raises(RuntimeError, match="unqualified:"):
            campaign.mcp_call("cad_plan_variant", {"variant": request}, profile="cad")
        with pytest.raises(RuntimeError):
            campaign.mcp_call("case_plan", {"case": case}, expect_error=True)
    finally:
        worker.terminate()
        worker.communicate(timeout=5)
