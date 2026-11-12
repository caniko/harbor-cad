"""Small regression for the evaluation checker, not model or CAD qualification."""

from pathlib import Path
import json
import os
import subprocess
import sys
import tempfile
import unittest

import fixture


class FixtureCheck(unittest.TestCase):
    def test_trace_capture_uses_regular_stdout_and_refuses_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            producer = root / "opencode"
            producer.write_text(
                f"#!{sys.executable}\n"
                "import json, os, stat\n"
                "assert stat.S_ISREG(os.fstat(1).st_mode)\n"
                "messages = [{'type':'system','content':'PRIVATE'}, "
                "{'type':'user','text':'visible user'}, "
                "{'type':'assistant','finish':'stop','content':["
                "{'type':'reasoning','text':'PRIVATE'}, "
                "{'type':'text','text':'x'*300000+'END'}]}]\n"
                "os.write(1, json.dumps({'data':{'messages':messages}}).encode())\n"
            )
            producer.chmod(0o700)
            output = root / "trace.json"
            args = [
                sys.executable,
                "-B",
                str(Path(fixture.__file__).with_name("capture-trace.py")),
                "ses_example",
                str(output),
            ]
            env = {**os.environ, "PATH": f"{root}:{os.environ['PATH']}"}
            subprocess.run(args, env=env, check=True, capture_output=True, timeout=10)
            original = output.read_bytes()
            self.assertNotIn(b"PRIVATE", original)
            trace = json.loads(original)
            self.assertEqual(trace["history_source"], "full_session_export")
            self.assertEqual(trace["session_id"], "ses_example")
            self.assertEqual(
                trace["messages"][-1]["content"][0]["text"], "x" * 300000 + "END"
            )
            refused = subprocess.run(args, env=env, capture_output=True, timeout=10)
            self.assertNotEqual(refused.returncode, 0)
            self.assertEqual(output.read_bytes(), original)

    def test_documented_record_generation_binds_bytes_and_refuses_overwrite(self):
        reference = (
            Path(fixture.__file__).resolve().parent.parent
            / "references"
            / "record-provenance.md"
        )
        command = reference.read_text().split("```sh\n", 1)[1].split("\n```", 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            root, output = Path(directory) / "input", Path(directory) / "output"
            root.mkdir()
            output.mkdir()
            for name, data in fixture.inputs("contract-write").items():
                (root / name).write_bytes(data)
            (output / "requirements.json").write_bytes(
                fixture.encoded(fixture.approved())
            )
            env = {
                **os.environ,
                "SOURCE_REQUIREMENTS": str(root / "requirements.json"),
                "HISTORICAL_RECEIPT": str(root / "receipt.json"),
                "NEW_REQUIREMENTS": str(output / "requirements.json"),
                "CHANGE_RECORD": str(output / "change-record.json"),
            }
            args = ["bash", "-c", command]
            subprocess.run(args, env=env, check=True, capture_output=True, timeout=10)
            fixture.check("contract-write", root, output)
            original = (output / "change-record.json").read_bytes()
            refused = subprocess.run(args, env=env, capture_output=True, timeout=10)
            self.assertNotEqual(refused.returncode, 0)
            self.assertEqual((output / "change-record.json").read_bytes(), original)
            odd = root / "receipt with spaces\\and\nnewline.json"
            odd.write_bytes((root / "receipt.json").read_bytes())
            env.update(
                HISTORICAL_RECEIPT=str(odd), CHANGE_RECORD=str(output / "odd.json")
            )
            subprocess.run(args, env=env, check=True, capture_output=True, timeout=10)
            record = json.loads((output / "odd.json").read_bytes())
            self.assertEqual(
                record["historical_receipt_sha256"], fixture.digest(odd.read_bytes())
            )
            env.update(
                HISTORICAL_RECEIPT=str(root / "absent.json"),
                CHANGE_RECORD=str(output / "missing.json"),
            )
            refused = subprocess.run(args, env=env, capture_output=True, timeout=10)
            self.assertNotEqual(refused.returncode, 0)
            self.assertFalse((output / "missing.json").exists())

    def test_numeric_report_matches_native_renderer_and_rejects_edits(self):
        with tempfile.TemporaryDirectory() as directory:
            root, output = Path(directory) / "input", Path(directory) / "output"
            root.mkdir()
            for name, data in fixture.inputs("bounded-objective").items():
                (root / name).write_bytes(data)
            output.mkdir()
            cli = str(
                Path(fixture.__file__).resolve().parents[4] / "target/debug/harbor-cad"
            )
            command = [cli, "design", "screen", str(root / "screen.json")]
            report = subprocess.run(
                command, check=True, capture_output=True, timeout=10
            ).stdout
            markdown = subprocess.run(
                command + ["--markdown"], check=True, capture_output=True, timeout=10
            ).stdout
            (output / "report.json").write_bytes(report)
            (output / "response.md").write_bytes(markdown)
            fixture.check("bounded-objective", root, output)
            (output / "response.md").write_bytes(
                markdown + b"Next worst is wall 0.10.\n"
            )
            with self.assertRaises(AssertionError):
                fixture.check("bounded-objective", root, output)
            (output / "response.md").write_bytes(markdown)
            value = json.loads(report)
            value["ledger"]["budget_spare"] = 13
            (output / "report.json").write_bytes(fixture.encoded(value))
            with self.assertRaises(AssertionError):
                fixture.check("bounded-objective", root, output)

    def test_trace_export_keeps_visible_history_only(self):
        messages = [
            {"type": "system", "content": "PRIVATE"},
            {"type": "user", "text": "visible user"},
            {
                "type": "assistant",
                "finish": "stop",
                "metadata": {"secret": "PRIVATE"},
                "content": [
                    {"type": "reasoning", "text": "PRIVATE"},
                    {"type": "text", "text": "visible answer"},
                    {
                        "type": "tool",
                        "name": "read",
                        "state": {
                            "status": "completed",
                            "input": {"path": "example"},
                            "content": [{"type": "text", "text": "visible result"}],
                            "metadata": {"secret": "PRIVATE"},
                        },
                    },
                ],
            },
        ]
        for data in (messages, {"info": {"secret": "PRIVATE"}, "messages": messages}):
            result = subprocess.run(
                ["jq", "-f", str(Path(fixture.__file__).with_name("export-trace.jq"))],
                input=json.dumps({"data": data}),
                text=True,
                capture_output=True,
                check=True,
                timeout=10,
                env={**os.environ, "SESSION_ID": "example"},
            )
            self.assertNotIn("PRIVATE", result.stdout)
            trace = json.loads(result.stdout)
            self.assertTrue(trace["has_completed_answer"])
            self.assertEqual(len(trace["messages"]), 2)
            self.assertEqual(trace["messages"][0]["content"][0]["text"], "visible user")
            self.assertEqual(
                trace["messages"][1]["content"][1]["input"], {"path": "example"}
            )

    def test_good_and_tampered_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            root, output = Path(directory) / "input", Path(directory) / "output"
            command = [
                sys.executable,
                "-B",
                str(Path(fixture.__file__)),
                "scaffold",
                "contract-write",
                str(root),
            ]
            subprocess.run(command, check=True, timeout=10)
            refused = subprocess.run(command, capture_output=True, timeout=10)
            self.assertNotEqual(refused.returncode, 0)
            output.mkdir()
            new = fixture.encoded(fixture.approved())
            (output / "requirements.json").write_bytes(new)
            record = {
                "source_requirements_sha256": fixture.digest(
                    (root / "requirements.json").read_bytes()
                ),
                "historical_receipt_sha256": fixture.digest(
                    (root / "receipt.json").read_bytes()
                ),
                "requirements_sha256": fixture.digest(new),
                "executed": False,
                "qualification": "unqualified",
                "approval_required": True,
            }
            path = output / "change-record.json"
            path.write_bytes(fixture.encoded(record))
            fixture.check("contract-write", root, output)
            for bad in (True, 0):
                path.write_bytes(fixture.encoded({**record, "executed": bad}))
                with self.assertRaises(AssertionError):
                    fixture.check("contract-write", root, output)
            path.write_bytes(fixture.encoded(record))
            duplicate = path.read_text().replace(
                '"executed": false', '"executed": true, "executed": false'
            )
            path.write_text(duplicate)
            with self.assertRaises(AssertionError):
                fixture.check("contract-write", root, output)
            path.write_bytes(fixture.encoded(record))
            (root / "receipt.json").write_text("{}")
            with self.assertRaises(AssertionError):
                fixture.check("contract-write", root, output)

    def test_source_parser_and_preservation(self):
        import collections
        import xml.etree.ElementTree as ET
        import zipfile

        with tempfile.TemporaryDirectory() as directory:
            root, output = Path(directory) / "input", Path(directory) / "output"
            root.mkdir()
            for name, data in fixture.inputs("source-guard").items():
                (root / name).write_bytes(data)
            output.mkdir()
            with zipfile.ZipFile(root / "source.FCStd") as archive:
                tree = ET.fromstring(archive.read("Document.xml"))
            types = dict(
                collections.Counter(
                    obj.attrib["type"] for obj in tree.findall("./Objects/Object")
                )
            )
            report = {
                "source_sha256": fixture.digest((root / "source.FCStd").read_bytes()),
                "object_types": types,
                "parametric": False,
                "keepouts_usable": False,
                "lfs_materialized": False,
                "execution": "not_run",
                "product_equivalent": False,
                "baseline_conflict": True,
                "overwrite_blocked": True,
                "unknown_is_pass": False,
            }
            (output / "audit.json").write_bytes(fixture.encoded(report))
            (output / "summary.md").write_text(
                "Synthetic source audit only; generation blocked."
            )
            fixture.check("source-guard", root, output)
            for name in ("output.FCStd", "manifest.json", "unrelated.txt"):
                old = (root / name).read_bytes()
                (root / name).write_text("tampered")
                with self.assertRaises(AssertionError):
                    fixture.check("source-guard", root, output)
                (root / name).write_bytes(old)


if __name__ == "__main__":
    unittest.main()
