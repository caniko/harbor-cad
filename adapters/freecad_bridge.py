"""Trusted fixed FreeCAD script, launched only inside the import sandbox.

Native FreeCAD Python ABI, never imported by MCP. The patched 1.1.4 importer
is mandatory even with safe-mode and macro preferences disabled.
"""

import hashlib
import importlib
import json
import os
import sys
from pathlib import Path

import FreeCAD as App
import MeshPart

sys.path.insert(0, "@policy_dir@")
verify_import_environment = importlib.import_module(
    "harbor_cad_import_policy"
).verify_import_environment


def atomic_json(path, value):
    partial = Path(str(path) + ".partial")
    partial.write_text(json.dumps(value, allow_nan=False, indent=2))
    partial.replace(path)


def main():
    op = os.environ["HARBOR_CAD_OPERATION"]
    plan = json.loads(Path(os.environ["HARBOR_CAD_PLAN"]).read_text())
    case = plan["case"]
    version = tuple(int(v) for v in App.Version()[:3])
    if version < (1, 1, 4):
        raise RuntimeError("FreeCAD >= 1.1.4 security fixes required")
    isolation = verify_import_environment(op)
    atomic_json("/work/import-isolation.json", isolation)
    App.ParamGet("User parameter:BaseApp/Preferences/Macro").SetBool("AutoRun", False)
    if op == "cad_fixture":
        if not case["geometry"]["synthetic"]:
            raise ValueError("fixture must be explicitly synthetic")
        if case["length"]["unit"] != "m" or case["channel_height"]["unit"] != "m":
            raise ValueError("synthetic fixture requires SI length inputs")
        doc = App.newDocument("HarborSyntheticChannel")
        fluid = doc.addObject("Part::Box", "fluid")
        fluid.Label = "synthetic fluid channel"
        fluid.Length = case["length"]["value"] * 1000
        fluid.Width = case["channel_height"]["value"] * 1000
        fluid.Height = case["channel_height"]["value"] * 1000
        doc.recompute()
        doc.saveAs("/work/source.FCStd")
    elif op == "enclosure_fixture":
        if not case["geometry"]["synthetic"]:
            raise ValueError("fixture must be explicitly synthetic")
        doc = App.newDocument("HarborSyntheticEnclosure")
        importlib.import_module("harbor_cad_enclosure_fixture").build(doc)
        doc.saveAs("/work/source.FCStd")
    elif op == "cad_inspect":
        # No inspection outside sandbox precedes this open.
        sheet_variant = "cad_sheet_variant" in plan
        variant = plan.get("cad_sheet_variant" if sheet_variant else "cad_variant")
        if variant is not None:
            raw = Path("/input.FCStd").read_bytes()
            if (
                len(raw) != variant["document"]["bytes"]
                or hashlib.sha256(raw).hexdigest() != variant["document"]["sha256"]
            ):
                raise ValueError("exact registered original CAD document required")
        doc = App.openDocument("/input.FCStd")
        if variant is not None:
            if plan["schema_version"] != 16:
                raise ValueError("distinct approved variant recipe required")
            native = importlib.import_module("harbor_cad_cad_variant")
            tolerance = case["geometry_tolerance"]["value"]
            result = (
                native.apply_sheet(
                    doc, variant, tolerance, importlib.import_module("Part")
                )
                if sheet_variant
                else native.apply(doc, variant, tolerance)
            )
            doc.saveAs("/work/variant.FCStd")
            output = Path("/work/variant.FCStd")
            if (
                output.is_symlink()
                or not output.is_file()
                or not 0 < output.stat().st_size <= 64 * 1024**2
            ):
                raise ValueError("bounded new closed variant document required")
            saved = output.read_bytes()
            if Path("/input.FCStd").read_bytes() != raw:
                raise ValueError("original CAD bytes changed during variant execution")
            atomic_json(
                "/work/cad-variant-recompute.json",
                {
                    "schema_version": 2 if sheet_variant else 1,
                    "approved_variant": variant,
                    "source_preserved": True,
                    **result,
                    "document": {
                        "path": "variant.FCStd",
                        "sha256": hashlib.sha256(saved).hexdigest(),
                        "bytes": len(saved),
                    },
                },
            )
        else:
            doc.recompute()
    else:
        raise ValueError("allowlisted CAD operation required")
    if any(obj.TypeId == "Spreadsheet::Sheet" for obj in doc.Objects):
        # Registered alias evidence lets version-2 planning refuse unknown
        # sheets/aliases before any document is opened again.
        atomic_json(
            "/work/sheets.json",
            importlib.import_module("harbor_cad_cad_variant").sheets(doc),
        )
    regions = []
    breps = []
    required = set(case["regions"])
    if op == "cad_fixture":
        # This procedural channel's walls are implicit fluid-boundary surfaces.
        required.discard("wall")
    for obj in doc.Objects:
        if obj.Name not in required:
            continue
        # Initial BREP/world-coordinate scope: top-level native Part geometry.
        # Assembly/link transforms need a verified traversal; local bounds must
        # never be relabelled as world bounds. Pinned DocumentObjectPyImp.cpp
        # defines getParentGeoFeatureGroup() as enclosing group or None.
        if (
            not obj.isDerivedFrom("Part::Feature")
            or obj.getParentGeoFeatureGroup() is not None
        ):
            raise ValueError(
                "top-level native Part solid required; assembly/link world transforms are not qualified"
            )
        if not hasattr(obj, "Shape") or obj.Shape.isNull() or not obj.Shape.isValid():
            raise ValueError("named region lacks valid geometry")
        if len(obj.Shape.Solids) != 1:
            raise ValueError("region must identify one unambiguous solid")
        tolerance = case["geometry_tolerance"]
        if tolerance["unit"] != "m":
            raise ValueError("explicit SI geometry tolerance required")
        mesh = MeshPart.meshFromShape(
            Shape=obj.Shape,
            LinearDeflection=tolerance["value"] * 1000,
            AngularDeflection=0.1,
            Relative=False,
        )
        mesh.write(f"/work/{obj.Name}.stl")
        bounds = obj.Shape.BoundBox
        # Export the approved solid itself, including its placement, through the
        # pinned TopoShape API. No reconstruction, healing or ordinal faces.
        # https://github.com/FreeCAD/FreeCAD/blob/4fd3bf320d9566a27e60069fc8387448aaa3a094/src/Mod/Part/App/TopoShapePyImp.cpp#L396
        brep_path = Path(f"/work/{obj.Name}.brep")
        partial = brep_path.with_suffix(".brep.partial")
        obj.Shape.exportBrep(str(partial))
        if (
            partial.is_symlink()
            or not partial.is_file()
            or not 0 < partial.stat().st_size <= 64 * 1024**2
        ):
            raise ValueError("bounded closed BREP geometry required")
        raw = partial.read_bytes()
        partial.replace(brep_path)
        regions.append(
            {
                "name": obj.Name,
                "label": obj.Label,
                "volume_m3": obj.Shape.Volume * 1e-9,
                "bounds_m": [
                    bounds.XMin / 1000,
                    bounds.XMax / 1000,
                    bounds.YMin / 1000,
                    bounds.YMax / 1000,
                    bounds.ZMin / 1000,
                    bounds.ZMax / 1000,
                ],
                "transform": list(obj.Placement.toMatrix().A),
                "triangles": mesh.CountFacets,
                "source_unit": "mm",
                "stl_scale_to_m": 0.001,
            }
        )
        breps.append(
            {
                "region_name": obj.Name,
                "path": brep_path.name,
                "bytes": len(raw),
                "sha256": hashlib.sha256(raw).hexdigest(),
                "source_unit": "mm",
                "scale_to_m": 0.001,
                "bounds_m": regions[-1]["bounds_m"],
                "volume_m3": regions[-1]["volume_m3"],
                "source_transform": regions[-1]["transform"],
                "placement_translation_unit": "mm",
            }
        )
    if {r["name"] for r in regions} != required:
        raise ValueError(
            "missing or ambiguous named regions; face numbering is not accepted"
        )
    atomic_json(
        "/work/regions.json",
        {
            "synthetic": case["geometry"]["synthetic"],
            "regions": regions,
            "gap_healing": False,
            "geometry_tolerance": case["geometry_tolerance"],
        },
    )
    atomic_json(
        "/work/brep-manifest.json",
        {
            "schema_version": 1,
            "synthetic": case["geometry"]["synthetic"],
            "regions": breps,
            "gap_healing": False,
        },
    )
    atomic_json(
        f"/work/{op}-receipt.json",
        {
            "adapter": "FreeCAD",
            "version": App.Version(),
            "executed": True,
            "backend": "cpu",
            "software_fallback": False,
            "security_minimum": "1.1.4",
            "sandbox_required": True,
            "import_policy": isolation["policy"],
        },
    )
    App.closeDocument(doc.Name)


try:
    main()
except Exception as exc:
    atomic_json("/work/import-error.json", {"error": str(exc)})
    raise
