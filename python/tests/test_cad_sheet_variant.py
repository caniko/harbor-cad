"""Version-2 spreadsheet allowlist/refusals; mocks establish no native CAD execution."""

import importlib.util
from pathlib import Path
from types import SimpleNamespace

import pytest


def module():
    path = Path(__file__).parents[2] / "adapters/cad_variant.py"
    source = importlib.util.spec_from_file_location("cad_variant", path)
    result = importlib.util.module_from_spec(source)
    source.loader.exec_module(result)
    result.min_wall = lambda shape, part: 2.0  # real probe needs OpenCASCADE
    return result


class Unit:
    def __init__(self, length=0):
        self.length = length

    def __eq__(self, other):
        return isinstance(other, Unit) and other.length == self.length


class Quantity:
    """Only `<number> mm` text, like the fixed adapter emits."""

    def __init__(self, text):
        number, _, unit = text.lstrip("=").partition(" ")
        self.Value = float(number)
        self.Unit = Unit(1 if unit == "mm" else 0)


class Sheet:
    TypeId = "Spreadsheet::Sheet"

    def __init__(self, name="Params"):
        self.Name = self.Label = name
        self.State = ["Up-to-date"]
        self.cells = {"A1": ["length", None], "B1": ["=20 mm", "length"]}
        self.cells["B2"] = ["8", "fin_count"]

    def isValid(self):
        return True

    def getUsedCells(self):
        return list(self.cells)

    def getContents(self, cell):
        return self.cells[cell][0]

    def getAlias(self, cell):
        return self.cells[cell][1]

    def getCellFromAlias(self, alias):
        return next((c for c, (_, a) in self.cells.items() if a == alias), None)

    def get(self, alias):
        text = self.cells[self.getCellFromAlias(alias)][0]
        return Quantity(text) if text.endswith("mm") else int(float(text))

    def set(self, cell, text):
        self.cells[cell][0] = ("=" + text) if text.endswith("mm") else text


class Box:
    TypeId = "Part::Box"
    Name = Label = "solid"
    PropertiesList = ("Length", "Refine")

    def __init__(self, sheet):
        self.sheet, self.State, self.Refine = sheet, ["Up-to-date"], False
        self.ExpressionEngine = [("Length", "Params.length")]
        self.Placement = SimpleNamespace(
            toMatrix=lambda: SimpleNamespace(A=[1.0, 0, 0, 0, 0, 1, 0, 0] + [0] * 8)
        )
        self.recompute()

    def recompute(self):
        self.Length = self.sheet.get("length").Value
        self.Shape = SimpleNamespace(
            isNull=lambda: False,
            isValid=lambda: True,
            Solids=[1],
            Volume=self.Length * 100.0,
            Area=2 * (self.Length * 20.0 + 100.0),
            BoundBox=SimpleNamespace(
                XMin=0.0, XMax=self.Length, YMin=0.0, YMax=10.0, ZMin=0.0, ZMax=10.0
            ),
        )

    def isValid(self):
        return "Invalid" not in self.State

    def isDerivedFrom(self, kind):
        return kind == "Part::Feature"

    def getParentGeoFeatureGroup(self):
        return None

    def getTypeIdOfProperty(self, name):
        return "App::PropertyLength"

    def getPropertyStatus(self, name):
        return []


class Document:
    def __init__(self):
        self.sheet = Sheet()
        self.box = Box(self.sheet)
        self.Objects = [self.sheet, self.box]
        self.recomputes = 0

    def getObject(self, name):
        return next((o for o in self.Objects if o.Name == name), None)

    def recompute(self):
        self.recomputes += 1
        self.box.recompute()


def specification(native, doc, **parameters):
    region = {"name": "solid", **native.region_geometry(doc.box)}
    return {
        "request": {
            "sheet_name": "Params",
            "parameters": parameters,
            "regions": ["solid"],
        },
        "source": {"regions": [region]},
    }


def test_only_existing_alias_cells_change_and_regions_report_metrics():
    native, doc = module(), Document()
    spec = specification(
        native,
        doc,
        length={"value": 0.03, "unit": "m"},
        fin_count={"value": 6.0, "unit": "1"},
    )
    report = native.apply_sheet(doc, spec, 1e-6, None)
    assert doc.recomputes == 1
    assert doc.sheet.getContents("A1") == "length"
    assert report["parameters"] == {
        "fin_count": {"cell": "B2", "before": "8", "after": "6.0"},
        "length": {"cell": "B1", "before": "=20 mm", "after": "=30.0 mm"},
    }
    after = report["after"]["solid"]
    assert after["bounds_m"][1] == pytest.approx(0.03)
    assert after["volume_m3"] == pytest.approx(3e-6)
    assert after["surface_area_m2"] > 0 and after["min_wall_m"] == 0.002
    assert report["before"]["solid"]["bounds_m"][1] == pytest.approx(0.02)
    assert report["only_sheet_cells_changed"] and report["expressions"]


@pytest.mark.parametrize(
    "drift",
    ["alias", "dimension", "count-unit", "unit", "range", "bool", "second-sheet"]
    + ["sheet-name", "original", "invalid", "input-drift"],
)
def test_unknown_aliases_units_sheets_and_document_drift_refuse(drift):
    native, doc = module(), Document()
    parameters = {"length": {"value": 25.0, "unit": "mm"}}
    spec = specification(native, doc, **parameters)
    request = spec["request"]
    if drift == "alias":
        request["parameters"] = {"width": {"value": 25.0, "unit": "mm"}}
    elif drift == "dimension":
        request["parameters"] = {"length": {"value": 25.0, "unit": "1"}}
    elif drift == "count-unit":
        request["parameters"] = {"fin_count": {"value": 2.0, "unit": "mm"}}
    elif drift == "unit":
        request["parameters"]["length"]["unit"] = "kg"
    elif drift == "range":
        request["parameters"]["length"] = {"value": 20.0, "unit": "m"}
    elif drift == "bool":
        request["parameters"]["length"]["value"] = True
    elif drift == "second-sheet":
        doc.Objects.append(Sheet("Other"))
    elif drift == "sheet-name":
        request["sheet_name"] = "Other"
    elif drift == "original":
        spec["source"]["regions"][0]["volume_m3"] *= 2
    elif drift == "invalid":
        doc.recompute = lambda: doc.box.State.append("Invalid")
    else:
        doc.recompute = lambda: setattr(doc.box, "Refine", True)
    with pytest.raises(ValueError):
        native.apply_sheet(doc, spec, 1e-6, None)


def test_sheet_text_is_composed_from_validated_numbers_only():
    native = module()
    assert native.content({"value": 12.5, "unit": "mm"}) == (
        "length",
        "12.5 mm",
        0.0125,
    )
    assert native.content({"value": 3, "unit": "1"})[:2] == ("dimensionless", "3.0")
    for bad in [
        {"value": float("nan"), "unit": "mm"},
        {"value": 1e-9, "unit": "mm"},
        {"value": -1, "unit": "1"},
        {"value": 1, "unit": "Params.wall"},
    ]:
        with pytest.raises(ValueError):
            native.content(bad)
