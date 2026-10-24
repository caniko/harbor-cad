"""Doctor identifies compiled dependency locks, never the caller's checkout."""

import hashlib
import json
import os
import subprocess
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator, ValidationError


def test_doctor_compiled_lock_provenance_is_not_checkout_or_runtime_qualification(
    tmp_path, monkeypatch
):
    binary = os.environ["HARBOR_CAD_TEST_BINARY"]
    repo = Path(__file__).parents[2]
    doctor = json.loads(subprocess.check_output([binary, "doctor"]))
    provenance = doctor["build_provenance"]
    assert provenance["schema_version"] == 1
    locks = {
        "flake.lock": "flake.lock",
        "Cargo.lock": "Cargo.lock",
        "uv.lock": "python/uv.lock",
    }
    assert provenance["lockfiles_sha256"] == {
        name: hashlib.sha256((repo / path).read_bytes()).hexdigest()
        for name, path in locks.items()
    }
    lock = json.loads((repo / "flake.lock").read_text())
    assert provenance["flake_inputs"] == {
        name: {
            "revision": lock["nodes"][node]["locked"]["rev"],
            "nar_hash": lock["nodes"][node]["locked"]["narHash"],
        }
        for name, node in lock["nodes"][lock["root"]]["inputs"].items()
    }
    assert provenance["fleetix_contract_sha256"] == doctor["fleetix_contract_digest"]
    assert provenance["sqlite_version"] == "3.53.2"
    assert provenance["qualification"] == "not_assessed"
    assert provenance["executed"] is False
    schemas = json.loads(subprocess.check_output([binary, "schema"]))
    validator = Draft202012Validator(schemas["BuildProvenance"])
    validator.validate(provenance)
    with pytest.raises(ValidationError):
        validator.validate({**provenance, "native_builds_passed": True})
    monkeypatch.chdir(tmp_path)
    for name in ("flake.lock", "Cargo.lock", "uv.lock"):
        (tmp_path / name).write_text("not the binary's compiled lock")
    assert (
        json.loads(subprocess.check_output([binary, "doctor"]))["build_provenance"]
        == provenance
    )
