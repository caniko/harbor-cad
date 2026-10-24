"""Exercise the fixed importer script with only the FreeCAD boundary substituted."""

import hashlib
import json
import runpy
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest


def inspect(monkeypatch, tmp_path, regions, objects):
    plan = tmp_path / "plan.json"
    plan.write_text(
        json.dumps(
            {
                "case": {
                    "regions": regions,
                    "geometry": {"synthetic": False},
                    "geometry_tolerance": {"value": 1e-5, "unit": "m"},
                }
            }
        )
    )
    document = SimpleNamespace(Name="Imported", Objects=objects, recompute=lambda: None)
    app = SimpleNamespace(
        Version=lambda: ["1", "1", "4"],
        ParamGet=lambda _: SimpleNamespace(SetBool=lambda *_: None),
        openDocument=lambda _: document,
        closeDocument=lambda _: None,
    )
    mesh = SimpleNamespace(CountFacets=12, write=lambda _: None)
    monkeypatch.setitem(sys.modules, "FreeCAD", app)
    monkeypatch.setitem(
        sys.modules,
        "harbor_cad_import_policy",
        SimpleNamespace(
            verify_import_environment=lambda _: {"policy": "harbor-cad-importer-v1"}
        ),
    )
    monkeypatch.setitem(
        sys.modules, "MeshPart", SimpleNamespace(meshFromShape=lambda **_: mesh)
    )
    monkeypatch.setenv("HARBOR_CAD_OPERATION", "cad_inspect")
    monkeypatch.setenv("HARBOR_CAD_PLAN", str(plan))
    write = Path.write_text
    replace = Path.replace
    stat = Path.stat
    read_bytes = Path.read_bytes
    monkeypatch.setattr(
        Path,
        "stat",
        lambda path, **kwargs: stat(
            tmp_path / path.name if str(path).startswith("/work/") else path, **kwargs
        ),
    )
    monkeypatch.setattr(
        Path,
        "read_bytes",
        lambda path: read_bytes(
            tmp_path / path.name if str(path).startswith("/work/") else path
        ),
    )
    monkeypatch.setattr(
        Path,
        "write_text",
        lambda path, text: write(tmp_path / path.name, text),
    )
    monkeypatch.setattr(
        Path,
        "replace",
        lambda path, destination: replace(
            tmp_path / path.name, tmp_path / Path(destination).name
        ),
    )
    runpy.run_path(str(Path(__file__).parents[2] / "adapters/freecad_bridge.py"))


def solid(name):
    return SimpleNamespace(
        Name=name,
        Label=name,
        TypeId="Part::Feature",
        isDerivedFrom=lambda name: name == "Part::Feature",
        getParentGeoFeatureGroup=lambda: None,
        Shape=SimpleNamespace(
            isNull=lambda: False,
            isValid=lambda: True,
            Solids=[object()],
            exportBrep=lambda path: Path(path).write_text(
                "controlled closed native BREP test boundary"
            ),
            Volume=1000,
            BoundBox=SimpleNamespace(XMin=0, XMax=10, YMin=0, YMax=10, ZMin=0, ZMax=10),
        ),
        Placement=SimpleNamespace(toMatrix=lambda: SimpleNamespace(A=[1] * 16)),
    )


@pytest.mark.parametrize(
    "regions,objects", [(["wall"], []), (["fluid", "wall"], [solid("fluid")])]
)
def test_inspection_rejects_every_missing_requested_region(
    monkeypatch, tmp_path, regions, objects
):
    with pytest.raises(ValueError, match="missing or ambiguous named regions"):
        inspect(monkeypatch, tmp_path, regions, objects)
    assert not (tmp_path / "cad_inspect-receipt.json").exists()


def test_inspection_keeps_an_explicit_wall_solid(monkeypatch, tmp_path):
    inspect(monkeypatch, tmp_path, ["wall"], [solid("wall")])
    report = json.loads((tmp_path / "regions.json").read_text())
    assert [region["name"] for region in report["regions"]] == ["wall"]
    assert report["regions"][0]["volume_m3"] == pytest.approx(1e-6, rel=1e-14, abs=0)
    assert json.loads((tmp_path / "cad_inspect-receipt.json").read_text())["executed"]
    brep = json.loads((tmp_path / "brep-manifest.json").read_text())["regions"][0]
    assert brep["path"] == "wall.brep" and brep["region_name"] == "wall"
    assert (
        brep["sha256"]
        == hashlib.sha256((tmp_path / "wall.brep").read_bytes()).hexdigest()
    )
    assert brep["bytes"] == (tmp_path / "wall.brep").stat().st_size
    assert brep["bounds_m"] == report["regions"][0]["bounds_m"]
    assert (
        brep["source_unit"] == "mm"
        and brep["placement_translation_unit"] == "mm"
        and brep["scale_to_m"] == 0.001
    )


@pytest.mark.parametrize("unsupported", ["parent-assembly", "link"])
def test_local_assembly_or_link_coordinates_cannot_be_reported_as_world_geometry(
    monkeypatch, tmp_path, unsupported
):
    shape = solid("solid")
    if unsupported == "parent-assembly":
        shape.getParentGeoFeatureGroup = lambda: SimpleNamespace(
            Name="TranslatedAssembly"
        )
    else:
        shape.isDerivedFrom = lambda _: False
    with pytest.raises(ValueError, match="assembly/link world transforms"):
        inspect(monkeypatch, tmp_path, ["solid"], [shape])
    assert not (tmp_path / "solid.brep").exists()
    assert not (tmp_path / "regions.json").exists()
    assert not (tmp_path / "cad_inspect-receipt.json").exists()
